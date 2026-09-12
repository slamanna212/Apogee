//! Calibrates and verifies the visualiser against real decoded audio.
//!
//! The previous implementation's dB range was calibrated against captured system audio.
//! This tap sits earlier in the chain, so the range had to be re-measured rather than
//! copied. The measurement test prints the numbers the constants are derived from.

use apogee_playback_core::analysis::{
    BAND_COUNT, LEVEL_CEILING_DB, LEVEL_FLOOR_DB, SpectrumAnalyzer,
};
use apogee_playback_core::pipeline::{Pipeline, TsIngest};

fn decode_fixture() -> (Vec<f32>, u32, usize) {
    let mut pcm = Vec::new();
    let mut ingest = TsIngest::new();
    let mut pipeline = Pipeline::new();
    let (mut rate, mut channels) = (44_100u32, 2usize);

    for name in [
        "aac-0.ts", "aac-1.ts", "aac-2.ts", "aac-3.ts", "aac-4.ts", "aac-5.ts",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        ingest.feed(&std::fs::read(path).unwrap());
    }
    ingest.finish();
    while let Some(event) = ingest.poll() {
        if let Ok(Some(block)) = pipeline.accept(event) {
            rate = block.rate;
            channels = block.channels;
            pcm.extend(block.samples);
        }
    }
    (pcm, rate, channels)
}

#[test]
fn measured_band_levels_fall_inside_the_configured_range() {
    let (pcm, rate, channels) = decode_fixture();
    assert!(!pcm.is_empty(), "fixture must decode");

    let mut analyzer = SpectrumAnalyzer::new(rate, channels);
    let mut mins = [f32::INFINITY; BAND_COUNT];
    let mut maxes = [f32::NEG_INFINITY; BAND_COUNT];
    let mut sums = [0.0f64; BAND_COUNT];
    let mut windows = 0usize;

    for block in pcm.chunks(1024 * channels) {
        if let Some(db) = analyzer.measure_band_db(block) {
            windows += 1;
            for b in 0..BAND_COUNT {
                if db[b].is_finite() {
                    mins[b] = mins[b].min(db[b]);
                    maxes[b] = maxes[b].max(db[b]);
                    sums[b] += f64::from(db[b]);
                }
            }
        }
    }

    assert!(
        windows > 20,
        "need enough windows to be meaningful, got {windows}"
    );
    println!("windows analysed: {windows}");
    println!("band |    min |    avg |    max   (dB, tilt-compensated)");
    for b in 0..BAND_COUNT {
        println!(
            "  {b}  | {:6.1} | {:6.1} | {:6.1}",
            mins[b],
            sums[b] / windows as f64,
            maxes[b]
        );
    }

    // The configured window must actually bracket real music, or every bar pegs or floors.
    let overall_avg: f64 = sums.iter().sum::<f64>() / (windows * BAND_COUNT) as f64;
    println!("overall average: {overall_avg:.1} dB");

    // Not merely inside the window: near its middle. A range that technically brackets the
    // signal but sits it at 0.95 gives a display of permanently full bars.
    let floor = f64::from(LEVEL_FLOOR_DB);
    let ceiling = f64::from(LEVEL_CEILING_DB);
    let normalised = (overall_avg - floor) / (ceiling - floor);
    println!("average maps to {normalised:.2} of full scale");
    assert!(
        (0.3..=0.7).contains(&normalised),
        "average band level {overall_avg:.1} dB maps to {normalised:.2} of full scale, \
         which would look {} rather than lively",
        if normalised > 0.7 {
            "permanently pegged"
        } else {
            "permanently dark"
        }
    );
}

#[test]
fn real_music_produces_varied_bars_rather_than_all_pegged_or_all_floored() {
    let (pcm, rate, channels) = decode_fixture();
    let mut analyzer = SpectrumAnalyzer::new(rate, channels);

    let mut last = [0.0f32; BAND_COUNT];
    let mut saw_any = false;
    for block in pcm.chunks(1024 * channels) {
        if let Some(levels) = analyzer.push(block) {
            last = levels;
            saw_any = true;
            assert!(
                levels
                    .iter()
                    .all(|l| (0.0..=1.0).contains(l) && l.is_finite()),
                "levels must stay in 0..1: {levels:?}"
            );
        }
    }
    assert!(
        saw_any,
        "should have produced at least one window of levels"
    );

    println!("final levels: {last:?}");
    let pegged = last.iter().filter(|l| **l >= 0.98).count();
    let floored = last.iter().filter(|l| **l <= 0.02).count();
    assert!(
        pegged < BAND_COUNT / 2,
        "most bands pegged near maximum: {last:?}"
    );
    assert!(
        floored < BAND_COUNT / 2,
        "most bands sat near zero: {last:?}"
    );

    // Bars should differ from each other; a flat wall of identical bars is not a spectrum.
    let spread =
        last.iter().cloned().fold(0.0f32, f32::max) - last.iter().cloned().fold(1.0f32, f32::min);
    assert!(
        spread > 0.1,
        "bands are nearly identical, spread {spread:.3}: {last:?}"
    );
}

#[test]
fn digital_silence_produces_zero_levels_without_non_finite_values() {
    let mut analyzer = SpectrumAnalyzer::new(44_100, 2);
    let silence = vec![0.0f32; 1024 * 2 * 8];
    let mut produced = false;
    for block in silence.chunks(1024 * 2) {
        if let Some(levels) = analyzer.push(block) {
            produced = true;
            assert!(
                levels.iter().all(|l| l.is_finite()),
                "silence must not produce NaN"
            );
            assert!(
                levels.iter().all(|l| *l <= 0.001),
                "silence should read as zero, got {levels:?}"
            );
        }
    }
    assert!(produced);
}

#[test]
fn levels_release_gradually_rather_than_snapping_to_zero() {
    let mut analyzer = SpectrumAnalyzer::new(44_100, 2);

    // Drive it with a loud tone, then cut to silence.
    let mut loud = Vec::new();
    for n in 0..1024 * 8 {
        let v = (2.0 * std::f32::consts::PI * 500.0 * n as f32 / 44_100.0).sin() * 0.8;
        loud.push(v);
        loud.push(v);
    }
    let mut peak = [0.0f32; BAND_COUNT];
    for block in loud.chunks(1024 * 2) {
        if let Some(levels) = analyzer.push(block) {
            peak = levels;
        }
    }
    assert!(
        peak.iter().any(|l| *l > 0.1),
        "a loud tone should raise some band: {peak:?}"
    );

    // One window of silence must not collapse the bar instantly.
    let silence = vec![0.0f32; 1024 * 2];
    let after = analyzer.push(&silence).expect("one window");
    let loudest = peak.iter().cloned().fold(0.0f32, f32::max);
    let after_loudest = after.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        after_loudest > loudest * 0.3,
        "release too fast: {loudest:.3} -> {after_loudest:.3} in one window"
    );
}

#[test]
fn a_low_rate_device_does_not_produce_garbage_in_the_top_bands() {
    // At 8 kHz, Nyquist is 4 kHz, so the top three band edges are unreachable.
    let mut analyzer = SpectrumAnalyzer::new(8_000, 2);
    let mut pcm = Vec::new();
    for n in 0..1024 * 4 {
        let v = (2.0 * std::f32::consts::PI * 400.0 * n as f32 / 8_000.0).sin() * 0.5;
        pcm.push(v);
        pcm.push(v);
    }
    let mut produced = false;
    for block in pcm.chunks(1024 * 2) {
        if let Some(levels) = analyzer.push(block) {
            produced = true;
            assert!(
                levels
                    .iter()
                    .all(|l| l.is_finite() && (0.0..=1.0).contains(l))
            );
        }
    }
    assert!(produced);
}
