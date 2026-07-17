use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::io::ErrorKind;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

const WINDOW_SIZE: usize = 1024;
/// 8 bands, log-spaced across the audible range.
const BAND_EDGES_HZ: [f32; 9] = [
    20.0, 150.0, 400.0, 1000.0, 2000.0, 4000.0, 8000.0, 12000.0, 20000.0,
];

/// Real captured audio has a big systematic tilt across these 8 log-spaced
/// bands - measured band averages during actual playback ran from -40dB
/// (bass) down to -87dB (treble), a ~47dB gap, consistent across different
/// songs. That's the natural spectral tilt of real music energy (compounded
/// by high bands averaging over far more FFT bins than low bands), and no
/// amount of floor/ceiling tuning fixes a *relative* gap between bands - it
/// just shifts everything together. So each band gets a fixed additive dB
/// offset, derived from that measured data (roughly: shift every band's
/// typical level to line up near a shared average), applied before the
/// floor/ceiling mapping below. This is an additive shift, not a gain -
/// it doesn't amplify a band's own noise/dynamics, it just repositions its
/// baseline to match the others.
const BAND_TILT_COMPENSATION_DB: [f32; 8] = [-18.0, -15.0, -5.0, -1.0, 0.0, 5.0, 10.0, 22.0];

/// Fixed dB reference range mapped onto the 0..1 output range, applied after
/// tilt compensation above. Every earlier attempt here used some form of
/// *adaptive* reference (a ceiling the signal gets measured against), and
/// all of them failed the same way: a reference that adapts to the same
/// signal it's measuring ends up chasing it, which either pins the ratio
/// near 1.0 (instant attack) or collapses genuine dynamics (independent
/// per-band adaptation). Real spectrum visualizers (Web Audio's
/// `AnalyserNode.minDecibels`/`maxDecibels`, audioMotion-analyzer, etc.) use
/// a fixed, calibrated range instead - these constants are calibrated
/// against this app's actual capture pipeline output post-compensation
/// (typical compensated averages clustered -56 to -66dB, compensated peaks
/// -43 to -55dB), not theoretical single-tone math.
const LEVEL_FLOOR_DB: f32 = -70.0;
const LEVEL_CEILING_DB: f32 = -35.0;

/// Fast attack so transients still visibly pop, slower release (VU-meter-
/// style) so a bar falls naturally between hits instead of snapping to zero
/// or staying pinned high.
const LEVEL_ATTACK_SECONDS: f32 = 0.05;
const LEVEL_RELEASE_SECONDS: f32 = 0.3;

fn level_from_range(db: f32) -> f32 {
    ((db - LEVEL_FLOOR_DB) / (LEVEL_CEILING_DB - LEVEL_FLOOR_DB)).clamp(0.0, 1.0)
}

/// The fixed floor/ceiling above is calibrated for *a* volume level, but the
/// capture pipeline sits downstream of system/app volume - turn playback up
/// and every band's dB rises together, turn it down and everything shrinks.
/// A fixed range alone forces a tradeoff between "big enough to read" and
/// "doesn't clip when the volume's turned up". This tracks how loud things
/// generally are right now (the loudest band, smoothed over several
/// seconds - much slower than any beat or musical phrase) and computes an
/// offset that keeps that trend sitting at a consistent spot in the range.
/// Crucially this is a *separate, slow* correction from `LevelSmoother`'s
/// fast attack/release: actual musical dynamics (a kick hit, a quiet verse)
/// still come through in full on top of it, only the sustained "what volume
/// is this" baseline gets normalized out.
const VOLUME_ADAPT_SECONDS: f32 = 8.0;
const VOLUME_TARGET_DB: f32 = -40.0;
/// Below this, treat it as silence (stream loading/buffering, a pause, a gap
/// between tracks) rather than "quiet volume" and don't adapt to it. Without
/// this, a few seconds of startup silence drags the tracked average way
/// down chasing it; when real audio then arrives, the gap between that
/// drifted-down average and the actual signal produces a large positive
/// offset that pins everything near the ceiling until the average claws
/// its way back over several seconds - the "starts maxed, slowly recovers"
/// bug. Freezing the average during silence means it stays at its last
/// good (or seed) value, so the offset starts near neutral once real audio
/// resumes instead of correcting from an extreme.
const VOLUME_TRACKER_MIN_ACTIVITY_DB: f32 = -75.0;

/// Retry backoff for (re)opening a capture source: short delays at first so
/// a transient hiccup (sink switch, PipeWire restart, a device momentarily
/// disappearing) recovers in a few seconds, capped so a platform/session
/// with no capture backend at all doesn't spin or spam logs.
const RETRY_DELAYS_SECONDS: [u64; 5] = [1, 2, 5, 10, 30];
const RETRY_DELAY_CEILING_SECONDS: u64 = 60;

struct RetryBackoff {
    attempt: usize,
}

impl RetryBackoff {
    fn new() -> Self {
        Self { attempt: 0 }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    fn next_delay(&mut self) -> Duration {
        let seconds = RETRY_DELAYS_SECONDS
            .get(self.attempt)
            .copied()
            .unwrap_or(RETRY_DELAY_CEILING_SECONDS);
        self.attempt += 1;
        Duration::from_secs(seconds)
    }
}

struct VolumeTracker {
    avg_db: f32,
}

impl VolumeTracker {
    fn new() -> Self {
        Self {
            avg_db: VOLUME_TARGET_DB,
        }
    }

    fn offset(&mut self, tilt_compensated_db: &[f32], frame_seconds: f32) -> f32 {
        let loudest = tilt_compensated_db.iter().cloned().fold(f32::MIN, f32::max);
        if loudest > VOLUME_TRACKER_MIN_ACTIVITY_DB {
            let alpha = 1.0 - (-frame_seconds / VOLUME_ADAPT_SECONDS).exp();
            self.avg_db += (loudest - self.avg_db) * alpha;
        }
        VOLUME_TARGET_DB - self.avg_db
    }
}

/// Per-band asymmetric attack/release smoothing applied directly to the
/// displayed level, the same way a VU/PPM meter smooths its needle - not to
/// an adaptive reference the level is divided by.
struct LevelSmoother {
    levels: Vec<f32>,
}

impl LevelSmoother {
    fn new(bands: usize) -> Self {
        Self {
            levels: vec![0.0; bands],
        }
    }

    fn smooth(&mut self, targets: &[f32], frame_seconds: f32) -> Vec<f32> {
        let attack = 1.0 - (-frame_seconds / LEVEL_ATTACK_SECONDS).exp();
        let release = 1.0 - (-frame_seconds / LEVEL_RELEASE_SECONDS).exp();
        targets
            .iter()
            .zip(self.levels.iter_mut())
            .map(|(&target, level)| {
                let alpha = if target > *level { attack } else { release };
                *level += (target - *level) * alpha;
                *level
            })
            .collect()
    }
}

static STARTED: OnceLock<()> = OnceLock::new();

/// The audio device the visualizer should capture, kept in lockstep with the
/// device mpv plays to so the bars react to the audio the user actually hears.
/// `name` is mpv's `audio-device-list` name (the shared identity across
/// playback and capture); `description` is its friendly label, needed to match
/// the device on Windows via cpal. `None` means "system default".
#[derive(Clone, Debug)]
pub struct SelectedAudioDevice {
    pub name: String,
    // Only read by the Windows opener (to match the cpal output device by its
    // friendly name); Linux derives everything from `name` and macOS ignores
    // the selection, so this is dead code on those targets.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub description: String,
}

/// Set once in `ensure_started`; `waveform_set_device` pushes new selections
/// through it and `run_capture_loop` reopens capture on the new target.
static WAVEFORM_DEVICE: OnceLock<watch::Sender<Option<SelectedAudioDevice>>> = OnceLock::new();

/// Points the visualizer at a specific output device (or `None` for the system
/// default). Applied live - the capture loop tears down the current source and
/// reopens against the new device without restarting playback.
#[tauri::command]
pub fn waveform_set_device(name: Option<String>, description: Option<String>) {
    let selected = name.map(|name| SelectedAudioDevice {
        name,
        description: description.unwrap_or_default(),
    });
    if let Some(tx) = WAVEFORM_DEVICE.get() {
        // The only receiver lives in run_capture_loop for the app's lifetime;
        // a send error would just mean capture never started, nothing to do.
        let _ = tx.send(selected);
    }
}

/// Starts a real-time spectrum analyzer once per app lifetime: captures
/// system audio output directly (not mpv's internal state - mpv's own
/// af-metadata mechanism was tested and cannot expose more than one overall
/// level, since ffmpeg's amix/merge filters drop per-branch metadata), runs
/// an FFT over a rolling window, and emits normalized per-band levels on
/// "waveform-levels". If no capture backend is available (e.g. no
/// PipeWire/PulseAudio on Linux, no default output device, or audio-capture
/// permission denied on macOS), this retries in the background with backoff
/// and the frontend keeps its synthetic fallback animation in the meantime.
pub fn ensure_started(app: &AppHandle) {
    if STARTED.set(()).is_err() {
        return;
    }
    let app = app.clone();
    // Create the device channel before the loop starts so waveform_set_device
    // (which may fire as soon as the frontend finishes loading settings) always
    // has a live sender to push through.
    let (device_tx, device_rx) = watch::channel(None);
    let _ = WAVEFORM_DEVICE.set(device_tx);
    // tokio::spawn requires an active Tokio task context; ensure_started is
    // called from Tauri's synchronous .setup() closure, so it must go through
    // Tauri's own runtime handle instead.
    tauri::async_runtime::spawn(async move {
        run_capture_loop(app, device_rx).await;
    });
}

/// A live capture source: mono samples at `sample_rate` arrive on `rx` for as
/// long as the platform-specific capture thread/process stays healthy. When
/// it ends (device change, process died, transient error), `rx` closes and
/// `run_capture_loop` reopens a fresh source after a backoff delay.
struct CaptureSource {
    rx: mpsc::Receiver<Vec<f32>>,
    sample_rate: u32,
    backend_name: &'static str,
}

async fn run_capture_loop(
    app: AppHandle,
    mut device_rx: watch::Receiver<Option<SelectedAudioDevice>>,
) {
    let mut backoff = RetryBackoff::new();
    loop {
        let selected = device_rx.borrow().clone();
        match open_capture_source(selected).await {
            Ok(CaptureSource {
                rx,
                sample_rate,
                backend_name,
            }) => {
                log::info!("waveform: capturing via {backend_name} at {sample_rate}Hz");
                backoff.reset();
                // Run until the source ends on its own, or the selected device
                // changes - in which case drop this source (killing the capture
                // process/stream) and immediately reopen against the new target,
                // skipping the retry backoff.
                tokio::select! {
                    _ = process_samples(&app, rx, sample_rate) => {
                        log::warn!("waveform: capture source ({backend_name}) ended, retrying");
                    }
                    changed = device_rx.changed() => {
                        if changed.is_ok() {
                            log::info!("waveform: audio device changed, reopening capture");
                            continue;
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "waveform: no audio capture backend available right now ({e}), retrying"
                );
            }
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// Consumes mono sample chunks from `rx`, running the shared FFT -> tilt
/// compensation -> volume offset -> smoothing -> emit pipeline, accumulating
/// chunks into fixed `WINDOW_SIZE` windows regardless of how the upstream
/// source happened to batch them.
async fn process_samples(app: &AppHandle, mut rx: mpsc::Receiver<Vec<f32>>, sample_rate: u32) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);
    let band_bins = band_bin_ranges(sample_rate);
    let mut smoother = LevelSmoother::new(band_bins.len());
    let mut volume_tracker = VolumeTracker::new();
    let frame_seconds = WINDOW_SIZE as f32 / sample_rate as f32;

    let mut window: Vec<f32> = Vec::with_capacity(WINDOW_SIZE);
    while let Some(chunk) = rx.recv().await {
        window.extend_from_slice(&chunk);
        while window.len() >= WINDOW_SIZE {
            let samples: Vec<f32> = window.drain(..WINDOW_SIZE).collect();

            let band_db = compute_band_db(&samples, fft.as_ref(), &band_bins);
            let tilt_compensated: Vec<f32> = band_db
                .iter()
                .enumerate()
                .map(|(i, &db)| db + BAND_TILT_COMPENSATION_DB[i])
                .collect();
            let volume_offset = volume_tracker.offset(&tilt_compensated, frame_seconds);
            let targets: Vec<f32> = tilt_compensated
                .iter()
                .map(|&db| level_from_range(db + volume_offset))
                .collect();
            let levels = smoother.smooth(&targets, frame_seconds);
            let _ = app.emit("waveform-levels", levels);
        }
    }
}

async fn open_capture_source(
    selected: Option<SelectedAudioDevice>,
) -> std::io::Result<CaptureSource> {
    #[cfg(target_os = "linux")]
    {
        open_linux_capture_source(selected).await
    }
    #[cfg(target_os = "windows")]
    {
        open_windows_capture_source(selected).await
    }
    #[cfg(target_os = "macos")]
    {
        open_macos_capture_source(selected).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = selected;
        Err(std::io::Error::new(
            ErrorKind::Unsupported,
            "audio loopback capture not implemented on this OS",
        ))
    }
}

const LINUX_SAMPLE_RATE: u32 = 44100;

#[cfg(target_os = "linux")]
async fn spawn_capture_process(
    selected: Option<SelectedAudioDevice>,
) -> std::io::Result<(Child, &'static str)> {
    // On PipeWire/PulseAudio, mpv's `pulse/<sink>` device name is exactly the
    // sink name, and that sink's monitor is `<sink>.monitor` - so a selected
    // device maps cleanly onto pw-record's `--target` and parec's `--device`.
    // For "system default" (None) or a non-pulse mpv AO (e.g. raw ALSA, whose
    // name doesn't carry a pulse sink), fall back to the default sink monitor.
    let (pw_target, parec_device) = match &selected {
        Some(dev) if dev.name.starts_with("pulse/") => {
            let sink = dev.name.trim_start_matches("pulse/");
            (format!("--target={sink}"), format!("--device={sink}.monitor"))
        }
        _ => (
            "--target=@DEFAULT_AUDIO_SINK@".to_string(),
            "--device=@DEFAULT_SINK@.monitor".to_string(),
        ),
    };

    // This app's own dev/test environment runs PipeWire (pw-record); classic
    // PulseAudio-only systems get parec as a fallback.
    let pw_record = Command::new("pw-record")
        .args([
            &pw_target,
            "--media-category=Capture",
            "--media-role=Music",
            &format!("--rate={LINUX_SAMPLE_RATE}"),
            "--channels=1",
            "--format=s16",
            "--raw",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();

    match pw_record {
        Ok(child) => Ok((child, "pw-record")),
        Err(e) if e.kind() == ErrorKind::NotFound => Command::new("parec")
            .args([
                &parec_device,
                &format!("--rate={LINUX_SAMPLE_RATE}"),
                "--format=s16le",
                "--channels=1",
                "--raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map(|child| (child, "parec")),
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "linux")]
async fn open_linux_capture_source(
    selected: Option<SelectedAudioDevice>,
) -> std::io::Result<CaptureSource> {
    let (mut child, backend_name) = spawn_capture_process(selected).await?;
    let Some(mut stdout) = child.stdout.take() else {
        return Err(std::io::Error::other(
            "pw-record/parec spawned without a stdout pipe",
        ));
    };

    let (tx, rx) = mpsc::channel::<Vec<f32>>(4);
    tokio::spawn(async move {
        // Keep the child alive for the duration of the reader loop; it's
        // killed on drop (kill_on_drop) once this task ends.
        let _child = child;
        let mut byte_buf = vec![0u8; WINDOW_SIZE * 2];
        loop {
            if stdout.read_exact(&mut byte_buf).await.is_err() {
                break;
            }
            let samples: Vec<f32> = byte_buf
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                .collect();
            if tx.send(samples).await.is_err() {
                break;
            }
        }
    });

    Ok(CaptureSource {
        rx,
        sample_rate: LINUX_SAMPLE_RATE,
        backend_name,
    })
}

fn band_bin_ranges(sample_rate: u32) -> Vec<(usize, usize)> {
    let bin_hz = sample_rate as f32 / WINDOW_SIZE as f32;
    let nyquist_bin = WINDOW_SIZE / 2;
    BAND_EDGES_HZ
        .windows(2)
        .map(|edge| {
            let lo = (edge[0] / bin_hz).floor() as usize;
            let hi = ((edge[1] / bin_hz).ceil() as usize).clamp(lo + 1, nyquist_bin);
            (lo.min(nyquist_bin - 1), hi)
        })
        .collect()
}

/// Raw per-band loudness in dB (unclamped, arbitrary reference). Absolute
/// calibration is handled by `LEVEL_FLOOR_DB`/`LEVEL_CEILING_DB` - this just
/// needs to be internally consistent frame to frame, which dividing out the
/// FFT's window-size-dependent scale (rustfft's forward transform is
/// unnormalized) gives us.
fn compute_band_db(samples: &[f32], fft: &dyn Fft<f32>, band_bins: &[(usize, usize)]) -> Vec<f32> {
    let n = samples.len();
    let mut buffer: Vec<Complex<f32>> = samples
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos();
            Complex::new(s * hann, 0.0)
        })
        .collect();
    fft.process(&mut buffer);

    let norm = n as f32 / 2.0;
    band_bins
        .iter()
        .map(|&(lo, hi)| {
            let sum: f32 = buffer[lo..hi].iter().map(|c| c.norm() / norm).sum();
            let avg = sum / (hi - lo).max(1) as f32;
            20.0 * avg.max(1e-6).log10()
        })
        .collect()
}

#[cfg(target_os = "windows")]
mod windows_capture;
#[cfg(target_os = "windows")]
use windows_capture::open_windows_capture_source;

#[cfg(target_os = "macos")]
mod macos_capture;
#[cfg(target_os = "macos")]
use macos_capture::open_macos_capture_source;
