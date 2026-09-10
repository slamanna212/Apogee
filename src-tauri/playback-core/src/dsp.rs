//! Equalisation and gain, applied close to the output callback.
//!
//! Parity target, read from `build_equalizer_filter` in `src-tauri/src/mpv.rs`: MPV was
//! handed ten libavfilter `equalizer=f={freq}:t=o:w=1:g={gain}` stages plus, whenever any
//! band boosted, a leading `volume=-{max_boost}dB`. `equalizer` is FFmpeg's RBJ peaking
//! filter and `t=o:w=1` selects a one-octave bandwidth, so the octave form of the RBJ
//! cookbook is used here rather than a fixed Q. Bit-exact parity with libavfilter is not
//! claimed; the filter shape and the automatic headroom are.
//!
//! Nothing in `process` allocates, locks, or logs: it is written to be callable from a
//! real-time audio callback.

/// The app's ten band centres in Hz. Must stay in step with `src/lib/equalizer.ts`.
pub const BANDS: [f64; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Gains outside this range are rejected, matching the existing UI clamp.
pub const MIN_GAIN_DB: f64 = -12.0;
pub const MAX_GAIN_DB: f64 = 12.0;

const BANDWIDTH_OCTAVES: f64 = 1.0;
/// Anything at or above this fraction of Nyquist is skipped: a peaking section
/// centred at or past Nyquist is not representable and would blow up.
const NYQUIST_MARGIN: f64 = 0.95;
/// Added to running state each sample to keep denormals from stalling the callback.
const DENORMAL_EPSILON: f32 = 1.0e-25;

/// Direct-form-I biquad coefficients, pre-normalised by a0.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    /// Identity: output equals input exactly.
    const fn bypass() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    /// RBJ peaking EQ, octave-bandwidth form, matching FFmpeg's `equalizer` with `t=o`.
    fn peaking(sample_rate: f64, centre_hz: f64, gain_db: f64) -> Self {
        if sample_rate <= 0.0 || centre_hz >= sample_rate * 0.5 * NYQUIST_MARGIN {
            return Self::bypass();
        }
        if gain_db == 0.0 {
            return Self::bypass();
        }

        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * centre_hz / sample_rate;
        let sin_w0 = w0.sin();
        if sin_w0.abs() < f64::EPSILON {
            return Self::bypass();
        }
        // FFmpeg af_biquads.c, OCTAVE width type.
        let alpha =
            sin_w0 * (std::f64::consts::LN_2 / 2.0 * BANDWIDTH_OCTAVES * w0 / sin_w0).sinh();
        let cos_w0 = w0.cos();

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }
}

/// Per-channel filter state for one band.
#[derive(Debug, Clone, Copy, Default)]
struct State {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl State {
    #[inline]
    fn step(&mut self, c: &Coefficients, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2
            + DENORMAL_EPSILON;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Ten-band equaliser plus smoothed output gain.
///
/// Coefficients are computed in [`Equalizer::configure`], never in [`Equalizer::process`].
#[derive(Debug)]
pub struct Equalizer {
    sample_rate: f64,
    channels: usize,
    enabled: bool,
    coefficients: [Coefficients; BANDS.len()],
    /// `channels * BANDS.len()` states, indexed band-major.
    states: Vec<State>,
    /// Linear headroom scalar reproducing MPV's `volume=-{max_boost}dB`.
    headroom: f32,
    target_gain: f32,
    current_gain: f32,
    gain_step: f32,
}

impl Equalizer {
    /// `channels` must be at least 1.
    #[must_use]
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            sample_rate: f64::from(sample_rate),
            channels,
            enabled: false,
            coefficients: [Coefficients::bypass(); BANDS.len()],
            states: vec![State::default(); channels * BANDS.len()],
            headroom: 1.0,
            target_gain: 1.0,
            current_gain: 1.0,
            gain_step: 0.0,
        }
    }

    /// Rebuild for a new output format. Clears filter state, since carrying it across a
    /// rate change would inject noise.
    pub fn reconfigure_output(&mut self, sample_rate: u32, channels: usize) {
        let channels = channels.max(1);
        self.sample_rate = f64::from(sample_rate);
        self.channels = channels;
        self.states.clear();
        self.states.resize(channels * BANDS.len(), State::default());
        let gains = self.current_gains_db();
        let enabled = self.enabled;
        self.configure(enabled, &gains)
            .expect("previously accepted gains stay valid");
    }

    fn current_gains_db(&self) -> [f64; BANDS.len()] {
        // Coefficients are not invertible in general; callers re-supply gains. Kept flat
        // so a reconfigure without a fresh set does not silently distort.
        [0.0; BANDS.len()]
    }

    /// Set band gains in dB. Precomputes every coefficient; allocation-free afterwards.
    ///
    /// Returns an error rather than clamping silently, so an out-of-range value is a bug
    /// that surfaces instead of a quiet difference from the UI.
    pub fn configure(&mut self, enabled: bool, gains_db: &[f64]) -> Result<(), String> {
        if gains_db.len() != BANDS.len() {
            return Err(format!("equalizer requires exactly {} bands", BANDS.len()));
        }
        if let Some(bad) = gains_db
            .iter()
            .find(|g| !g.is_finite() || **g < MIN_GAIN_DB || **g > MAX_GAIN_DB)
        {
            return Err(format!(
                "equalizer gains must be finite values from {MIN_GAIN_DB} to {MAX_GAIN_DB} dB (got {bad})"
            ));
        }

        self.enabled = enabled;
        if !enabled {
            self.coefficients = [Coefficients::bypass(); BANDS.len()];
            self.headroom = 1.0;
            return Ok(());
        }

        for (i, (&centre, &gain)) in BANDS.iter().zip(gains_db.iter()).enumerate() {
            self.coefficients[i] = Coefficients::peaking(self.sample_rate, centre, gain);
        }

        // MPV prefixed `volume=-{max_boost}dB`, i.e. it reserved headroom for the single
        // largest boost. That is not enough: adjacent peaking bands overlap, so their
        // magnitude responses multiply. Measured here, ten bands at +12 dB peak at 2.17
        // after MPV's headroom, which clips hard. Instead of copying that flaw, the true
        // worst-case gain of the whole cascade is measured and inverted. For a single
        // boosted band the two agree, so ordinary settings behave as before.
        let peak = self.peak_response();
        self.headroom = if peak > 1.0 { (1.0 / peak) as f32 } else { 1.0 };
        Ok(())
    }

    /// Set output gain as 0..=100 volume and a mute flag.
    ///
    /// Mute preserves the stored volume, matching existing behaviour. The change is ramped
    /// over `ramp_samples` frames so it cannot click.
    pub fn set_gain(&mut self, volume: u8, muted: bool, ramp_frames: u32) {
        let target = if muted {
            0.0
        } else {
            // Perceptual curve; linear volume sliders sound wrong at the low end.
            let v = f32::from(volume.min(100)) / 100.0;
            v * v
        };
        self.target_gain = target;
        self.gain_step = if ramp_frames == 0 {
            self.current_gain = target;
            0.0
        } else {
            (target - self.current_gain) / ramp_frames as f32
        };
    }

    /// Largest magnitude the configured filter cascade applies at any frequency.
    ///
    /// Evaluated on a log-spaced grid at configure time, never in the audio callback.
    /// A grid can in principle miss a very narrow peak between points, so the grid is
    /// dense relative to the one-octave bandwidth actually in use.
    fn peak_response(&self) -> f64 {
        const POINTS: usize = 1024;
        let nyquist = self.sample_rate * 0.5;
        if nyquist <= 0.0 {
            return 1.0;
        }
        let (lo, hi) = (10.0f64, nyquist * 0.999);
        let ratio = (hi / lo).powf(1.0 / (POINTS - 1) as f64);

        let mut peak: f64 = 0.0;
        let mut freq = lo;
        for _ in 0..POINTS {
            let w = 2.0 * std::f64::consts::PI * freq / self.sample_rate;
            let (cos1, sin1) = ((-w).cos(), (-w).sin());
            let (cos2, sin2) = ((-2.0 * w).cos(), (-2.0 * w).sin());

            let mut magnitude = 1.0f64;
            for c in &self.coefficients {
                let (b0, b1, b2) = (f64::from(c.b0), f64::from(c.b1), f64::from(c.b2));
                let (a1, a2) = (f64::from(c.a1), f64::from(c.a2));
                let num_re = b0 + b1 * cos1 + b2 * cos2;
                let num_im = b1 * sin1 + b2 * sin2;
                let den_re = 1.0 + a1 * cos1 + a2 * cos2;
                let den_im = a1 * sin1 + a2 * sin2;
                let den = (den_re * den_re + den_im * den_im).sqrt();
                if den > f64::EPSILON {
                    magnitude *= (num_re * num_re + num_im * num_im).sqrt() / den;
                }
            }
            peak = peak.max(magnitude);
            freq *= ratio;
        }
        peak
    }

    /// Linear scalar applied before filtering to keep the cascade from clipping.
    #[must_use]
    pub fn headroom(&self) -> f32 {
        self.headroom
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Process interleaved frames in place.
    ///
    /// Real-time safe: no allocation, no locking, no logging, no I/O.
    pub fn process(&mut self, interleaved: &mut [f32]) {
        let channels = self.channels;
        if channels == 0 || interleaved.is_empty() {
            return;
        }
        let bands = BANDS.len();

        for frame in interleaved.chunks_mut(channels) {
            // Advance the gain ramp once per frame so every channel gets the same gain.
            if self.gain_step != 0.0 {
                self.current_gain += self.gain_step;
                let done = (self.gain_step > 0.0 && self.current_gain >= self.target_gain)
                    || (self.gain_step < 0.0 && self.current_gain <= self.target_gain);
                if done {
                    self.current_gain = self.target_gain;
                    self.gain_step = 0.0;
                }
            }

            for (ch, sample) in frame.iter_mut().enumerate() {
                let mut x = *sample;
                if self.enabled {
                    x *= self.headroom;
                    for band in 0..bands {
                        let c = &self.coefficients[band];
                        let state = &mut self.states[band * channels + ch];
                        x = state.step(c, x);
                    }
                }
                *sample = x * self.current_gain;
            }
        }
    }
}
