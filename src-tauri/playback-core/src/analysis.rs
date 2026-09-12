//! Spectrum analysis for the visualiser, from decoded PCM rather than captured audio.
//!
//! The old path captured system audio through a loopback device. This one taps the samples
//! Apogee itself is about to hand the output device, post-EQ and post-volume, so it
//! reflects what the app produces before any OS or hardware effect. No microphone or
//! system-audio permission is needed.
//!
//! Runs on a worker, never in the audio callback. Dropping analysis data when behind is
//! acceptable; blocking playback for a visualiser is not.

use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use std::sync::Arc;

/// FFT window. Matches the previous implementation so band resolution is unchanged.
pub const WINDOW_SIZE: usize = 1024;

/// Eight log-spaced bands, unchanged from `waveform.rs` so `Waveform.tsx` keeps working.
pub const BAND_EDGES_HZ: [f32; 9] = [
    20.0, 150.0, 400.0, 1000.0, 2000.0, 4000.0, 8000.0, 12000.0, 20000.0,
];

pub const BAND_COUNT: usize = BAND_EDGES_HZ.len() - 1;

/// Per-band additive dB offset.
///
/// Carried over from the capture implementation, but only the part that is genuinely
/// source-independent: real music has a steep spectral tilt, and log-spaced bands average
/// over very different numbers of linear FFT bins, so high bands read far quieter than low
/// ones for the same perceived energy. Neither cause has anything to do with how the audio
/// was obtained, so the shape stays. The absolute floor and ceiling below are a different
/// matter and are recalibrated for this tap.
pub const BAND_TILT_COMPENSATION_DB: [f32; BAND_COUNT] =
    [-18.0, -15.0, -5.0, -1.0, 0.0, 5.0, 10.0, 22.0];

/// Attack and release, in seconds. Fast attack so transients pop, slower release so bars
/// fall like a VU meter needle rather than snapping.
pub const ATTACK_SECONDS: f32 = 0.05;
pub const RELEASE_SECONDS: f32 = 0.3;

/// Fixed dB window mapped onto 0..1.
///
/// The capture path used -70..-35 dB, calibrated against parec output sitting downstream of
/// system volume. This tap is upstream of the OS mixer, so those numbers do not transfer.
///
/// Measured against decoded provider audio (`analysis_calibration`): per-band averages ran
/// -50.9 to -69.6 dB with an overall average of -61.4 dB, minima near -97 and maxima near
/// -37. This window puts that average near the middle of the display rather than pegged.
///
/// Calibrated on a short sample of one station, so it is a starting point that wants
/// checking by eye across several stations before release. The tests assert the average
/// lands mid-range, so drifting into all-pegged or all-floored fails rather than shipping.
pub const LEVEL_FLOOR_DB: f32 = -85.0;
pub const LEVEL_CEILING_DB: f32 = -40.0;

fn level_from_range(db: f32) -> f32 {
    ((db - LEVEL_FLOOR_DB) / (LEVEL_CEILING_DB - LEVEL_FLOOR_DB)).clamp(0.0, 1.0)
}

/// Asymmetric attack/release smoothing, applied to the displayed level.
#[derive(Debug)]
struct Smoother {
    levels: [f32; BAND_COUNT],
}

impl Smoother {
    fn new() -> Self {
        Self {
            levels: [0.0; BAND_COUNT],
        }
    }

    fn smooth(&mut self, targets: &[f32; BAND_COUNT], frame_seconds: f32) -> [f32; BAND_COUNT] {
        for (level, target) in self.levels.iter_mut().zip(targets.iter()) {
            let tau = if *target > *level {
                ATTACK_SECONDS
            } else {
                RELEASE_SECONDS
            };
            // One-pole smoothing; the coefficient is derived from the real elapsed time so
            // the feel does not change with block size or sample rate.
            let alpha = 1.0 - (-frame_seconds / tau).exp();
            *level += (*target - *level) * alpha.clamp(0.0, 1.0);
        }
        self.levels
    }
}

/// Accumulates PCM and produces smoothed band levels.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    sample_rate: f32,
    channels: usize,
    /// Mono-summed samples awaiting a full window.
    pending: Vec<f32>,
    window: Vec<f32>,
    scratch: Vec<Complex32>,
    band_bins: [(usize, usize); BAND_COUNT],
    smoother: Smoother,
}

impl SpectrumAnalyzer {
    #[must_use]
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(WINDOW_SIZE);
        let channels = channels.max(1);
        let sample_rate = sample_rate.max(1) as f32;

        // Hann window, precomputed.
        let window = (0..WINDOW_SIZE)
            .map(|n| {
                let x = std::f32::consts::PI * 2.0 * n as f32 / WINDOW_SIZE as f32;
                0.5 * (1.0 - x.cos())
            })
            .collect();

        let bin_hz = sample_rate / WINDOW_SIZE as f32;
        let mut band_bins = [(0usize, 0usize); BAND_COUNT];
        for (i, slot) in band_bins.iter_mut().enumerate() {
            let lo = (BAND_EDGES_HZ[i] / bin_hz).floor().max(1.0) as usize;
            let hi = (BAND_EDGES_HZ[i + 1] / bin_hz).ceil() as usize;
            // Never run past Nyquist; at low device rates the top bands collapse.
            let hi = hi.min(WINDOW_SIZE / 2);
            *slot = (lo.min(hi), hi);
        }

        Self {
            fft,
            sample_rate,
            channels,
            pending: Vec::with_capacity(WINDOW_SIZE),
            window,
            scratch: vec![Complex32::new(0.0, 0.0); WINDOW_SIZE],
            band_bins,
            smoother: Smoother::new(),
        }
    }

    /// Feed interleaved output samples. Returns levels each time a window completes.
    ///
    /// Channels are averaged to mono: the visualiser shows overall output, and a
    /// per-channel display would double the work for no visible benefit here.
    pub fn push(&mut self, interleaved: &[f32]) -> Option<[f32; BAND_COUNT]> {
        for frame in interleaved.chunks(self.channels) {
            let sum: f32 = frame.iter().sum();
            self.pending.push(sum / self.channels as f32);
        }
        if self.pending.len() < WINDOW_SIZE {
            return None;
        }

        // Analyse the most recent window and discard the rest: falling behind must drop
        // data rather than queue it, or the display would lag further and further.
        let start = self.pending.len() - WINDOW_SIZE;
        for (i, slot) in self.scratch.iter_mut().enumerate() {
            *slot = Complex32::new(self.pending[start + i] * self.window[i], 0.0);
        }
        self.pending.clear();

        self.fft.process(&mut self.scratch);

        let mut targets = [0.0f32; BAND_COUNT];
        for (band, &(lo, hi)) in self.band_bins.iter().enumerate() {
            if hi <= lo {
                targets[band] = 0.0;
                continue;
            }
            let mut sum = 0.0f32;
            for bin in lo..hi {
                let c = self.scratch[bin];
                // Normalise by window length so the result is independent of WINDOW_SIZE.
                let magnitude = (c.re * c.re + c.im * c.im).sqrt() / WINDOW_SIZE as f32;
                sum += magnitude * magnitude;
            }
            let mean_power = sum / (hi - lo) as f32;
            let db = if mean_power > 0.0 {
                10.0 * mean_power.log10() + BAND_TILT_COMPENSATION_DB[band]
            } else {
                f32::NEG_INFINITY
            };
            targets[band] = level_from_range(db);
        }

        let frame_seconds = WINDOW_SIZE as f32 / self.sample_rate;
        Some(self.smoother.smooth(&targets, frame_seconds))
    }

    /// Raw per-band dB before the floor/ceiling mapping. Used to calibrate the range
    /// against real decoded audio rather than guessing it.
    #[must_use]
    pub fn measure_band_db(&mut self, interleaved: &[f32]) -> Option<[f32; BAND_COUNT]> {
        for frame in interleaved.chunks(self.channels) {
            let sum: f32 = frame.iter().sum();
            self.pending.push(sum / self.channels as f32);
        }
        if self.pending.len() < WINDOW_SIZE {
            return None;
        }
        let start = self.pending.len() - WINDOW_SIZE;
        for (i, slot) in self.scratch.iter_mut().enumerate() {
            *slot = Complex32::new(self.pending[start + i] * self.window[i], 0.0);
        }
        self.pending.clear();
        self.fft.process(&mut self.scratch);

        let mut out = [f32::NEG_INFINITY; BAND_COUNT];
        for (band, &(lo, hi)) in self.band_bins.iter().enumerate() {
            if hi <= lo {
                continue;
            }
            let mut sum = 0.0f32;
            for bin in lo..hi {
                let c = self.scratch[bin];
                let magnitude = (c.re * c.re + c.im * c.im).sqrt() / WINDOW_SIZE as f32;
                sum += magnitude * magnitude;
            }
            let mean_power = sum / (hi - lo) as f32;
            if mean_power > 0.0 {
                out[band] = 10.0 * mean_power.log10() + BAND_TILT_COMPENSATION_DB[band];
            }
        }
        Some(out)
    }
}
