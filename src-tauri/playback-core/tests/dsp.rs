//! Measures what the equaliser actually does to signals, rather than asserting it ran.

use apogee_playback_core::dsp::{BANDS, Equalizer};

const RATE: u32 = 44_100;

fn sine(freq: f64, frames: usize, channels: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        let v = (2.0 * std::f64::consts::PI * freq * n as f64 / f64::from(RATE)).sin() as f32;
        for _ in 0..channels {
            out.push(v);
        }
    }
    out
}

/// RMS of the second half only, so filter start-up transients are excluded.
fn settled_rms(buf: &[f32], channels: usize) -> f64 {
    let start = (buf.len() / channels / 2) * channels;
    let tail = &buf[start..];
    let sum: f64 = tail.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / tail.len() as f64).sqrt()
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

/// Gain in dB the equaliser applies to a steady tone at `freq`.
fn response_db(eq: &mut Equalizer, freq: f64, channels: usize) -> f64 {
    let input = sine(freq, 16_384, channels);
    let reference = settled_rms(&input, channels);
    let mut buf = input;
    eq.process(&mut buf);
    db(settled_rms(&buf, channels) / reference)
}

#[test]
fn a_disabled_equalizer_is_bit_exact_unity() {
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(false, &[0.0; 10]).unwrap();
    eq.set_gain(100, false, 0);

    let input = sine(1000.0, 4096, 2);
    let mut buf = input.clone();
    eq.process(&mut buf);
    assert_eq!(buf, input, "bypass must not alter samples at all");
}

#[test]
fn an_enabled_but_flat_equalizer_does_not_change_level() {
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(true, &[0.0; 10]).unwrap();
    eq.set_gain(100, false, 0);

    for freq in [100.0, 1000.0, 8000.0] {
        let r = response_db(&mut eq, freq, 2);
        assert!(r.abs() < 0.01, "flat EQ shifted {freq} Hz by {r:.4} dB");
    }
}

#[test]
fn boosting_one_band_raises_that_band_relative_to_the_others() {
    let mut gains = [0.0; 10];
    let idx = BANDS.iter().position(|b| *b == 1000.0).unwrap();
    gains[idx] = 12.0;

    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(true, &gains).unwrap();
    eq.set_gain(100, false, 0);

    let at_band = response_db(&mut eq, 1000.0, 2);
    let far_away = response_db(&mut eq, 100.0, 2);

    // Absolute levels are shifted down by the automatic headroom, so compare the
    // difference, which is the actual filter shape.
    let separation = at_band - far_away;
    assert!(
        (10.0..=14.0).contains(&separation),
        "expected roughly 12 dB of separation, got {separation:.2} dB \
         (band {at_band:.2} dB, reference {far_away:.2} dB)"
    );
}

#[test]
fn automatic_headroom_keeps_a_full_scale_boost_from_clipping() {
    // Every band boosted to the maximum is the worst case for clipping.
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(true, &[12.0; 10]).unwrap();
    eq.set_gain(100, false, 0);

    assert!(eq.headroom() < 1.0, "a boost must engage headroom");

    let mut buf = sine(1000.0, 8192, 2);
    eq.process(&mut buf);
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak <= 1.0,
        "full-scale input peaked at {peak}, which would clip"
    );
    assert!(buf.iter().all(|s| s.is_finite()), "output must stay finite");
}

#[test]
fn no_headroom_is_applied_when_nothing_boosts() {
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(true, &[-6.0; 10]).unwrap();
    assert_eq!(eq.headroom(), 1.0, "cuts alone must not attenuate further");
}

#[test]
fn bands_at_or_above_nyquist_are_skipped_rather_than_exploding() {
    // At 8 kHz the 4k, 8k and 16k band centres are at or beyond Nyquist.
    let mut eq = Equalizer::new(8_000, 2);
    eq.configure(true, &[12.0; 10]).unwrap();
    eq.set_gain(100, false, 0);

    let mut buf = sine(500.0, 8192, 2);
    eq.process(&mut buf);
    assert!(
        buf.iter().all(|s| s.is_finite()),
        "low-rate output must stay finite"
    );
    assert!(
        buf.iter().any(|s| *s != 0.0),
        "output must not collapse to silence"
    );
}

#[test]
fn gain_changes_ramp_instead_of_stepping() {
    let mut eq = Equalizer::new(RATE, 1);
    eq.configure(false, &[0.0; 10]).unwrap();
    eq.set_gain(100, false, 0);

    // Constant input makes any discontinuity in the output a gain discontinuity.
    let mut buf = vec![1.0f32; 1024];
    eq.set_gain(0, false, 512);
    eq.process(&mut buf);

    let max_step = buf
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_step < 0.01,
        "gain moved by {max_step} in one sample, which would click"
    );
    assert!(
        buf.last().unwrap().abs() < 1e-6,
        "ramp should have reached silence"
    );
}

#[test]
fn mute_silences_output_and_unmute_restores_the_stored_volume() {
    let mut eq = Equalizer::new(RATE, 1);
    eq.configure(false, &[0.0; 10]).unwrap();

    eq.set_gain(80, false, 0);
    let mut before = vec![1.0f32; 64];
    eq.process(&mut before);
    let level = before[63];
    assert!(level > 0.0);

    eq.set_gain(80, true, 0);
    let mut muted = vec![1.0f32; 64];
    eq.process(&mut muted);
    assert!(
        muted.iter().all(|s| s.abs() < 1e-9),
        "mute must silence output"
    );

    eq.set_gain(80, false, 0);
    let mut after = vec![1.0f32; 64];
    eq.process(&mut after);
    assert!(
        (after[63] - level).abs() < 1e-6,
        "unmute must restore the same volume: {} vs {}",
        after[63],
        level
    );
}

#[test]
fn out_of_range_or_non_finite_gains_are_rejected_not_clamped() {
    let mut eq = Equalizer::new(RATE, 2);
    let mut gains = [0.0; 10];
    gains[0] = 13.0;
    assert!(
        eq.configure(true, &gains).is_err(),
        "above +12 dB must be rejected"
    );
    gains[0] = -13.0;
    assert!(
        eq.configure(true, &gains).is_err(),
        "below -12 dB must be rejected"
    );
    gains[0] = f64::NAN;
    assert!(eq.configure(true, &gains).is_err(), "NaN must be rejected");
    assert!(
        eq.configure(true, &[0.0; 9]).is_err(),
        "wrong band count must be rejected"
    );
}

#[test]
fn channels_are_filtered_independently() {
    let mut gains = [0.0; 10];
    gains[BANDS.iter().position(|b| *b == 1000.0).unwrap()] = 12.0;
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(true, &gains).unwrap();
    eq.set_gain(100, false, 0);

    // Left carries signal, right is silent. Cross-talk would leak into the right.
    let mut buf = Vec::new();
    for n in 0..8192 {
        let v = (2.0 * std::f64::consts::PI * 1000.0 * f64::from(n) / f64::from(RATE)).sin() as f32;
        buf.push(v);
        buf.push(0.0);
    }
    eq.process(&mut buf);

    let right_peak = buf
        .iter()
        .skip(1)
        .step_by(2)
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        right_peak < 1e-6,
        "silent channel picked up {right_peak}, so state is shared"
    );
}

#[test]
fn processing_is_stable_over_a_long_run() {
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(
        true,
        &[
            12.0, -12.0, 12.0, -12.0, 12.0, -12.0, 12.0, -12.0, 12.0, -12.0,
        ],
    )
    .unwrap();
    eq.set_gain(100, false, 0);

    // Roughly 20 seconds of audio in blocks, checking for drift or blow-up.
    for block in 0..500 {
        let mut buf = sine(440.0, 1764, 2);
        eq.process(&mut buf);
        assert!(
            buf.iter().all(|s| s.is_finite() && s.abs() <= 4.0),
            "block {block} produced an out-of-range or non-finite sample"
        );
    }
}

#[test]
fn single_band_headroom_still_matches_mpvs_behaviour() {
    // MPV reserved exactly the largest boost. For one isolated band the measured
    // worst-case cascade gain is that same boost, so ordinary settings are unchanged.
    for boost_db in [3.0, 6.0, 12.0] {
        let mut gains = [0.0; 10];
        gains[BANDS.iter().position(|b| *b == 1000.0).unwrap()] = boost_db;

        let mut eq = Equalizer::new(RATE, 2);
        eq.configure(true, &gains).unwrap();

        let mpv_headroom = 10f64.powf(-boost_db / 20.0);
        let actual = f64::from(eq.headroom());
        assert!(
            (actual - mpv_headroom).abs() < 0.01,
            "{boost_db} dB boost: headroom {actual:.4} vs MPV's {mpv_headroom:.4}"
        );
    }
}

#[test]
fn overlapping_boosts_reserve_more_headroom_than_mpv_did() {
    // The case that clipped under MPV's scheme.
    let mut eq = Equalizer::new(RATE, 2);
    eq.configure(true, &[12.0; 10]).unwrap();
    let mpv_headroom = 10f64.powf(-12.0 / 20.0);
    assert!(
        f64::from(eq.headroom()) < mpv_headroom,
        "overlapping bands must reserve strictly more headroom than the single-band rule"
    );
}
