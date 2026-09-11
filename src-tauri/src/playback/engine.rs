//! The playback engine: wires source, decode, DSP and output into one session.
//!
//! Threading follows the plan's ownership rules:
//!
//! - Network I/O and HLS timers run as a Tokio task, because they are genuinely async.
//! - Demux, decode, EQ and resampling run on a **dedicated OS thread**, never on a Tokio
//!   executor worker. These are synchronous, stateful and CPU-bound; running them on the
//!   async runtime would stall unrelated tasks.
//! - The CPAL callback runs on the audio thread and only pops from a lock-free ring.
//!
//! Cancellation reaches every stage: the token stops network waits, and the decode thread
//! polls for it (and for output health) rather than depending solely on a blocked channel
//! operation to wake it up.
//!
//! # The connect-to-play watchdog
//!
//! A successful HTTP response, TS/HLS detection, or even a constructed decoder are not
//! evidence that this attempt will ever produce audible sound: a TS stream with no
//! supported audio track, an HLS playlist that never advances, or a very slow trickle can
//! all keep the transport "healthy" forever while the output callback never consumes a real
//! frame. [`spawn_connect_watchdog`] enforces an attempt-level, monotonic deadline
//! (`CONNECT_TIMEOUT_MS`, shared with `playback-core`'s documented default) independently of
//! whatever the source is doing, and reports a transient failure if it fires. It races
//! against the decode thread's own "first confirmed frame" transition through
//! `connect_decided`, a single-attempt latch: whichever of {the watchdog, the decode thread's
//! first `Playing`} resolves first is the only one that reports anything for this attempt,
//! so a callback-confirmation-vs-timeout race can never produce two contradictory outcomes
//! for the same attempt (see the module tests).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use apogee_playback_core::analysis::{SpectrumAnalyzer, BAND_COUNT};
use apogee_playback_core::dsp::{ControlUpdate, EqCoefficients};
use apogee_playback_core::output::{
    control_channel, pcm_ring, BufferState, BufferStateFlag, ControlProducer, ConversionStage,
    GatedConsumer, OutputFormat, RingConsumer, RingProducer,
};
use apogee_playback_core::pipeline::Pipeline;
use apogee_playback_core::session::{BufferingReason, ErrorClass, Generation, CONNECT_TIMEOUT_MS};
use tokio_util::sync::CancellationToken;

use super::audio_out::{AudioOutput, OutputStats};
use super::device::DeviceRequest;
use super::source::{self, SourceError};
use crate::network::NetworkService;

/// Bounded queue between the network task and the decode thread.
const EVENT_QUEUE_DEPTH: usize = 256;
/// How often the decode thread polls for new input, cancellation, and output health while it
/// would otherwise be blocked. Small enough that stop/failure detection stays prompt; large
/// enough not to burn a core spinning.
const DECODE_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How often the ring-full wait re-checks output health/cancellation before retrying the
/// push. Deliberately shorter than `DECODE_POLL_INTERVAL`: a full ring at steady state drains
/// quickly, so short waits here keep normal playback latency low.
const RING_FULL_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// How often [`spawn_connect_watchdog`] wakes to check whether the deadline has passed or the
/// attempt has been cancelled/confirmed.
const CONNECT_WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Decoded audio buffering, fixed for the lifetime of an output session.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferSettings {
    pub capacity_ms: u64,
    pub start_ms: u64,
    pub rebuffer_ms: u64,
}

impl Default for BufferSettings {
    fn default() -> Self {
        Self {
            capacity_ms: 2000,
            start_ms: 500,
            rebuffer_ms: 150,
        }
    }
}

impl BufferSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(100..=10_000).contains(&self.capacity_ms) {
            return Err("Buffer capacity must be between 100 and 10,000 ms".into());
        }
        if self.start_ms < 50 || self.start_ms > self.capacity_ms {
            return Err(
                "Startup buffer must be at least 50 ms and no greater than capacity".into(),
            );
        }
        if self.rebuffer_ms >= self.start_ms {
            return Err("Rebuffer threshold must be less than the startup buffer".into());
        }
        Ok(())
    }
}

/// Audio preferences. Buffering is fixed at session start; other settings can change live.
#[derive(Debug, Clone)]
pub struct AudioSettings {
    pub buffering: BufferSettings,
    pub volume: u8,
    pub muted: bool,
    pub equalizer_enabled: bool,
    pub equalizer_gains: [f64; 10],
    /// When false no FFT work is done at all, so a hidden visualiser costs nothing.
    pub visualizer_enabled: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            buffering: BufferSettings::default(),
            volume: 100,
            muted: false,
            equalizer_enabled: false,
            equalizer_gains: [0.0; 10],
            visualizer_enabled: false,
        }
    }
}

/// Domain events, translated to Tauri events at the boundary so the engine stays
/// independent of `AppHandle`.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    Buffering {
        generation: Generation,
        reason: BufferingReason,
    },
    Playing {
        generation: Generation,
    },
    Format {
        generation: Generation,
        sample_rate: u32,
        channels: u16,
    },
    Bitrate {
        generation: Generation,
        kbps: Option<u32>,
    },
    Underrun {
        generation: Generation,
        frames: u64,
    },
    /// Smoothed spectrum levels, 0..1 per band, for the visualiser.
    Spectrum {
        levels: [f32; BAND_COUNT],
    },
    Failed {
        generation: Generation,
        class: ErrorClass,
        message: String,
    },
}

/// Where engine events go. Injected so tests can observe without Tauri.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: EngineEvent);
}

/// A running session. Dropping it cancels and tears down every stage.
pub struct Session {
    /// Kept for diagnostics and for asserting identity in tests; the controller is the
    /// authority on which generation is current.
    #[allow(dead_code)]
    generation: Generation,
    cancel: CancellationToken,
    decode_thread: Option<std::thread::JoinHandle<()>>,
    // Dropped promptly on teardown, though the decode thread no longer depends on that to
    // wake up - it polls `cancel`/output health on a short interval regardless (see
    // `DECODE_POLL_INTERVAL`), which is what actually bounds teardown time now.
    events_tx: Option<tokio::sync::mpsc::Sender<DecodeInput>>,
    output: Option<AudioOutput>,
    settings_tx: std::sync::mpsc::Sender<AudioSettings>,
    visualizer_enabled: Arc<AtomicBool>,
    analysis_thread: Option<std::thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}

impl Session {
    #[allow(dead_code)]
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Apply new volume/mute/EQ without interrupting playback.
    pub fn update_settings(&self, settings: AudioSettings) {
        self.visualizer_enabled
            .store(settings.visualizer_enabled, Ordering::Relaxed);
        let _ = self.settings_tx.send(settings);
    }

    /// The device actually in use, which may differ from the one requested.
    #[must_use]
    pub fn device_name(&self) -> Option<String> {
        self.output.as_ref().map(|o| o.descriptor().name.clone())
    }

    #[must_use]
    pub fn device_id(&self) -> Option<String> {
        self.output.as_ref().map(|o| o.descriptor().id.clone())
    }

    /// Exposed for diagnostics: the device format the decode thread is producing.
    #[allow(dead_code)]
    #[must_use]
    pub fn output_format(&self) -> Option<OutputFormat> {
        self.output.as_ref().map(AudioOutput::format)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.cancel.cancel();
        // The decode thread no longer depends on the sender dropping (or on any channel
        // event at all) to notice a stop: it polls `stopped`/output health on a short
        // interval (see `DECODE_POLL_INTERVAL`) both while waiting for input and while
        // waiting for PCM ring capacity, so it cannot be left blocked indefinitely by a full
        // ring or a starved queue the way a plain blocking `recv` could be. Dropping the
        // sender here just shaves a little latency off that.
        self.events_tx = None;
        self.output = None;
        if let Some(thread) = self.analysis_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.decode_thread.take() {
            let _ = thread.join();
        }
    }
}

enum DecodeInput {
    Event(apogee_playback_core::pipeline::SourceEvent),
    Finished(Result<(), SourceError>),
}

/// Start a session playing `url`.
///
/// Returns once the output device is open and the pipeline is running; playback is
/// confirmed later, through an `EngineEvent::Playing` raised when the audio callback has
/// actually consumed decoded frames.
pub fn start_session(
    generation: Generation,
    url: String,
    device: &DeviceRequest,
    settings: AudioSettings,
    network: NetworkService,
    sink: Arc<dyn EventSink>,
) -> Result<Session, String> {
    settings.buffering.validate()?;
    let cancel = CancellationToken::new();
    let stopped = Arc::new(AtomicBool::new(false));
    // A single-attempt latch: whichever of {this watchdog, the decode thread's first
    // confirmed frame} resolves first is the only one that reports an outcome for this
    // attempt. See the module-level doc comment.
    let connect_decided = Arc::new(AtomicBool::new(false));

    // Resolve the actual device and its format once. A system-default change between two
    // independent resolutions could otherwise pair a ring for one device with another.
    let prepared = super::audio_out::prepare(device).map_err(|e| e.to_string())?;
    let format = prepared.format();

    let (producer, consumer) = pcm_ring(
        format,
        format.frames_for_millis(settings.buffering.capacity_ms),
    );
    let buffer_state = Arc::new(BufferStateFlag::default());
    let consumer = GatedConsumer::new(
        consumer,
        format.frames_for_millis(settings.buffering.start_ms),
        format.frames_for_millis(settings.buffering.rebuffer_ms),
        Arc::clone(&buffer_state),
    );
    let (mut control_tx, control_rx) = control_channel(8);
    control_tx
        .try_send(build_control_update(format, &settings)?)
        .map_err(|_| "initial audio control queue was unexpectedly full".to_string())?;
    let (analysis_tx, analysis_rx) = pcm_ring(format, format.frames_for_millis(250));
    let visualizer_enabled = Arc::new(AtomicBool::new(settings.visualizer_enabled));
    let output = AudioOutput::start(
        prepared,
        consumer,
        control_rx,
        analysis_tx,
        Arc::clone(&visualizer_enabled),
    )
    .map_err(|e| e.to_string())?;

    // An async channel, not `std::sync::mpsc::sync_channel`: the producer (`pump_source`,
    // below) runs on a Tokio task, and blocking that task's OS thread on a full channel -
    // as a `SyncSender::send` call would - can starve unrelated work on a constrained
    // runtime. `.send(..).await` yields instead of blocking, and race it against `cancel`
    // so a full queue never delays a stop/station-switch.
    let (events_tx, events_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_DEPTH);
    let (settings_tx, settings_rx) = std::sync::mpsc::channel();

    let decode_sink = Arc::clone(&sink);
    let decode_stats = Arc::clone(output.stats());
    let decode_stopped = Arc::clone(&stopped);
    let decode_connect_decided = Arc::clone(&connect_decided);
    let decode_buffer_state = Arc::clone(&buffer_state);
    let decode_thread = std::thread::Builder::new()
        .name("apogee-decode".to_string())
        .spawn(move || {
            decode_loop(
                generation,
                format,
                settings,
                producer,
                events_rx,
                &settings_rx,
                &decode_sink,
                &decode_stats,
                &decode_stopped,
                &decode_connect_decided,
                &decode_buffer_state,
                control_tx,
            );
        })
        .map_err(|e| format!("could not start decode thread: {e}"))?;

    spawn_connect_watchdog(
        generation,
        Duration::from_millis(CONNECT_TIMEOUT_MS),
        cancel.clone(),
        Arc::clone(output.stats()),
        Arc::clone(&sink),
        Arc::clone(&connect_decided),
    );

    // Network side: async, cancellable, feeds the bounded queue.
    let net_tx = events_tx.clone();
    let net_cancel = cancel.clone();
    let net_sink = Arc::clone(&sink);
    tokio::spawn(async move {
        let outcome = pump_source(&url, &network, &net_cancel, &net_tx).await;
        if let Err(ref e) = outcome {
            log::warn!("playback source ended: {e}");
        }
        let _ = net_tx.send(DecodeInput::Finished(outcome)).await;
        drop(net_sink);
    });

    let analysis_stopped = Arc::clone(&stopped);
    let analysis_enabled = Arc::clone(&visualizer_enabled);
    let analysis_sink = Arc::clone(&sink);
    let analysis_thread = match std::thread::Builder::new()
        .name("apogee-spectrum".to_string())
        .spawn(move || {
            analysis_loop(
                format,
                analysis_rx,
                &analysis_enabled,
                &analysis_stopped,
                &analysis_sink,
            );
        }) {
        Ok(thread) => thread,
        Err(error) => {
            // At this point the output, decode thread, watchdog and source task are live.
            // Unwind them explicitly rather than returning a partially detached session.
            stopped.store(true, Ordering::Relaxed);
            cancel.cancel();
            drop(output);
            drop(events_tx);
            let _ = decode_thread.join();
            return Err(format!("could not start spectrum thread: {error}"));
        }
    };

    Ok(Session {
        generation,
        cancel,
        decode_thread: Some(decode_thread),
        events_tx: Some(events_tx),
        output: Some(output),
        settings_tx,
        visualizer_enabled,
        analysis_thread: Some(analysis_thread),
        stopped,
    })
}

/// Watches for `timeout` of monotonic elapsed time, from the moment this attempt started,
/// without the output callback ever consuming a real frame. Enforced independently of the
/// source: a TS stream carrying no supported audio, a playlist that never advances, and a
/// very slow trickle all leave `stats.started` false forever, and none of those are visible
/// to `ContinuousTsStream`'s own stall detection (which only ever sees *some* bytes moving).
/// Stops polling as soon as the attempt is confirmed audible or cancelled - it does not
/// itself need to cancel anything on the happy path.
///
/// `timeout` is a parameter (rather than hard-coding `CONNECT_TIMEOUT_MS` here) purely so
/// tests can exercise this against a short, deterministic budget instead of the real 20s
/// production default; `start_session` always passes `CONNECT_TIMEOUT_MS`.
fn spawn_connect_watchdog(
    generation: Generation,
    timeout: Duration,
    cancel: CancellationToken,
    stats: Arc<OutputStats>,
    sink: Arc<dyn EventSink>,
    connect_decided: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if cancel.is_cancelled() {
                return;
            }
            let (_played, _underrun, started, _failed) = stats.snapshot();
            if started {
                return;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                // If this returns `false`, the decode thread already won the race (it just
                // confirmed audible playback) - see the module doc comment. Reporting a
                // timeout on top of that would be a second, contradictory outcome for the
                // same attempt.
                if !connect_decided.swap(true, Ordering::SeqCst) {
                    sink.emit(EngineEvent::Failed {
                        generation,
                        class: ErrorClass::Transient,
                        message: format!(
                            "no audio reached the output device within {:.1}s of starting this attempt",
                            timeout.as_secs_f64()
                        ),
                    });
                }
                return;
            }
            let remaining = deadline.saturating_duration_since(now);
            let sleep_for = remaining.min(CONNECT_WATCHDOG_POLL_INTERVAL);
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(sleep_for) => {}
            }
        }
    });
}

/// Drives the source, forwarding demuxed events to the decode thread.
async fn pump_source(
    url: &str,
    network: &NetworkService,
    cancel: &CancellationToken,
    tx: &tokio::sync::mpsc::Sender<DecodeInput>,
) -> Result<(), SourceError> {
    let mut src = source::open(url, network, cancel).await?;
    loop {
        if cancel.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        match src.next_event().await? {
            Some(event) => {
                // A full queue means the decoder is behind; waiting here is correct
                // backpressure, but it must stay cancellable - a blocking send would not
                // notice a stop/station-switch while the decode thread is itself stuck
                // (e.g. an output failure it hasn't yet reported; see `decode_loop`).
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(SourceError::Cancelled),
                    result = tx.send(DecodeInput::Event(event)) => {
                        if result.is_err() {
                            return Ok(()); // The decode thread is gone, i.e. stop.
                        }
                    }
                }
            }
            None => return Ok(()),
        }
    }
}

/// Reports a startup failure unless a concurrent connect-to-play watchdog already reported a
/// timeout for this same attempt (see the module doc comment on `connect_decided`). Once
/// `announced_playing` is true, the race this latch guards against is over - a device that
/// fails five minutes into a healthy attempt is never suppressed.
fn maybe_emit_failed(
    sink: &Arc<dyn EventSink>,
    connect_decided: &AtomicBool,
    generation: Generation,
    announced_playing: bool,
    class: ErrorClass,
    message: String,
) {
    if announced_playing || !connect_decided.swap(true, Ordering::SeqCst) {
        sink.emit(EngineEvent::Failed {
            generation,
            class,
            message,
        });
    }
}

/// Reports the attempt's first confirmed audible frame, unless the connect-to-play watchdog
/// already decided this attempt timed out (an exceedingly narrow race - see the module doc
/// comment). Either way the caller should treat playback as started from here on: the audio
/// genuinely is flowing even on the rare tie the watchdog wins.
fn maybe_emit_playing(
    sink: &Arc<dyn EventSink>,
    connect_decided: &AtomicBool,
    generation: Generation,
) {
    if !connect_decided.swap(true, Ordering::SeqCst) {
        sink.emit(EngineEvent::Playing { generation });
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_loop(
    generation: Generation,
    format: OutputFormat,
    settings: AudioSettings,
    mut producer: RingProducer,
    mut events: tokio::sync::mpsc::Receiver<DecodeInput>,
    settings_rx: &std::sync::mpsc::Receiver<AudioSettings>,
    sink: &Arc<dyn EventSink>,
    stats: &Arc<super::audio_out::OutputStats>,
    stopped: &Arc<AtomicBool>,
    connect_decided: &Arc<AtomicBool>,
    buffer_state: &Arc<BufferStateFlag>,
    mut control_tx: ControlProducer<ControlUpdate>,
) {
    let mut pipeline = Pipeline::new();
    let mut conversion = ConversionStage::new(format);
    let _ = settings;
    let mut has_played = false;
    let mut observed_buffer_state = BufferState::Buffering;
    let mut pending_control: Option<ControlUpdate> = None;
    let mut last_underrun = 0u64;
    let mut last_bitrate: Option<u32> = None;

    sink.emit(EngineEvent::Buffering {
        generation,
        reason: BufferingReason::Connecting,
    });

    loop {
        if stopped.load(Ordering::Relaxed) {
            return;
        }

        observe_buffer_state(
            sink,
            connect_decided,
            generation,
            buffer_state,
            &mut observed_buffer_state,
            &mut has_played,
        );

        // Prepare expensive EQ coefficients here and send only the ready-to-copy value to
        // the callback. Check this even when the network is idle so mute/volume stay prompt.
        while let Ok(next) = settings_rx.try_recv() {
            match build_control_update(format, &next) {
                Ok(update) => pending_control = Some(update),
                Err(e) => log::warn!("ignoring invalid equalizer settings: {e}"),
            }
        }
        if let Some(update) = pending_control.take() {
            pending_control = control_tx.try_send(update).err();
        }

        // Waiting for the next event from the network task must not be an unbounded
        // blocking `recv`: that would be the only thing capable of noticing a stop or an
        // output failure while the source is legitimately idle (or stuck) between events.
        // Poll instead, at `DECODE_POLL_INTERVAL`, checking exactly those two things on
        // every empty poll.
        let input = match events.try_recv() {
            Ok(input) => input,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                let (_played, _underrun, _started, failed) = stats.snapshot();
                if failed {
                    maybe_emit_failed(
                        sink,
                        connect_decided,
                        generation,
                        has_played,
                        ErrorClass::Transient,
                        "the audio output device failed".to_string(),
                    );
                    return;
                }
                std::thread::sleep(DECODE_POLL_INTERVAL);
                continue;
            }
        };

        let event = match input {
            DecodeInput::Event(event) => event,
            DecodeInput::Finished(Ok(())) => {
                maybe_emit_failed(
                    sink,
                    connect_decided,
                    generation,
                    has_played,
                    ErrorClass::Transient,
                    "the stream ended".to_string(),
                );
                return;
            }
            DecodeInput::Finished(Err(error)) => {
                let class = classify(&error);
                maybe_emit_failed(
                    sink,
                    connect_decided,
                    generation,
                    has_played,
                    class,
                    error.to_string(),
                );
                return;
            }
        };

        let block = match pipeline.accept(event) {
            Ok(Some(block)) => block,
            Ok(None) => continue,
            Err(error) => {
                maybe_emit_failed(
                    sink,
                    connect_decided,
                    generation,
                    has_played,
                    ErrorClass::Permanent,
                    error.to_string(),
                );
                return;
            }
        };

        let source_format = (block.rate, block.channels as u16);
        let format_changed = conversion.source_format() != Some(source_format);
        let (ready, changed) =
            match conversion.process(block.rate, block.channels as u16, &block.samples) {
                Ok(result) => result,
                Err(e) => {
                    maybe_emit_failed(
                        sink,
                        connect_decided,
                        generation,
                        has_played,
                        ErrorClass::Permanent,
                        format!("unsupported audio format: {e}"),
                    );
                    return;
                }
            };
        if changed || format_changed {
            sink.emit(EngineEvent::Format {
                generation,
                sample_rate: block.rate,
                channels: block.channels as u16,
            });
        }

        let mut offset = 0usize;
        while offset < ready.len() {
            if stopped.load(Ordering::Relaxed) {
                return;
            }
            // A full ring must not be the only thing keeping this loop busy: without this
            // check, a producer that stalls here (because the device has failed and nothing
            // is draining the ring any more) would spin forever without ever reaching the
            // failure check below, which only runs after a whole block has been queued.
            let (_played, _underrun, _started, failed) = stats.snapshot();
            if failed {
                maybe_emit_failed(
                    sink,
                    connect_decided,
                    generation,
                    has_played,
                    ErrorClass::Transient,
                    "the audio output device failed".to_string(),
                );
                return;
            }
            let written = producer.push_frames(&ready[offset..]);
            if written == 0 {
                // Ring full: the device has plenty queued. Wait a little rather than spin.
                std::thread::sleep(RING_FULL_POLL_INTERVAL);
                continue;
            }
            offset += written * format.channels as usize;
        }

        observe_buffer_state(
            sink,
            connect_decided,
            generation,
            buffer_state,
            &mut observed_buffer_state,
            &mut has_played,
        );

        let (_played, underrun, _started, failed) = stats.snapshot();
        if failed {
            maybe_emit_failed(
                sink,
                connect_decided,
                generation,
                has_played,
                ErrorClass::Transient,
                "the audio output device failed".to_string(),
            );
            return;
        }
        if underrun > last_underrun {
            // Reported from here, outside the callback, as the plan requires.
            sink.emit(EngineEvent::Underrun {
                generation,
                frames: underrun - last_underrun,
            });
            last_underrun = underrun;
        }

        let bitrate = pipeline.bitrate_kbps(block.rate);
        if bitrate != last_bitrate {
            last_bitrate = bitrate;
            sink.emit(EngineEvent::Bitrate {
                generation,
                kbps: bitrate,
            });
        }
    }
}

fn build_control_update(
    format: OutputFormat,
    settings: &AudioSettings,
) -> Result<ControlUpdate, String> {
    Ok(ControlUpdate {
        eq: EqCoefficients::build(
            f64::from(format.sample_rate),
            settings.equalizer_enabled,
            &settings.equalizer_gains,
        )?,
        volume: settings.volume,
        muted: settings.muted,
    })
}

fn observe_buffer_state(
    sink: &Arc<dyn EventSink>,
    connect_decided: &AtomicBool,
    generation: Generation,
    state: &BufferStateFlag,
    observed: &mut BufferState,
    has_played: &mut bool,
) {
    let current = state.get();
    if current == *observed {
        return;
    }
    *observed = current;
    match current {
        BufferState::Playing => {
            if *has_played {
                sink.emit(EngineEvent::Playing { generation });
            } else {
                *has_played = true;
                maybe_emit_playing(sink, connect_decided, generation);
            }
        }
        BufferState::Buffering if *has_played => sink.emit(EngineEvent::Buffering {
            generation,
            reason: BufferingReason::Underrun,
        }),
        BufferState::Buffering => {}
    }
}

/// Pulls the callback's bounded post-control tap and performs FFT work away from the
/// real-time thread. Disabling the visualizer drains any stale tap data without analysis.
fn analysis_loop(
    format: OutputFormat,
    mut consumer: RingConsumer,
    enabled: &AtomicBool,
    stopped: &AtomicBool,
    sink: &Arc<dyn EventSink>,
) {
    let channels = format.channels.max(1) as usize;
    let mut scratch = vec![0.0f32; 1024 * channels];
    let mut analyzer: Option<SpectrumAnalyzer> = None;
    while !stopped.load(Ordering::Relaxed) {
        let frames = consumer.pop_into(&mut scratch);
        if frames == 0 {
            std::thread::sleep(DECODE_POLL_INTERVAL);
            continue;
        }
        if !enabled.load(Ordering::Relaxed) {
            analyzer = None;
            continue;
        }
        let analyzer =
            analyzer.get_or_insert_with(|| SpectrumAnalyzer::new(format.sample_rate, channels));
        if let Some(levels) = analyzer.push(&scratch[..frames * channels]) {
            sink.emit(EngineEvent::Spectrum { levels });
        }
    }
}

/// Whether an error is worth retrying. Authentication and unsupported content are not.
/// Whether retrying could possibly help.
///
/// A 4xx is the server saying the request itself is wrong - bad credentials, an unknown
/// channel - and repeating it unchanged cannot succeed. A 5xx is the server saying it could
/// not serve the request *now*, which is exactly the upstream spin-up case the plan
/// documents, so those stay retryable.
fn classify(error: &SourceError) -> ErrorClass {
    match error {
        SourceError::Unsupported(_) => ErrorClass::Permanent,
        SourceError::Status { status, .. } if (400..500).contains(status) => ErrorClass::Permanent,
        _ => ErrorClass::Transient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<EngineEvent>>,
    }

    impl EventSink for Recorder {
        fn emit(&self, event: EngineEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl Recorder {
        fn snapshot(&self) -> Vec<EngineEvent> {
            self.events.lock().unwrap().clone()
        }
        fn has_playing(&self) -> bool {
            self.snapshot()
                .iter()
                .any(|e| matches!(e, EngineEvent::Playing { .. }))
        }
        fn failure(&self) -> Option<String> {
            self.snapshot().iter().find_map(|e| match e {
                EngineEvent::Failed { message, .. } => Some(message.clone()),
                _ => None,
            })
        }
    }

    fn fixture_bytes() -> Vec<u8> {
        let mut all = Vec::new();
        for n in [
            "aac-0.ts", "aac-1.ts", "aac-2.ts", "aac-3.ts", "aac-4.ts", "aac-5.ts",
        ] {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("playback-core/tests/fixtures")
                .join(n);
            all.extend(std::fs::read(p).unwrap());
        }
        all
    }

    /// Serves the fixture as one continuous chunked MPEG-TS body, repeating so the
    /// stream behaves like live radio rather than ending immediately.
    fn spawn_ts_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let body = fixture_bytes();
                std::thread::spawn(move || {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\n\
                          Transfer-Encoding: chunked\r\n\r\n",
                    );
                    for _ in 0..6 {
                        for piece in body.chunks(4096) {
                            if write!(stream, "{:x}\r\n", piece.len()).is_err() {
                                return;
                            }
                            if stream.write_all(piece).is_err() {
                                return;
                            }
                            if stream.write_all(b"\r\n").is_err() {
                                return;
                            }
                        }
                    }
                    let _ = stream.write_all(b"0\r\n\r\n");
                });
            }
        });
        format!("http://{addr}/live.ts")
    }

    fn silent_device() -> Option<DeviceRequest> {
        let devices = super::super::device::list_output_devices().ok()?;
        devices
            .iter()
            .find(|d| d.id.contains("null") || d.name.to_lowercase().contains("discard"))
            .map(|d| DeviceRequest::Specific(d.id.clone()))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_real_ts_stream_decodes_all_the_way_to_the_audio_device() {
        let Some(device) = silent_device() else {
            eprintln!("skipping: no silent output device on this machine");
            return;
        };
        let url = spawn_ts_server();
        let network = NetworkService::new().unwrap();
        let sink = Arc::new(Recorder::default());

        let session = start_session(
            Generation::default(),
            url,
            &device,
            AudioSettings::default(),
            network,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        )
        .expect("session should start");

        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !sink.has_playing() {
            if let Some(failure) = sink.failure() {
                panic!("engine failed before playing: {failure}");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert!(
            sink.has_playing(),
            "never reached Playing; events were {:?}",
            sink.snapshot()
        );

        // The format event must report the stream's real rate, not the device's.
        let format_event = sink.snapshot().into_iter().find_map(|e| match e {
            EngineEvent::Format {
                sample_rate,
                channels,
                ..
            } => Some((sample_rate, channels)),
            _ => None,
        });
        assert_eq!(
            format_event,
            Some((44_100, 2)),
            "should report the decoded stream format"
        );

        // Bitrate should settle in the AAC range rather than reporting transport throughput.
        let bitrate = sink.snapshot().into_iter().rev().find_map(|e| match e {
            EngineEvent::Bitrate { kbps: Some(k), .. } => Some(k),
            _ => None,
        });
        if let Some(kbps) = bitrate {
            assert!(
                (200..=300).contains(&kbps),
                "bitrate {kbps} kbps looks like transport throughput, not audio"
            );
        }

        assert!(session.device_name().is_some());
        drop(session);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stopping_a_session_tears_down_every_stage_promptly() {
        let Some(device) = silent_device() else {
            eprintln!("skipping: no silent output device on this machine");
            return;
        };
        let url = spawn_ts_server();
        let network = NetworkService::new().unwrap();
        let sink = Arc::new(Recorder::default());

        let session = start_session(
            Generation::default(),
            url,
            &device,
            AudioSettings::default(),
            network,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        )
        .unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        let start = Instant::now();
        drop(session);
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "teardown took {elapsed:?}; a station switch must not stall"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_html_error_page_fails_the_session_instead_of_stalling() {
        let Some(device) = silent_device() else {
            eprintln!("skipping: no silent output device on this machine");
            return;
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: 46\r\n\r\n\
                      <!DOCTYPE html><html><body>Forbidden</body></html>",
                );
            }
        });

        let network = NetworkService::new().unwrap();
        let sink = Arc::new(Recorder::default());
        let started = start_session(
            Generation::default(),
            format!("http://{addr}/live.ts"),
            &device,
            AudioSettings::default(),
            network,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        );

        let Ok(session) = started else { return };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && sink.failure().is_none() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            sink.failure().is_some(),
            "an HTML error body must fail the session, not hang"
        );
        assert!(!sink.has_playing(), "nothing should ever have played");
        drop(session);
    }

    // -----------------------------------------------------------------
    // Finding 2: the connect-to-play watchdog.
    // -----------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_watchdog_fires_when_no_frame_is_ever_confirmed() {
        let sink = Arc::new(Recorder::default());
        let stats = Arc::new(OutputStats::default()); // `started` never set: no frame ever confirmed.
        let cancel = CancellationToken::new();
        let connect_decided = Arc::new(AtomicBool::new(false));

        spawn_connect_watchdog(
            Generation::default(),
            Duration::from_millis(150),
            cancel.clone(),
            Arc::clone(&stats),
            Arc::clone(&sink) as Arc<dyn EventSink>,
            Arc::clone(&connect_decided),
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && sink.failure().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let failure = sink.failure();
        assert!(
            failure.is_some(),
            "the watchdog must fire independently of any source activity"
        );
        assert!(connect_decided.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_watchdog_never_fires_once_playback_is_confirmed() {
        let sink = Arc::new(Recorder::default());
        let stats = Arc::new(OutputStats::default());
        stats.started.store(true, Ordering::Relaxed); // Confirmed before the watchdog ever checks.
        let cancel = CancellationToken::new();
        let connect_decided = Arc::new(AtomicBool::new(false));

        spawn_connect_watchdog(
            Generation::default(),
            Duration::from_millis(100),
            cancel.clone(),
            Arc::clone(&stats),
            Arc::clone(&sink) as Arc<dyn EventSink>,
            Arc::clone(&connect_decided),
        );

        // Wait well past the deadline; nothing should ever be reported.
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            sink.snapshot().is_empty(),
            "a confirmed attempt must never be timed out: {:?}",
            sink.snapshot()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_watchdog_stops_promptly_when_the_attempt_is_cancelled() {
        let sink = Arc::new(Recorder::default());
        let stats = Arc::new(OutputStats::default());
        let cancel = CancellationToken::new();
        let connect_decided = Arc::new(AtomicBool::new(false));

        spawn_connect_watchdog(
            Generation::default(),
            Duration::from_millis(100),
            cancel.clone(),
            Arc::clone(&stats),
            Arc::clone(&sink) as Arc<dyn EventSink>,
            Arc::clone(&connect_decided),
        );
        // Station switch / stop, well before the deadline.
        cancel.cancel();

        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            sink.snapshot().is_empty(),
            "a cancelled attempt must not report a timeout: {:?}",
            sink.snapshot()
        );
    }

    /// Directly exercises the `connect_decided` latch used by both the watchdog and the
    /// decode thread: whichever of {confirmed playback, a pre-play failure} is reported
    /// first must be the *only* outcome for the attempt, even when both are attempted.
    /// This is the mechanism behind "callback confirmation races with timeout: exactly one
    /// outcome wins for that attempt".
    #[test]
    fn connect_decided_latch_allows_exactly_one_outcome_playing_first() {
        let sink = Arc::new(Recorder::default());
        let event_sink = Arc::clone(&sink) as Arc<dyn EventSink>;
        let latch = AtomicBool::new(false);

        maybe_emit_playing(&event_sink, &latch, Generation::default());
        maybe_emit_failed(
            &event_sink,
            &latch,
            Generation::default(),
            false,
            ErrorClass::Transient,
            "late timeout".to_string(),
        );

        let events = sink.snapshot();
        assert_eq!(
            events.len(),
            1,
            "exactly one outcome must be reported, got {events:?}"
        );
        assert!(matches!(events[0], EngineEvent::Playing { .. }));
    }

    #[test]
    fn connect_decided_latch_allows_exactly_one_outcome_failure_first() {
        let sink = Arc::new(Recorder::default());
        let event_sink = Arc::clone(&sink) as Arc<dyn EventSink>;
        let latch = AtomicBool::new(false);

        maybe_emit_failed(
            &event_sink,
            &latch,
            Generation::default(),
            false,
            ErrorClass::Transient,
            "timed out".to_string(),
        );
        // The decode thread's own first-frame confirmation loses the race.
        maybe_emit_playing(&event_sink, &latch, Generation::default());

        let events = sink.snapshot();
        assert_eq!(
            events.len(),
            1,
            "exactly one outcome must be reported, got {events:?}"
        );
        assert!(matches!(events[0], EngineEvent::Failed { .. }));
    }

    /// A failure reported long after playback was confirmed (a device that dies five
    /// minutes in) must never be suppressed by the connect-phase latch - only the narrow
    /// startup race is guarded.
    #[test]
    fn a_post_play_failure_is_never_suppressed_by_the_connect_latch() {
        let sink = Arc::new(Recorder::default());
        let event_sink = Arc::clone(&sink) as Arc<dyn EventSink>;
        let latch = AtomicBool::new(false);

        maybe_emit_playing(&event_sink, &latch, Generation::default());
        // `announced_playing = true` here models a failure discovered well after startup.
        maybe_emit_failed(
            &event_sink,
            &latch,
            Generation::default(),
            true,
            ErrorClass::Transient,
            "the audio output device failed".to_string(),
        );

        let events = sink.snapshot();
        assert_eq!(
            events.len(),
            2,
            "both outcomes must be reported: {events:?}"
        );
        assert!(matches!(events[0], EngineEvent::Playing { .. }));
        assert!(matches!(events[1], EngineEvent::Failed { .. }));
    }

    // -----------------------------------------------------------------
    // Finding 3: output health must be visible while the decode thread is waiting for
    // decoder input or for PCM ring capacity, not only after a whole block is queued.
    // -----------------------------------------------------------------

    fn fixture_source_events() -> Vec<apogee_playback_core::pipeline::SourceEvent> {
        use apogee_playback_core::pipeline::TsIngest;
        let mut ingest = TsIngest::new();
        ingest.feed(&fixture_bytes());
        ingest.finish();
        let mut events = Vec::new();
        while let Some(event) = ingest.poll() {
            events.push(event);
        }
        events
    }

    /// Runs `decode_loop` on its own thread the same way `start_session` does, but with
    /// full control over the ring/channel/stats so the failure paths can be driven directly
    /// without a real audio device or network.
    #[allow(clippy::too_many_arguments)]
    fn spawn_decode_loop_for_test(
        format: OutputFormat,
        producer: RingProducer,
        events_rx: tokio::sync::mpsc::Receiver<DecodeInput>,
        sink: Arc<dyn EventSink>,
        stats: Arc<OutputStats>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let (_settings_tx, settings_rx) = std::sync::mpsc::channel();
            let stopped = Arc::new(AtomicBool::new(false));
            let connect_decided = Arc::new(AtomicBool::new(false));
            let buffer_state = Arc::new(BufferStateFlag::default());
            let (control_tx, _control_rx) = control_channel(2);
            decode_loop(
                Generation::default(),
                format,
                AudioSettings::default(),
                producer,
                events_rx,
                &settings_rx,
                &sink,
                &stats,
                &stopped,
                &connect_decided,
                &buffer_state,
                control_tx,
            );
        })
    }

    #[test]
    fn device_failure_is_noticed_while_waiting_for_decoder_input() {
        let format = OutputFormat::new(48_000, 2);
        let (producer, _consumer) = pcm_ring(format, format.frames_for_millis(2_000));
        let (tx, rx) = tokio::sync::mpsc::channel::<DecodeInput>(8);
        let sink = Arc::new(Recorder::default());
        let stats = Arc::new(OutputStats::default());
        // The device has already failed; no event is ever sent, so the only way the
        // decode thread can notice is by checking output health while idle.
        stats.output_failed.store(true, Ordering::Relaxed);

        let handle = spawn_decode_loop_for_test(
            format,
            producer,
            rx,
            Arc::clone(&sink) as Arc<dyn EventSink>,
            Arc::clone(&stats),
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !handle.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            handle.is_finished(),
            "the decode thread never noticed the output failure while idle waiting for input"
        );
        handle.join().unwrap();
        drop(tx); // keep alive until here: the scenario is "idle", not "source ended"

        let failure = sink.failure();
        assert!(
            failure.is_some_and(|m| m.contains("output device")),
            "expected an output-device failure, got {:?}",
            sink.snapshot()
        );
        assert!(!sink.has_playing());
    }

    #[test]
    fn device_failure_is_noticed_even_when_the_pcm_ring_stays_full() {
        let format = OutputFormat::new(48_000, 2);
        // Tiny ring: the very first decoded block cannot possibly fit, forcing the
        // ring-full wait almost immediately.
        let (producer, _consumer) = pcm_ring(format, 8);
        let (tx, rx) = tokio::sync::mpsc::channel::<DecodeInput>(2_000);
        let sink = Arc::new(Recorder::default());
        let stats = Arc::new(OutputStats::default());
        // Failed before anything is even queued, so success here can only mean the
        // ring-full wait itself is checking output health.
        stats.output_failed.store(true, Ordering::Relaxed);

        for event in fixture_source_events() {
            tx.try_send(DecodeInput::Event(event))
                .expect("test channel capacity should comfortably fit the fixture");
        }
        drop(tx);

        let handle = spawn_decode_loop_for_test(
            format,
            producer,
            rx,
            Arc::clone(&sink) as Arc<dyn EventSink>,
            Arc::clone(&stats),
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !handle.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            handle.is_finished(),
            "the decode thread spun forever pushing into a full ring instead of noticing \
             the output failure"
        );
        handle.join().unwrap();

        let failure = sink.failure();
        assert!(
            failure.is_some_and(|m| m.contains("output device")),
            "expected an output-device failure, got {:?}",
            sink.snapshot()
        );
        assert!(!sink.has_playing());
    }

    #[test]
    fn a_healthy_ring_full_wait_drains_normally_without_a_spurious_failure() {
        // Control case for the previous two tests: the same tiny ring and full fixture
        // input, but the output is healthy and something drains the ring, so the block
        // should be fully queued and no failure reported.
        let format = OutputFormat::new(48_000, 2);
        let (producer, mut consumer) = pcm_ring(format, format.frames_for_millis(200));
        let (tx, rx) = tokio::sync::mpsc::channel::<DecodeInput>(2_000);
        let sink = Arc::new(Recorder::default());
        let stats = Arc::new(OutputStats::default());

        for event in fixture_source_events() {
            tx.try_send(DecodeInput::Event(event)).unwrap();
        }
        drop(tx);

        // A stand-in "audio callback": drains the ring and marks frames as started, the
        // way the real CPAL callback does, so the gate can leave `Buffering`.
        let drain_stats = Arc::clone(&stats);
        let draining = Arc::new(AtomicBool::new(true));
        let draining_for_thread = Arc::clone(&draining);
        let drain_handle = std::thread::spawn(move || {
            let mut scratch = vec![0.0f32; 4096];
            while draining_for_thread.load(Ordering::Relaxed) {
                let real = consumer.pop_into(&mut scratch);
                if real > 0 {
                    drain_stats.started.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });

        let handle = spawn_decode_loop_for_test(
            format,
            producer,
            rx,
            Arc::clone(&sink) as Arc<dyn EventSink>,
            Arc::clone(&stats),
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !handle.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            handle.is_finished(),
            "decode loop should finish once the source ends"
        );
        handle.join().unwrap();
        draining.store(false, Ordering::Relaxed);
        drain_handle.join().unwrap();

        assert!(
            sink.failure().is_none(),
            "a healthy, draining ring must not report a failure: {:?}",
            sink.snapshot()
        );
    }

    /// `pump_source`'s backpressure wait (a full event queue) must be a proper async wait,
    /// not a blocking `SyncSender::send` call directly inside a Tokio task - that would
    /// starve every other task scheduled on the same worker. Deliberately uses the default
    /// (single-threaded/"constrained") `#[tokio::test]` flavor: a blocking-worker mistake
    /// has nowhere to hide behind a spare thread here. This exercises the exact
    /// `tokio::select!` pattern `pump_source` uses for its send, without needing a real
    /// network/device.
    #[tokio::test]
    async fn full_event_queue_backpressure_does_not_starve_other_work_on_a_single_worker() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u32>(1);
        // Fill the one slot so the next send below must wait for room.
        tx.send(1).await.unwrap();

        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter_task = {
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                for _ in 0..20 {
                    counter.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
        };

        let cancel = CancellationToken::new();
        let cancel_for_send = cancel.clone();
        let send_task = tokio::spawn(async move {
            // Mirrors `pump_source`'s exact backpressure pattern.
            tokio::select! {
                biased;
                () = cancel_for_send.cancelled() => Err(()),
                result = tx.send(2) => result.map_err(|_| ()),
            }
        });

        // Give the counter task a chance to run several ticks while `send_task` sits
        // waiting for room. If the send were a blocking call instead of a proper async
        // wait, this single-worker runtime would never schedule the counter task at all.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            counter.load(Ordering::Relaxed) >= 5,
            "unrelated work on the same worker was starved while backpressure was pending \
             (got {} ticks)",
            counter.load(Ordering::Relaxed)
        );

        // Cancellation - a stop/station-switch - must win over waiting for ring capacity.
        cancel.cancel();
        let started = Instant::now();
        let outcome = send_task.await.unwrap();
        assert!(
            outcome.is_err(),
            "cancellation must be able to interrupt a pending send"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cancellation did not return promptly: {:?}",
            started.elapsed()
        );

        counter_task.abort();
        let _ = rx.recv().await; // keep the receiver alive until here
    }
}

#[cfg(test)]
mod buffer_settings_tests {
    use super::BufferSettings;

    #[test]
    fn buffering_accepts_defaults_and_valid_boundaries() {
        assert!(BufferSettings::default().validate().is_ok());
        assert!(BufferSettings {
            capacity_ms: 100,
            start_ms: 50,
            rebuffer_ms: 0
        }
        .validate()
        .is_ok());
        assert!(BufferSettings {
            capacity_ms: 10_000,
            start_ms: 10_000,
            rebuffer_ms: 9_999
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn buffering_rejects_unreachable_or_unbounded_thresholds() {
        for (capacity_ms, start_ms, rebuffer_ms) in [
            (99, 50, 0),
            (10_001, 500, 150),
            (2000, 49, 0),
            (100, 101, 0),
            (2000, 500, 500),
            (2000, 500, 501),
        ] {
            assert!(BufferSettings {
                capacity_ms,
                start_ms,
                rebuffer_ms
            }
            .validate()
            .is_err());
        }
    }
}
