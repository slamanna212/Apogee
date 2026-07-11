use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::io::ErrorKind;
use std::process::Stdio;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

const SAMPLE_RATE: u32 = 44100;
const WINDOW_SIZE: usize = 1024;
/// 8 bands, log-spaced across the audible range.
const BAND_EDGES_HZ: [f32; 9] = [20.0, 150.0, 400.0, 1000.0, 2000.0, 4000.0, 8000.0, 12000.0, 20000.0];

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

struct VolumeTracker {
    avg_db: f32,
}

impl VolumeTracker {
    fn new() -> Self {
        Self { avg_db: VOLUME_TARGET_DB }
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
        Self { levels: vec![0.0; bands] }
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

/// Starts a real-time spectrum analyzer once per app lifetime: captures
/// system audio output directly (not mpv's internal state - mpv's own
/// af-metadata mechanism was tested and cannot expose more than one overall
/// level, since ffmpeg's amix/merge filters drop per-branch metadata), runs
/// an FFT over a rolling window, and emits normalized per-band levels on
/// "waveform-levels". If no capture backend is available (e.g. no
/// PipeWire/PulseAudio, or on platforms without one implemented yet), this
/// silently does nothing and the frontend keeps its synthetic fallback
/// animation.
pub fn ensure_started(app: &AppHandle) {
    if STARTED.set(()).is_err() {
        return;
    }
    let app = app.clone();
    // tokio::spawn requires an active Tokio task context; ensure_started is
    // called from Tauri's synchronous .setup() closure, so it must go through
    // Tauri's own runtime handle instead.
    tauri::async_runtime::spawn(async move {
        run_capture_loop(app).await;
    });
}

#[cfg(target_os = "linux")]
async fn spawn_capture_process() -> std::io::Result<Child> {
    // This app's own dev/test environment runs PipeWire (pw-record); classic
    // PulseAudio-only systems get parec as a fallback.
    let pw_record = Command::new("pw-record")
        .args([
            "--target=@DEFAULT_AUDIO_SINK@",
            "--media-category=Capture",
            "--media-role=Music",
            &format!("--rate={SAMPLE_RATE}"),
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
        Ok(child) => Ok(child),
        Err(e) if e.kind() == ErrorKind::NotFound => Command::new("parec")
            .args([
                "--device=@DEFAULT_SINK@.monitor",
                &format!("--rate={SAMPLE_RATE}"),
                "--format=s16le",
                "--channels=1",
                "--raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn(),
        Err(e) => Err(e),
    }
}

/// No loopback-capture backend implemented yet for this platform - the
/// frontend's synthetic fallback animation covers it in the meantime.
#[cfg(not(target_os = "linux"))]
async fn spawn_capture_process() -> std::io::Result<Child> {
    Err(std::io::Error::new(ErrorKind::Unsupported, "audio loopback capture not implemented on this OS"))
}

async fn run_capture_loop(app: AppHandle) {
    let mut child = match spawn_capture_process().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("waveform: no audio capture backend available ({e}) - using synthetic animation instead");
            return;
        }
    };
    let Some(mut stdout) = child.stdout.take() else {
        return;
    };

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);
    let band_bins = band_bin_ranges();
    let mut byte_buf = vec![0u8; WINDOW_SIZE * 2];
    let mut smoother = LevelSmoother::new(band_bins.len());
    let mut volume_tracker = VolumeTracker::new();
    let frame_seconds = WINDOW_SIZE as f32 / SAMPLE_RATE as f32;

    loop {
        if stdout.read_exact(&mut byte_buf).await.is_err() {
            break;
        }
        let samples: Vec<f32> = byte_buf
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();

        let band_db = compute_band_db(&samples, fft.as_ref(), &band_bins);
        let tilt_compensated: Vec<f32> =
            band_db.iter().enumerate().map(|(i, &db)| db + BAND_TILT_COMPENSATION_DB[i]).collect();
        let volume_offset = volume_tracker.offset(&tilt_compensated, frame_seconds);
        let targets: Vec<f32> = tilt_compensated.iter().map(|&db| level_from_range(db + volume_offset)).collect();
        let levels = smoother.smooth(&targets, frame_seconds);
        let _ = app.emit("waveform-levels", levels);
    }

    let _ = child.kill().await;
}

fn band_bin_ranges() -> Vec<(usize, usize)> {
    let bin_hz = SAMPLE_RATE as f32 / WINDOW_SIZE as f32;
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
