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
//! Cancellation reaches every stage: the token stops network waits, and dropping the event
//! channel wakes the decode thread even when it is blocked waiting for input.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use apogee_playback_core::analysis::{SpectrumAnalyzer, BAND_COUNT};
use apogee_playback_core::dsp::Equalizer;
use apogee_playback_core::output::{
    pcm_ring, BufferGate, ChannelConverter, OutputFormat, PcmResampler, RingProducer,
};
use apogee_playback_core::pipeline::Pipeline;
use apogee_playback_core::session::{BufferingReason, ErrorClass, Generation};
use tokio_util::sync::CancellationToken;

use super::audio_out::AudioOutput;
use super::device::DeviceRequest;
use super::source::{self, SourceError};
use crate::network::NetworkService;

/// How much decoded audio to hold before playback starts, and total ring capacity.
/// Internal thresholds, tuned against tests, not latency guarantees.
const START_BUFFER_MS: u64 = 500;
const RING_CAPACITY_MS: u64 = 2_000;
const REBUFFER_MS: u64 = 150;
/// Bounded queue between the network task and the decode thread.
const EVENT_QUEUE_DEPTH: usize = 256;

/// Audio settings that can change while a session runs.
#[derive(Debug, Clone)]
pub struct AudioSettings {
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
    // Dropped before the thread is joined, which wakes a blocked decode loop.
    events_tx: Option<std::sync::mpsc::SyncSender<DecodeInput>>,
    output: Option<AudioOutput>,
    settings_tx: std::sync::mpsc::Sender<AudioSettings>,
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
        let _ = self.settings_tx.send(settings);
    }

    /// The device actually in use, which may differ from the one requested.
    #[must_use]
    pub fn device_name(&self) -> Option<String> {
        self.output.as_ref().map(|o| o.descriptor().name.clone())
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
        // Order matters: dropping the sender wakes a decode thread parked on recv, and
        // dropping the output joins the audio thread so no callback can still touch the
        // ring afterwards.
        self.events_tx = None;
        self.output = None;
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
    let cancel = CancellationToken::new();
    let stopped = Arc::new(AtomicBool::new(false));

    // The device's format decides what the decode thread must produce, so it has to be
    // known before the ring is sized. Query it directly rather than opening a throwaway
    // stream, which would grab the device twice and risk an audible glitch.
    let format = super::audio_out::preferred_format(device).map_err(|e| e.to_string())?;

    let (producer, consumer) = pcm_ring(format, format.frames_for_millis(RING_CAPACITY_MS));
    let output = AudioOutput::start(device, consumer).map_err(|e| e.to_string())?;

    let (events_tx, events_rx) = std::sync::mpsc::sync_channel(EVENT_QUEUE_DEPTH);
    let (settings_tx, settings_rx) = std::sync::mpsc::channel();

    let decode_sink = Arc::clone(&sink);
    let decode_stats = Arc::clone(output.stats());
    let decode_stopped = Arc::clone(&stopped);
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
            );
        })
        .map_err(|e| format!("could not start decode thread: {e}"))?;

    // Network side: async, cancellable, feeds the bounded queue.
    let net_tx = events_tx.clone();
    let net_cancel = cancel.clone();
    let net_sink = Arc::clone(&sink);
    tokio::spawn(async move {
        let outcome = pump_source(&url, &network, &net_cancel, &net_tx).await;
        if let Err(ref e) = outcome {
            log::warn!("playback source ended: {e}");
        }
        let _ = net_tx.send(DecodeInput::Finished(outcome));
        drop(net_sink);
    });

    Ok(Session {
        generation,
        cancel,
        decode_thread: Some(decode_thread),
        events_tx: Some(events_tx),
        output: Some(output),
        settings_tx,
        stopped,
    })
}

/// Drives the source, forwarding demuxed events to the decode thread.
async fn pump_source(
    url: &str,
    network: &NetworkService,
    cancel: &CancellationToken,
    tx: &std::sync::mpsc::SyncSender<DecodeInput>,
) -> Result<(), SourceError> {
    let mut src = source::open(url, network, cancel).await?;
    loop {
        if cancel.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        match src.next_event().await? {
            Some(event) => {
                // A full queue means the decoder is behind; blocking here is correct
                // backpressure. The send fails only once the receiver is gone, i.e. stop.
                if tx.send(DecodeInput::Event(event)).is_err() {
                    return Ok(());
                }
            }
            None => return Ok(()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_loop(
    generation: Generation,
    format: OutputFormat,
    settings: AudioSettings,
    mut producer: RingProducer,
    events: std::sync::mpsc::Receiver<DecodeInput>,
    settings_rx: &std::sync::mpsc::Receiver<AudioSettings>,
    sink: &Arc<dyn EventSink>,
    stats: &Arc<super::audio_out::OutputStats>,
    stopped: &Arc<AtomicBool>,
) {
    let mut pipeline = Pipeline::new();
    let mut equalizer = Equalizer::new(format.sample_rate, format.channels as usize);
    apply_settings(&mut equalizer, &settings);

    let mut converter: Option<ChannelConverter> = None;
    let mut resampler: Option<PcmResampler> = None;
    let mut source_rate: Option<u32> = None;
    let mut gate = BufferGate::new(
        format.frames_for_millis(START_BUFFER_MS),
        format.frames_for_millis(REBUFFER_MS),
    );
    let mut analyzer: Option<SpectrumAnalyzer> = None;
    let mut visualizer_enabled = settings.visualizer_enabled;
    let mut announced_playing = false;
    let mut last_underrun = 0u64;
    let mut last_bitrate: Option<u32> = None;

    sink.emit(EngineEvent::Buffering {
        generation,
        reason: BufferingReason::Connecting,
    });

    while let Ok(input) = events.recv() {
        if stopped.load(Ordering::Relaxed) {
            return;
        }
        // Settings changes are applied between blocks, never inside the audio callback.
        while let Ok(next) = settings_rx.try_recv() {
            visualizer_enabled = next.visualizer_enabled;
            if !visualizer_enabled {
                // Drop the analyser entirely rather than keep feeding a hidden display.
                analyzer = None;
            }
            apply_settings(&mut equalizer, &next);
        }

        let event = match input {
            DecodeInput::Event(event) => event,
            DecodeInput::Finished(Ok(())) => {
                sink.emit(EngineEvent::Failed {
                    generation,
                    class: ErrorClass::Transient,
                    message: "the stream ended".to_string(),
                });
                return;
            }
            DecodeInput::Finished(Err(error)) => {
                let class = classify(&error);
                sink.emit(EngineEvent::Failed {
                    generation,
                    class,
                    message: error.to_string(),
                });
                return;
            }
        };

        let block = match pipeline.accept(event) {
            Ok(Some(block)) => block,
            Ok(None) => continue,
            Err(error) => {
                sink.emit(EngineEvent::Failed {
                    generation,
                    class: ErrorClass::Permanent,
                    message: error.to_string(),
                });
                return;
            }
        };

        // Configure conversion on the first block, and again if the stream changes format.
        if source_rate != Some(block.rate) {
            source_rate = Some(block.rate);
            converter = Some(ChannelConverter::new(
                block.channels as u16,
                format.channels,
            ));
            resampler = match PcmResampler::new(block.rate, format) {
                Ok(r) => Some(r),
                Err(e) => {
                    sink.emit(EngineEvent::Failed {
                        generation,
                        class: ErrorClass::Permanent,
                        message: format!("unsupported audio format: {e}"),
                    });
                    return;
                }
            };
            sink.emit(EngineEvent::Format {
                generation,
                sample_rate: block.rate,
                channels: block.channels as u16,
            });
        }

        let converted = match converter.as_mut() {
            Some(c) => c.convert(&block.samples).to_vec(),
            None => block.samples.clone(),
        };
        let resampled = match resampler.as_mut() {
            Some(r) => match r.process(&converted) {
                Ok(pcm) => pcm.to_vec(),
                Err(e) => {
                    sink.emit(EngineEvent::Failed {
                        generation,
                        class: ErrorClass::Permanent,
                        message: format!("resampling failed: {e}"),
                    });
                    return;
                }
            },
            None => converted,
        };

        let mut ready = resampled;
        // EQ and volume are applied here, close to consumption, so a volume change is not
        // delayed by everything already queued.
        equalizer.process(&mut ready);

        // Tap post-EQ and post-volume, so the display reflects what Apogee actually sends
        // to the device. Analysis never blocks playback: it happens before queueing and
        // simply produces nothing when a window is incomplete.
        if visualizer_enabled {
            let analyzer = analyzer.get_or_insert_with(|| {
                SpectrumAnalyzer::new(format.sample_rate, format.channels as usize)
            });
            if let Some(levels) = analyzer.push(&ready) {
                sink.emit(EngineEvent::Spectrum { levels });
            }
        }

        let mut offset = 0usize;
        while offset < ready.len() {
            if stopped.load(Ordering::Relaxed) {
                return;
            }
            let written = producer.push_frames(&ready[offset..]);
            if written == 0 {
                // Ring full: the device has plenty queued. Wait a little rather than spin.
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            offset += written * format.channels as usize;
        }

        if gate.update(producer.occupied_frames()) && !announced_playing {
            // Only announce once the callback has genuinely consumed audio.
            let (_played, _under, started, _failed) = stats.snapshot();
            if started {
                announced_playing = true;
                sink.emit(EngineEvent::Playing { generation });
            }
        }

        let (_played, underrun, started, failed) = stats.snapshot();
        if started && !announced_playing {
            announced_playing = true;
            sink.emit(EngineEvent::Playing { generation });
        }
        if failed {
            sink.emit(EngineEvent::Failed {
                generation,
                class: ErrorClass::Transient,
                message: "the audio output device failed".to_string(),
            });
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

fn apply_settings(equalizer: &mut Equalizer, settings: &AudioSettings) {
    if let Err(e) = equalizer.configure(settings.equalizer_enabled, &settings.equalizer_gains) {
        log::warn!("ignoring invalid equalizer settings: {e}");
    }
    // Ramp over roughly 20 ms so changes cannot click.
    equalizer.set_gain(settings.volume, settings.muted, 960);
}

/// Whether an error is worth retrying. Authentication and unsupported content are not.
fn classify(error: &SourceError) -> ErrorClass {
    match error {
        SourceError::Unsupported(_) => ErrorClass::Permanent,
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
}
