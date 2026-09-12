//! Proves the plan's central invariant: continuous TS and segmented HLS carrying the
//! same audio converge on identical PCM, with exactly one demux stage each and no
//! decoder reset at ordinary segment boundaries.

use apogee_playback_core::pipeline::{HlsIngest, Pipeline, SourceEvent, TsIngest};
use hls_runtime::client::{Action, HlsClient};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Decode a continuous TS body fed in `chunk_size` pieces.
fn direct(bytes: &[u8], chunk_size: usize) -> (Vec<f32>, Pipeline) {
    let mut ingest = TsIngest::new();
    let mut pipeline = Pipeline::new();
    let mut pcm = Vec::new();

    for chunk in bytes.chunks(chunk_size).chain(std::iter::once(&[][..])) {
        if chunk.is_empty() {
            ingest.finish();
        } else {
            ingest.feed(chunk);
        }
        while let Some(event) = ingest.poll() {
            if let Some(block) = pipeline.accept(event).unwrap() {
                assert_eq!((block.rate, block.channels), (44100, 2));
                pcm.extend(block.samples);
            }
        }
    }
    (pcm, pipeline)
}

/// Drive the HLS client against local fixtures with no network, socket, or credentials.
fn via_hls(playlist: &[u8]) -> (Vec<f32>, Pipeline, HlsIngest) {
    let mut client = HlsClient::new("https://example.invalid/aac.m3u8");
    let mut ingest = HlsIngest::new();
    let mut pipeline = Pipeline::new();
    let mut pcm = Vec::new();
    let mut ended = false;

    for _ in 0..200 {
        if let Some(action) = client.poll() {
            match action {
                Action::FetchPlaylist { .. } => client.on_playlist(playlist).unwrap(),
                Action::FetchResource {
                    id,
                    url,
                    byte_range,
                } => {
                    assert!(byte_range.is_none(), "fixtures use whole segments");
                    let name = url.rsplit('/').next().unwrap();
                    client.on_resource(id, &fixture(name)).unwrap();
                }
                Action::WaitMs(_) => {}
                other => panic!("unhandled action {other:?}"),
            }
        }
        while let Some(output) = client.next_output() {
            for event in ingest.translate(output).unwrap() {
                if matches!(event, SourceEvent::EndOfStream) {
                    ended = true;
                }
                if let Some(block) = pipeline.accept(event).unwrap() {
                    pcm.extend(block.samples);
                }
            }
        }
        if ended {
            break;
        }
    }
    assert!(ended, "fixture playlist has an ENDLIST and must terminate");
    (pcm, pipeline, ingest)
}

#[test]
fn continuous_ts_and_segmented_hls_decode_to_identical_pcm() {
    let playlist = fixture("aac.m3u8");
    let text = std::str::from_utf8(&playlist).unwrap();
    let segments: Vec<_> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert!(
        segments.len() > 1,
        "parity is only meaningful across several segments"
    );

    let continuous: Vec<u8> = segments.iter().flat_map(|s| fixture(s)).collect();
    // A 137-byte stride is deliberately coprime with the 188-byte TS packet size,
    // so nearly every chunk boundary falls mid-packet.
    let (expected, direct_pipeline) = direct(&continuous, 137);

    assert!(
        expected.len() > 44100 * 2,
        "need more than a second of audio to be meaningful"
    );
    assert!(
        expected.iter().all(|s| s.is_finite()),
        "decoded PCM must be finite"
    );
    assert!(
        expected.iter().any(|s| s.abs() > 0.01),
        "fixture must not be silence"
    );

    let (actual, hls_pipeline, ingest) = via_hls(&playlist);

    assert_eq!(
        expected.len(),
        actual.len(),
        "decoded sample count must match"
    );
    let mismatch = expected.iter().zip(&actual).position(|(a, b)| a != b);
    assert!(
        mismatch.is_none(),
        "first differing sample at index {mismatch:?}"
    );

    // Demux exactly once, and no gratuitous decoder churn.
    assert_eq!(
        ingest.inits(),
        1,
        "one initialization section for the whole playlist"
    );
    assert_eq!(
        hls_pipeline.resets(),
        0,
        "normal segment boundaries must not reset the decoder"
    );
    assert_eq!(
        direct_pipeline.resets(),
        0,
        "continuous TS must not reset the decoder"
    );
    assert_eq!(
        direct_pipeline.decoded_frames(),
        hls_pipeline.decoded_frames(),
        "both paths must decode the same number of frames"
    );
}

#[test]
fn compressed_bitrate_reflects_audio_not_transport_overhead() {
    let playlist = fixture("aac.m3u8");
    let text = std::str::from_utf8(&playlist).unwrap();
    let segments: Vec<_> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let continuous: Vec<u8> = segments.iter().flat_map(|s| fixture(s)).collect();
    let (_, pipeline) = direct(&continuous, 4096);

    let kbps = pipeline
        .bitrate_kbps(44100)
        .expect("enough audio decoded for an estimate");
    // The station is ~258 kbps. TS packetisation adds roughly 6% overhead, so a
    // transport-derived figure would land well above this range.
    assert!(
        (230..=275).contains(&kbps),
        "bitrate {kbps} kbps outside expected AAC range"
    );
    assert!(
        (pipeline.compressed_bytes() as usize) < continuous.len(),
        "audio payload must be smaller than the transport stream carrying it"
    );
}

#[test]
fn bitrate_is_unknown_until_there_is_enough_evidence() {
    let pipeline = Pipeline::new();
    assert_eq!(
        pipeline.bitrate_kbps(44100),
        None,
        "no evidence yet means no guess"
    );
}

#[test]
fn mp3_ts_decodes_independently_of_network_chunk_boundaries() {
    let bytes = fixture("mp3.ts");
    let (expected, _) = direct(&bytes, bytes.len());
    assert!(expected.len() >= 44100 * 2);
    for size in [1, 187, 188, 1024] {
        let (actual, pipeline) = direct(&bytes, size);
        assert_eq!(
            expected, actual,
            "chunk size {size} changed the decoded audio"
        );
        assert_eq!(pipeline.resets(), 0);
    }
}

// ---------------------------------------------------------------------------------------
// Finding 10: bitrate accounting must not mix frames counted under two different rates
// into one seconds-estimate across a genuine mid-stream format change.
// ---------------------------------------------------------------------------------------

#[test]
fn bitrate_accounting_resets_across_a_genuine_format_change() {
    let mut pipeline = Pipeline::new();

    // First format: 44100 Hz stereo MP3 (the existing `mp3.ts` fixture).
    let mut ingest_a = TsIngest::new();
    ingest_a.feed(&fixture("mp3.ts"));
    ingest_a.finish();
    while let Some(event) = ingest_a.poll() {
        pipeline.accept(event).unwrap();
    }
    assert!(
        pipeline.bitrate_kbps(44_100).is_some(),
        "should already have enough evidence from the first format"
    );

    // Second format, decoded into the SAME pipeline: 22050 Hz mono MP3 at ~32 kbps (a
    // fresh fixture generated specifically at a different rate, channel count AND
    // bitrate, since the repository's other fixtures all happen to share one format).
    // This models a live mid-stream re-encode or ABR switch landing a new
    // `SourceEvent::Track` on an already-running pipeline.
    //
    // The bitrate is deliberately NOT simply half of the first format's ~128 kbps: at
    // exactly half, contaminating the accounting with the old (44100 Hz, 128 kbps)
    // frames exactly cancels out when mis-divided by the new 22050 Hz rate (mis-dividing
    // doubles the apparent old duration, and doubling both the numerator's contribution
    // and the denominator by the same old/new rate ratio only leaves the result
    // unchanged when the bitrate ratio matches the rate ratio) - which would make this
    // test pass whether or not the reset in `Pipeline::accept` actually runs. A 4x
    // bitrate ratio against a 2x rate ratio has no such cancellation.
    let mut ingest_b = TsIngest::new();
    ingest_b.feed(&fixture("mp3-22050-mono.ts"));
    ingest_b.finish();
    while let Some(event) = ingest_b.poll() {
        pipeline.accept(event).unwrap();
    }

    // If frames decoded under the old 44100 Hz format were still mixed into the running
    // total, dividing the combined frame count by the NEW 22050 Hz rate would inflate the
    // derived duration and push the apparent bitrate well above the genuine ~32 kbps
    // content (see the arithmetic above) - a contaminated value lands far outside this
    // window.
    let kbps_after = pipeline
        .bitrate_kbps(22_050)
        .expect("enough evidence from the new-format fixture alone");
    assert!(
        (24..=42).contains(&kbps_after),
        "bitrate {kbps_after} kbps looks contaminated by mixed-rate accounting carried \
         over from before the format change"
    );
}

#[test]
fn samples_arriving_before_a_track_configuration_are_a_clear_error() {
    use apogee_playback_core::pipeline::PipelineError;

    // Take a genuine access unit from a fixture rather than synthesising one, so the
    // error path is exercised with input the decoder would otherwise accept.
    let mut ingest = TsIngest::new();
    ingest.feed(&fixture("mp3.ts"));
    ingest.finish();
    let mut real_access = None;
    while let Some(event) = ingest.poll() {
        if let SourceEvent::Access(sample) = event {
            real_access = Some(sample);
            break;
        }
    }
    let sample = real_access.expect("fixture yields at least one access unit");

    let mut pipeline = Pipeline::new();
    assert!(!pipeline.is_configured());
    assert_eq!(
        pipeline.accept(SourceEvent::Access(sample)).unwrap_err(),
        PipelineError::SamplesBeforeTrack,
        "audio before a track declaration must be a clear error, not a panic or silence"
    );
}
