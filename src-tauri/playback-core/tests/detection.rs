use apogee_playback_core::detect::{Detection, Detector, SourceKind, Unsupported, select_variant};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn detect_all(bytes: &[u8], content_type: Option<&str>) -> Detection {
    let mut d = Detector::new();
    d.push(bytes);
    d.detect(content_type)
}

#[test]
fn raw_ts_is_identified_even_when_served_as_a_playlist_mime() {
    // The target provider returns raw MPEG-TS from `.m3u8` URLs with an HLS MIME.
    // Bytes must win over both the extension and the declared type.
    let d = detect_all(&fixture("aac-0.ts"), Some("application/vnd.apple.mpegurl"));
    assert!(
        matches!(d, Detection::Identified(SourceKind::MpegTs { stride: 188 })),
        "expected MpegTs, got {d:?}"
    );
}

#[test]
fn playlist_is_identified_regardless_of_extension_or_mime() {
    let d = detect_all(&fixture("aac.m3u8"), Some("video/mp2t"));
    assert_eq!(d, Detection::Identified(SourceKind::HlsMediaPlaylist));
}

#[test]
fn playlist_survives_utf8_bom_and_leading_whitespace() {
    let mut body = vec![0xEF, 0xBB, 0xBF];
    body.extend_from_slice(b"\r\n  \n");
    body.extend_from_slice(&fixture("aac.m3u8"));
    assert_eq!(
        detect_all(&body, None),
        Detection::Identified(SourceKind::HlsMediaPlaylist)
    );
}

#[test]
fn detection_is_stable_across_arbitrary_chunk_boundaries() {
    let bytes = fixture("aac-0.ts");
    for chunk in [1usize, 7, 188, 189, 1024] {
        let mut d = Detector::new();
        let mut result = None;
        for c in bytes.chunks(chunk) {
            d.push(c);
            match d.detect(None) {
                Detection::NeedMoreData { .. } => continue,
                other => {
                    result = Some(other);
                    break;
                }
            }
        }
        assert!(
            matches!(
                result,
                Some(Detection::Identified(SourceKind::MpegTs { .. }))
            ),
            "chunk size {chunk} gave {result:?}"
        );
    }
}

#[test]
fn a_split_extm3u_prefix_asks_for_more_data_instead_of_guessing() {
    let mut d = Detector::new();
    d.push(b"#EXT");
    assert!(
        matches!(d.detect(None), Detection::NeedMoreData { .. }),
        "a partial #EXTM3U must not be judged yet"
    );
    d.push(b"M3U\n#EXT-X-VERSION:3\n");
    assert_eq!(
        d.detect(None),
        Detection::Identified(SourceKind::HlsMediaPlaylist)
    );
}

#[test]
fn html_error_body_served_as_http_200_becomes_an_actionable_error() {
    let d = detect_all(
        b"<!DOCTYPE html>\n<html><body>403 Forbidden</body></html>",
        Some("video/mp2t"),
    );
    match d {
        Detection::Unsupported(Unsupported::NotMedia { preview }) => {
            assert!(
                preview.contains("403"),
                "preview should retain the message: {preview}"
            );
        }
        other => panic!("expected NotMedia, got {other:?}"),
    }
}

#[test]
fn json_error_body_becomes_an_actionable_error() {
    let d = detect_all(br#"{"error":"invalid credentials"}"#, None);
    assert!(
        matches!(d, Detection::Unsupported(Unsupported::NotMedia { .. })),
        "got {d:?}"
    );
}

#[test]
fn encrypted_playlist_is_rejected_rather_than_handed_to_a_client_that_cannot_decrypt() {
    let playlist = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://example.invalid/key\"\n\
#EXTINF:2.0,\nseg0.ts\n";
    assert_eq!(
        detect_all(playlist, None),
        Detection::Unsupported(Unsupported::EncryptedPlaylist)
    );
}

#[test]
fn master_playlist_is_distinguished_from_a_media_playlist() {
    let master = b"#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\nlow.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=258000,CODECS=\"mp4a.40.2\"\nhigh.m3u8\n";
    assert_eq!(
        detect_all(master, None),
        Detection::Identified(SourceKind::HlsMasterPlaylist)
    );
}

#[test]
fn variant_selection_is_deterministic_and_prefers_highest_bandwidth() {
    let master = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\nlow.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=258000,CODECS=\"mp4a.40.2\"\nhigh.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=64000\nlowest.m3u8\n";
    assert_eq!(select_variant(master), Some("high.m3u8"));
    // Same input, same answer, every time.
    for _ in 0..5 {
        assert_eq!(select_variant(master), Some("high.m3u8"));
    }
}

#[test]
fn variant_selection_handles_commas_inside_quoted_attributes() {
    // A naive split on ',' would misparse CODECS and lose the BANDWIDTH.
    let master = "#EXTM3U\n\
#EXT-X-STREAM-INF:CODECS=\"mp4a.40.2,avc1.4d401f\",BANDWIDTH=900000\nbig.m3u8\n\
#EXT-X-STREAM-INF:CODECS=\"mp4a.40.2\",BANDWIDTH=100000\nsmall.m3u8\n";
    assert_eq!(select_variant(master), Some("big.m3u8"));
}

#[test]
fn ties_break_deterministically_on_uri() {
    let master = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=100000\nb.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=100000\na.m3u8\n";
    assert_eq!(select_variant(master), Some("b.m3u8"));
}

#[test]
fn probe_bytes_are_bounded_and_retained_for_replay() {
    let bytes = fixture("aac-0.ts");
    let mut d = Detector::with_budget(512);
    d.push(&bytes);
    assert_eq!(
        d.buffered().len(),
        512,
        "must not retain more than the budget"
    );
    assert!(d.is_full());
    // Every retained byte is the true prefix, so replay cannot corrupt the stream.
    assert_eq!(d.buffered(), &bytes[..512]);
}

#[test]
fn an_exhausted_budget_reports_exhaustion_rather_than_asking_forever() {
    // Random-looking bytes that never resolve, with a tiny budget.
    let noise: Vec<u8> = (0..256u32)
        .map(|i| (i.wrapping_mul(37) % 251) as u8)
        .collect();
    let mut d = Detector::with_budget(64);
    d.push(&noise);
    match d.detect(None) {
        Detection::Unsupported(_) => {}
        other => panic!("a full budget must conclude, got {other:?}"),
    }
}

#[test]
fn detection_never_consumes_bytes_playback_needs() {
    let bytes = fixture("aac-0.ts");
    let mut d = Detector::new();
    d.push(&bytes);
    assert!(matches!(d.detect(None), Detection::Identified(_)));
    assert_eq!(
        d.buffered(),
        &bytes[..],
        "detection must be non-destructive"
    );
}
