//! Owns the CPAL output stream on its own thread.
//!
//! `cpal::Stream` is not `Send` on every platform, and the plan is explicit that trait
//! errors here must not be papered over with unsafe `Send`/`Sync` impls. So the stream is
//! created and dropped on one dedicated thread, and everything else talks to it by message.
//!
//! The audio callback itself only pops from a lock-free ring and reports counters through
//! atomics. It never allocates, locks, logs, or touches Tauri.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use apogee_playback_core::dsp::{ControlUpdate, Equalizer};
use apogee_playback_core::output::{ControlConsumer, GatedConsumer, OutputFormat, RingProducer};
use cpal::traits::{DeviceTrait, StreamTrait};

use super::device::{resolve, DeviceDescriptor, DeviceError, DeviceRequest};

/// Counters published by the audio callback and read by ordinary threads.
///
/// Atomics only: the callback must never block a reader, and a reader must never block
/// the callback.
#[derive(Debug, Default)]
pub struct OutputStats {
    /// Frames actually carrying decoded audio.
    pub frames_played: AtomicU64,
    /// Frames the callback had to fill with silence because the ring was dry.
    pub frames_underrun: AtomicU64,
    /// Set once the callback has consumed at least one real frame. This, not a successful
    /// HTTP response, is what confirms playback.
    pub started: AtomicBool,
    /// Set when the backend reports a stream error, so the controller can distinguish an
    /// output failure from network starvation.
    pub output_failed: AtomicBool,
}

impl OutputStats {
    #[must_use]
    pub fn snapshot(&self) -> (u64, u64, bool, bool) {
        (
            self.frames_played.load(Ordering::Relaxed),
            self.frames_underrun.load(Ordering::Relaxed),
            self.started.load(Ordering::Relaxed),
            self.output_failed.load(Ordering::Relaxed),
        )
    }
}

enum Command {
    Stop,
}

struct CallbackPipeline {
    ring: GatedConsumer,
    controls: ControlConsumer<ControlUpdate>,
    analysis: RingProducer,
    visualizer_enabled: Arc<AtomicBool>,
}

/// A running output stream. Dropping this stops and joins the owner thread.
pub struct AudioOutput {
    commands: Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
    format: OutputFormat,
    descriptor: DeviceDescriptor,
    stats: Arc<OutputStats>,
}

/// A device and its negotiated configuration resolved as one unit. Keeping these together
/// prevents the system default from being resolved once for the ring format and again for
/// the stream, which can produce mismatched formats if the default changes between calls.
pub struct PreparedOutput {
    device: cpal::Device,
    descriptor: DeviceDescriptor,
    config: cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    format: OutputFormat,
}

/// The format a device wants, without opening a stream.
///
/// Needed before the ring can be sized, and building a throwaway stream just to read this
/// risks an audible glitch and a needless device grab.
pub fn prepare(request: &DeviceRequest) -> Result<PreparedOutput, DeviceError> {
    let (device, descriptor) = resolve(request)?;
    let supported = device
        .default_output_config()
        .map_err(|e| DeviceError::Backend(e.to_string()))?;
    let format = OutputFormat::new(supported.sample_rate(), supported.channels());
    let sample_format = supported.sample_format();
    let config = supported.into();
    Ok(PreparedOutput {
        device,
        descriptor,
        config,
        sample_format,
        format,
    })
}

impl PreparedOutput {
    #[must_use]
    pub fn format(&self) -> OutputFormat {
        self.format
    }
}

impl AudioOutput {
    /// Open `request` (falling back to the system default) and start pulling from `ring`.
    ///
    /// The device's own preferred configuration is used rather than forcing a rate or
    /// format, so returns the format the caller must resample and convert to.
    pub fn start(
        prepared: PreparedOutput,
        ring: GatedConsumer,
        controls: ControlConsumer<ControlUpdate>,
        analysis: RingProducer,
        visualizer_enabled: Arc<AtomicBool>,
    ) -> Result<Self, DeviceError> {
        let PreparedOutput {
            device,
            descriptor,
            config,
            sample_format,
            format,
        } = prepared;

        let stats = Arc::new(OutputStats::default());
        let (commands, rx) = channel();
        let (ready_tx, ready_rx) = channel();

        let thread_stats = Arc::clone(&stats);
        let pipeline = CallbackPipeline {
            ring,
            controls,
            analysis,
            visualizer_enabled,
        };
        let thread = std::thread::Builder::new()
            .name("apogee-audio-out".to_string())
            .spawn(move || {
                run_stream(
                    device,
                    config,
                    sample_format,
                    pipeline,
                    thread_stats,
                    &ready_tx,
                    &rx,
                );
            })
            .map_err(|e| DeviceError::Backend(format!("could not start audio thread: {e}")))?;

        // Surface a build/play failure to the caller instead of leaving a silent thread.
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                thread: Some(thread),
                format,
                descriptor,
                stats,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(DeviceError::Backend(e))
            }
            Err(_) => Err(DeviceError::Backend(
                "audio thread exited before reporting readiness".to_string(),
            )),
        }
    }

    /// The format the device actually wants. Feed the ring in exactly this.
    #[must_use]
    pub fn format(&self) -> OutputFormat {
        self.format
    }

    /// The device actually opened, which may differ from the one requested.
    #[must_use]
    pub fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn stats(&self) -> &Arc<OutputStats> {
        &self.stats
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            // Joining guarantees no callback is still running against the ring, which is
            // what makes a station switch safe rather than racy.
            let _ = thread.join();
        }
    }
}

fn run_stream(
    device: cpal::Device,
    config: cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    pipeline: CallbackPipeline,
    stats: Arc<OutputStats>,
    ready: &Sender<Result<(), String>>,
    commands: &Receiver<Command>,
) {
    let built = build_stream(&device, &config, sample_format, pipeline, &stats);
    let stream = match built {
        Ok(stream) => stream,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("could not start playback: {e}")));
        return;
    }
    let _ = ready.send(Ok(()));

    // Hold the stream alive on this thread until told to stop. `recv` parks, so this
    // thread costs nothing while playing.
    match commands.recv() {
        Ok(Command::Stop) | Err(_) => {}
    }
    drop(stream);
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    pipeline: CallbackPipeline,
    stats: &Arc<OutputStats>,
) -> Result<cpal::Stream, String> {
    let error_stats = Arc::clone(stats);
    let on_error = move |e: cpal::Error| {
        // Not the callback: logging here is safe.
        error_stats.output_failed.store(true, Ordering::Relaxed);
        log::warn!("audio output stream error: {e}");
    };

    match sample_format {
        cpal::SampleFormat::F32 => build_typed::<f32>(device, config, pipeline, stats, on_error),
        cpal::SampleFormat::I16 => build_typed::<i16>(device, config, pipeline, stats, on_error),
        cpal::SampleFormat::U16 => build_typed::<u16>(device, config, pipeline, stats, on_error),
        other => Err(format!("unsupported output sample format: {other:?}")),
    }
}

/// Builds the stream for one concrete sample type.
///
/// A scratch buffer sized for the largest callback is allocated ONCE here, outside the
/// callback, and reused. The callback itself never allocates.
fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut pipeline: CallbackPipeline,
    stats: &Arc<OutputStats>,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let channels = config.channels.max(1) as usize;
    let max_frames = match config.buffer_size {
        cpal::BufferSize::Fixed(frames) => frames as usize,
        cpal::BufferSize::Default => 8192,
    };
    let sample_rate = config.sample_rate;
    let mut scratch = vec![0.0f32; max_frames.max(1) * channels];
    let mut equalizer = Equalizer::new(config.sample_rate, channels);
    let callback_stats = Arc::clone(stats);

    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                let wanted = out.len();
                if scratch.len() < wanted {
                    // Only reachable if the host asks for more than it advertised. Emit
                    // silence rather than allocating on the audio thread.
                    out.fill(T::from_sample(0.0f32));
                    callback_stats
                        .frames_underrun
                        .fetch_add((wanted / channels) as u64, Ordering::Relaxed);
                    return;
                }
                let buffer = &mut scratch[..wanted];
                let real_frames = pipeline.ring.pop_into(buffer);
                let total_frames = wanted / channels;

                // Coefficients were prepared off-callback. Applying an update and processing
                // this preallocated buffer are both bounded and allocation-free.
                while let Some(update) = pipeline.controls.try_recv() {
                    equalizer.apply_coefficients(&update.eq);
                    equalizer.set_gain(update.volume, update.muted, sample_rate / 50);
                }
                // Gated buffering promises exact silence and must not advance EQ/gain
                // state through synthetic frames. Once real audio is present, processing
                // the whole callback (including an underrun tail) lets filters decay
                // naturally at the end of a partially filled playing callback.
                if real_frames > 0 {
                    equalizer.process(buffer);
                }

                // The visualizer sees exactly the post-control samples consumed here. Its
                // bounded ring drops excess samples rather than ever waiting in the callback.
                if pipeline.visualizer_enabled.load(Ordering::Relaxed) && real_frames > 0 {
                    let real_samples = real_frames * channels;
                    let _ = pipeline.analysis.push_frames(&buffer[..real_samples]);
                }

                for (dst, src) in out.iter_mut().zip(buffer.iter()) {
                    *dst = T::from_sample(*src);
                }

                if real_frames > 0 {
                    callback_stats
                        .frames_played
                        .fetch_add(real_frames as u64, Ordering::Relaxed);
                    callback_stats.started.store(true, Ordering::Relaxed);
                }
                if real_frames < total_frames {
                    callback_stats
                        .frames_underrun
                        .fetch_add((total_frames - real_frames) as u64, Ordering::Relaxed);
                }
            },
            on_error,
            None,
        )
        .map_err(|e| format!("could not build output stream: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apogee_playback_core::dsp::EqCoefficients;
    use apogee_playback_core::output::{control_channel, pcm_ring, BufferStateFlag};
    use std::time::{Duration, Instant};

    /// Prefers a null/dummy sink so the suite makes no audible noise. Falls back to the
    /// system default only if no such device exists.
    fn quiet_request() -> Option<DeviceRequest> {
        let devices = super::super::device::list_output_devices().ok()?;
        if devices.is_empty() {
            return None;
        }
        let null = devices
            .iter()
            .find(|d| d.id.contains("null") || d.name.to_lowercase().contains("discard"));
        null.map(|d| DeviceRequest::Specific(d.id.clone()))
    }

    fn start_test_output(
        request: &DeviceRequest,
        consumer: apogee_playback_core::output::RingConsumer,
    ) -> AudioOutput {
        let prepared = prepare(request).expect("device should resolve");
        let format = prepared.format();
        let state = Arc::new(BufferStateFlag::default());
        let gated = GatedConsumer::new(consumer, 1, 0, state);
        let (mut controls, control_rx) = control_channel(2);
        controls
            .try_send(ControlUpdate {
                eq: EqCoefficients::build(f64::from(format.sample_rate), false, &[0.0; 10])
                    .unwrap(),
                volume: 100,
                muted: false,
            })
            .unwrap();
        let (analysis, _analysis_rx) = pcm_ring(format, 1024);
        AudioOutput::start(
            prepared,
            gated,
            control_rx,
            analysis,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("stream should start")
    }

    #[test]
    fn the_callback_consumes_queued_audio_and_confirms_playback() {
        let Some(request) = quiet_request() else {
            eprintln!("skipping: no silent output device available on this machine");
            return;
        };

        let format = OutputFormat::new(48_000, 2);
        let (mut producer, consumer) = pcm_ring(format, format.frames_for_millis(2_000));

        // Queue a second of quiet audio before starting, so there is something to consume.
        let frames = format.frames_for_millis(1_000);
        let block: Vec<f32> = (0..frames * 2)
            .map(|i| ((i % 100) as f32) * 0.0001)
            .collect();
        producer.push_frames(&block);

        let output = start_test_output(&request, consumer);
        assert!(!output.descriptor().id.is_empty());

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let (played, _underrun, started, failed) = output.stats().snapshot();
            assert!(!failed, "the output backend reported a failure");
            if started && played > 0 {
                break;
            }
            if Instant::now() > deadline {
                panic!("callback never consumed a frame: played={played} started={started}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn an_empty_ring_underruns_into_silence_rather_than_failing() {
        let Some(request) = quiet_request() else {
            eprintln!("skipping: no silent output device available on this machine");
            return;
        };

        let format = OutputFormat::new(48_000, 2);
        // Never fed: every callback must underrun cleanly.
        let (_producer, consumer) = pcm_ring(format, format.frames_for_millis(500));
        let output = start_test_output(&request, consumer);

        std::thread::sleep(Duration::from_millis(300));
        let (played, underrun, _started, failed) = output.stats().snapshot();
        assert!(
            !failed,
            "starvation must not be reported as an output failure"
        );
        assert_eq!(played, 0, "nothing was queued, so nothing can have played");
        assert!(
            underrun > 0,
            "a dry ring should be reported as underrun, got {underrun}"
        );
    }

    #[test]
    fn dropping_the_output_stops_and_joins_its_thread() {
        let Some(request) = quiet_request() else {
            eprintln!("skipping: no silent output device available on this machine");
            return;
        };
        let format = OutputFormat::new(48_000, 2);
        let (_producer, consumer) = pcm_ring(format, 4096);
        let output = start_test_output(&request, consumer);
        // Dropping must not hang: the owner thread parks on recv and exits on Stop.
        let start = Instant::now();
        drop(output);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "dropping the output should stop promptly, took {:?}",
            start.elapsed()
        );
    }
}
