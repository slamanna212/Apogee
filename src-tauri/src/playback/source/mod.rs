//! Common source abstraction: something that emits
//! `apogee_playback_core::pipeline::SourceEvent`s (or a terminal error),
//! driven by cancellable network IO via `crate::network::NetworkService`.
//!
//! [`open`] is the single entry point. It never routes on the URL extension
//! or a declared content type - only on the bytes, via
//! `apogee_playback_core::detect::Detector` - because the target provider
//! mislabels both (see `SYMPHONIA_PLAYBACK_PLAN.md` section 7 and
//! `docs/symphonia-migration-progress.md`'s live provider re-check). Every
//! byte the detector examines is retained and replayed into whichever path
//! is chosen; detection must never consume stream data that playback still
//! needs.
//!
//! This module builds no decoder and reaches no further than
//! `SourceEvent` - turning those into PCM (`apogee_playback_core::pipeline::Pipeline`)
//! and retry/backoff policy on top of a failed [`SourceError`] are
//! milestone M3's job (the controller), per the plan.

pub mod hls;
pub mod http;

use apogee_playback_core::detect::{select_variant, Detection, Detector, SourceKind, Unsupported};
use apogee_playback_core::pipeline::SourceEvent;
use tokio_util::sync::CancellationToken;

use crate::network::{
    redact_text, resolve_relative, ContinuousTsStream, NetworkError, NetworkService,
};

pub use hls::HlsSource;
pub use http::DirectTsSource;

/// Upper bound on how many bytes [`open`] will read from the detection
/// probe connection while assembling a complete HLS playlist body (media or
/// master) that was still arriving when `Detector` reached a conclusion.
///
/// This is this module's own safety bound for that one ad hoc read - not
/// `NetworkServiceConfig::hls_playlist_max_body_bytes`, which governs only
/// the *subsequent*, properly-profiled `NetworkService::fetch_hls_playlist`
/// calls `HlsSource` makes for every later reload. A real live playlist
/// (a bounded sliding window of `EXTINF`+URI lines) is nowhere near this
/// size; it exists to fail loudly on a pathological body rather than grow
/// memory without bound.
const MAX_PROBED_PLAYLIST_BYTES: usize = 2 * 1024 * 1024;

/// Errors that stop [`open`] or a source's `next_event`. Every variant is
/// actionable - none of them should ever surface to a user as silence or an
/// endless stall - and every message is already redacted (see
/// `network::redact_text`): credentials in this app live in URL path
/// segments, and any message here may have started life inside a
/// `reqwest`/`hls_runtime` error that embedded a request URL as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// Transport/timeout/redirect/status/body-bound failure from
    /// `NetworkService`.
    Network(String),
    /// Bytes were conclusively identified as something this app does not
    /// play (yet), or detection could not resolve them at all.
    Unsupported(String),
    /// `hls-runtime` rejected a playlist/resource, or `HlsIngest`/
    /// `Fmp4Demux` could not make sense of already-demuxed output.
    Demux(String),
    /// The connection closed before `Detector` could reach a conclusion.
    StreamEndedDuringDetection { examined: usize },
    /// An HLS master playlist named zero renditions.
    NoVariants,
    /// The caller's `CancellationToken` fired while a request, a chunk
    /// read, or a wait was pending.
    Cancelled,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(message) => write!(f, "network error: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported source: {message}"),
            Self::Demux(message) => write!(f, "stream demux error: {message}"),
            Self::StreamEndedDuringDetection { examined } => write!(
                f,
                "connection closed before the stream format could be identified \
                 ({examined} bytes examined)"
            ),
            Self::NoVariants => write!(f, "HLS master playlist named no renditions"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<NetworkError> for SourceError {
    fn from(error: NetworkError) -> Self {
        match error {
            NetworkError::Cancelled => Self::Cancelled,
            other => Self::Network(redact_text(&other.to_string())),
        }
    }
}

impl From<Unsupported> for SourceError {
    fn from(unsupported: Unsupported) -> Self {
        match unsupported {
            Unsupported::NotMedia { preview } => Self::Unsupported(format!(
                "server returned a non-media (HTML/JSON) body instead of an audio stream: {}",
                redact_text(&preview)
            )),
            Unsupported::EncryptedPlaylist => Self::Unsupported(
                "HLS playlist is encrypted (EXT-X-KEY); decryption is not supported".to_string(),
            ),
            Unsupported::UnknownFormat => {
                Self::Unsupported("stream bytes did not match any supported format".to_string())
            }
            Unsupported::Ambiguous { candidates } => {
                Self::Unsupported(format!("stream format is ambiguous between {candidates:?}"))
            }
            Unsupported::BudgetExhausted { examined } => Self::Unsupported(format!(
                "could not identify the stream format within {examined} probed bytes"
            )),
        }
    }
}

/// A driveable source, already routed to the right path.
///
/// Both variants are boxed so `Source` itself stays small (a couple of
/// words) regardless of which path is live, rather than always reserving
/// space for the larger of the two.
pub enum Source {
    Ts(Box<DirectTsSource>),
    Hls(Box<HlsSource>),
}

impl std::fmt::Debug for Source {
    // Manual, not derived: `DirectTsSource`/`HlsSource` hold non-`Debug`
    // types (`ContinuousTsStream`, `NetworkService`, ...) that have no
    // useful debug representation anyway; this exists only so `Source`
    // satisfies `Result::{expect_err, unwrap_err}` in tests.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ts(_) => f.write_str("Source::Ts(..)"),
            Self::Hls(_) => f.write_str("Source::Hls(..)"),
        }
    }
}

impl Source {
    /// Drains the next event. See [`DirectTsSource::next_event`] /
    /// [`HlsSource::next_event`] for what `Ok(None)` means on each path.
    pub async fn next_event(&mut self) -> Result<Option<SourceEvent>, SourceError> {
        match self {
            Self::Ts(source) => source.next_event().await,
            Self::Hls(source) => source.next_event().await,
        }
    }
}

/// Opens `url`, identifies what it actually is from its bytes, and returns a
/// driveable [`Source`] for the matching path. All network IO goes through
/// `network` and is cancellable via `cancel`.
///
/// # Routing (plan section 7 / requirement B)
///
/// - `MpegTs` -> the direct path ([`DirectTsSource`]), continuing to read
///   from the very connection detection already opened.
/// - `HlsMediaPlaylist` -> [`HlsSource`], constructed against this
///   connection's *final* (post-redirect) URL, with the client's seeded
///   first playlist fetch serviced from bytes already read off this same
///   connection (never a second, possibly-inconsistent live fetch).
/// - `HlsMasterPlaylist` -> [`select_variant`] picks a rendition
///   deterministically, its URI is resolved against the *final* master
///   playlist URL (never the originally requested one - see
///   `docs/symphonia-dependency-validation.md`'s gap #1), that variant is
///   fetched fresh, and [`HlsSource`] is constructed against *its* final
///   URL.
/// - `AdtsAac`/`Mp3`/`Isobmff` -> a clear "unsupported source" error. No
///   adapter is built for these; see the plan's scope note.
/// - Every `Unsupported` outcome, and a connection that closes before
///   detection concludes, become a specific, actionable [`SourceError`] -
///   never a silent stall.
pub async fn open(
    url: &str,
    network: &NetworkService,
    cancel: &CancellationToken,
) -> Result<Source, SourceError> {
    let mut stream = network.open_continuous_stream(url, cancel).await?;
    let final_url = stream.final_url().clone();
    let mut detector = Detector::new();

    let detection = loop {
        match detector.detect(None) {
            Detection::NeedMoreData { .. } => {
                if detector.is_full() {
                    break Detection::Unsupported(Unsupported::BudgetExhausted {
                        examined: detector.buffered().len(),
                    });
                }
                match stream.next_chunk().await? {
                    Some(chunk) => detector.push(&chunk),
                    None => {
                        return Err(SourceError::StreamEndedDuringDetection {
                            examined: detector.buffered().len(),
                        });
                    }
                }
            }
            other => break other,
        }
    };

    match detection {
        Detection::Identified(SourceKind::MpegTs { .. }) => Ok(Source::Ts(Box::new(
            DirectTsSource::new(stream, detector.buffered().to_vec()),
        ))),
        Detection::Identified(SourceKind::HlsMediaPlaylist) => {
            let body = read_remaining_playlist(&mut stream, detector.buffered().to_vec()).await?;
            let source = HlsSource::from_initial_playlist(
                final_url,
                &body,
                network.clone(),
                cancel.clone(),
            )?;
            Ok(Source::Hls(Box::new(source)))
        }
        Detection::Identified(SourceKind::HlsMasterPlaylist) => {
            let master_body =
                read_remaining_playlist(&mut stream, detector.buffered().to_vec()).await?;
            let master_text = std::str::from_utf8(&master_body).map_err(|_| {
                SourceError::Unsupported("HLS master playlist was not valid UTF-8".to_string())
            })?;
            let variant_uri = select_variant(master_text).ok_or(SourceError::NoVariants)?;
            let variant_url = resolve_relative(&final_url, variant_uri)?;
            let fetched = network
                .fetch_hls_playlist(variant_url.as_str(), cancel)
                .await?;
            let source = HlsSource::from_initial_playlist(
                fetched.final_url,
                &fetched.bytes,
                network.clone(),
                cancel.clone(),
            )?;
            Ok(Source::Hls(Box::new(source)))
        }
        Detection::Identified(SourceKind::AdtsAac) => Err(SourceError::Unsupported(
            "raw ADTS AAC elementary streams are not a supported source yet".to_string(),
        )),
        Detection::Identified(SourceKind::Mp3) => Err(SourceError::Unsupported(
            "raw MP3 elementary streams are not a supported source yet".to_string(),
        )),
        Detection::Identified(SourceKind::Isobmff) => Err(SourceError::Unsupported(
            "raw ISO-BMFF/fragmented MP4 is not a supported source yet".to_string(),
        )),
        Detection::Unsupported(unsupported) => Err(SourceError::from(unsupported)),
        Detection::NeedMoreData { .. } => {
            unreachable!("the loop above only ever exits on Identified or Unsupported")
        }
    }
}

/// Continues reading off `stream` (the same connection [`Detector`] already
/// probed) until the body ends, returning the full accumulated bytes. Used
/// only for the two HLS playlist branches of [`open`] - see
/// [`MAX_PROBED_PLAYLIST_BYTES`].
async fn read_remaining_playlist(
    stream: &mut ContinuousTsStream,
    mut buffer: Vec<u8>,
) -> Result<Vec<u8>, SourceError> {
    loop {
        if buffer.len() > MAX_PROBED_PLAYLIST_BYTES {
            return Err(SourceError::Unsupported(format!(
                "playlist body exceeded {MAX_PROBED_PLAYLIST_BYTES} bytes without ending"
            )));
        }
        match stream.next_chunk().await? {
            Some(chunk) => buffer.extend_from_slice(&chunk),
            None => return Ok(buffer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::NetworkServiceConfig;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    // -----------------------------------------------------------------
    // Fixtures: real, previously-captured MPEG-TS audio (see
    // playback-core/tests/fixtures), reused here as real segment/body
    // payloads rather than hand-typed bytes.
    // -----------------------------------------------------------------

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("playback-core/tests/fixtures")
                .join(name),
        )
        .unwrap_or_else(|error| panic!("missing fixture {name}: {error}"))
    }

    // -----------------------------------------------------------------
    // Minimal hand-rolled HTTP/1.1 test server - same technique as
    // `network.rs`'s own tests, for the same reason: exact control over
    // wire bytes/timing/host, proven against real sockets and real Reqwest
    // parsing rather than a canned mock.
    // -----------------------------------------------------------------

    async fn read_request_head(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if stream.read_exact(&mut byte).await.is_err() {
                break;
            }
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    fn request_path(head: &str) -> String {
        head.lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_string()
    }

    fn spawn_server<F, Fut>(handler: F) -> (SocketAddr, tokio::task::JoinHandle<()>)
    where
        F: Fn(TcpStream, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let listener = TcpListener::from_std(listener).unwrap();
        let handler = Arc::new(handler);
        let join = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let head = read_request_head(&mut stream).await;
                    handler(stream, head).await;
                });
            }
        });
        (addr, join)
    }

    fn base_url(addr: SocketAddr, path: &str) -> String {
        format!("http://{addr}{path}")
    }

    async fn write_response(stream: &mut TcpStream, status: &str, headers: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(body).await;
    }

    fn default_service() -> NetworkService {
        NetworkService::with_config(NetworkServiceConfig {
            continuous_ts_stall_timeout: Duration::from_secs(5),
            hls_playlist_timeout: Duration::from_secs(5),
            hls_segment_timeout: Duration::from_secs(5),
            ..NetworkServiceConfig::default()
        })
        .unwrap()
    }

    // -----------------------------------------------------------------
    // Detection routes on bytes, not the URL extension or MIME.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn an_m3u8_url_whose_body_is_actually_raw_ts_routes_to_the_direct_ts_path() {
        let ts_bytes = fixture("aac-0.ts");
        let body = ts_bytes.clone();
        let (addr, _server) = spawn_server(move |mut stream, _head| {
            let body = body.clone();
            async move {
                write_response(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/vnd.apple.mpegurl\r\n",
                    &body,
                )
                .await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let source = open(&base_url(addr, "/stream.m3u8"), &service, &cancel)
            .await
            .unwrap();

        assert!(
            matches!(source, Source::Ts(_)),
            "an .m3u8 URL serving raw TS bytes must route to the direct-TS path, \
             not the HLS path, despite its extension and declared content type"
        );
    }

    #[tokio::test]
    async fn probed_bytes_are_replayed_intact_into_the_ts_demuxer() {
        // A real AAC-LC track decodes to a known, non-zero number of PCM
        // frames. If the probe's buffered prefix were dropped instead of
        // replayed, the leading access unit(s) would be lost and either the
        // decode would fail outright or the frame count would come up
        // short against the same bytes decoded directly.
        let ts_bytes = fixture("aac-0.ts");
        let body = ts_bytes.clone();
        let (addr, _server) = spawn_server(move |mut stream, _head| {
            let body = body.clone();
            async move {
                write_response(&mut stream, "200 OK", "Content-Type: video/mp2t\r\n", &body).await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/live.ts"), &service, &cancel)
            .await
            .unwrap();

        let mut track_seen = false;
        let mut access_units = 0usize;
        while let Some(event) = source.next_event().await.unwrap() {
            match event {
                SourceEvent::Track(_) => track_seen = true,
                SourceEvent::Access(_) => access_units += 1,
                _ => {}
            }
        }

        assert!(track_seen, "expected a track configuration event");
        assert!(
            access_units > 0,
            "expected at least one decoded access unit from the replayed probe bytes"
        );

        // Cross-check against feeding the exact same bytes directly into a
        // fresh `TsIngest`, with no network/probe layer involved at all -
        // proves the probe-then-replay path lost nothing.
        let mut direct = apogee_playback_core::pipeline::TsIngest::new();
        direct.feed(&ts_bytes);
        direct.finish();
        let mut direct_units = 0usize;
        while let Some(event) = direct.poll() {
            if matches!(event, SourceEvent::Access(_)) {
                direct_units += 1;
            }
        }
        assert_eq!(
            access_units, direct_units,
            "replaying the detector's buffered prefix must decode exactly as many \
             access units as feeding the same bytes directly"
        );
    }

    // -----------------------------------------------------------------
    // HTML/JSON error body served as HTTP 200 is an actionable error, not
    // endless probing.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn an_html_error_body_served_as_http_200_becomes_an_actionable_error() {
        let (addr, _server) = spawn_server(move |mut stream, _head| async move {
            write_response(
                &mut stream,
                "200 OK",
                "Content-Type: text/html\r\n",
                b"<html><body>upstream channel unavailable</body></html>",
            )
            .await;
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let error = open(&base_url(addr, "/live.ts"), &service, &cancel)
            .await
            .expect_err("an HTML body must not be treated as playable media");

        assert!(
            matches!(error, SourceError::Unsupported(_)),
            "expected an actionable Unsupported error, got {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("non-media"),
            "error message should explain what went wrong: {message}"
        );
    }

    // -----------------------------------------------------------------
    // A credential in the URL never reaches a surfaced error.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn a_credential_in_the_url_is_absent_from_every_surfaced_error() {
        // Nothing is listening past the accept - `open` will fail with a
        // status error carrying a real, un-redacted-by-Reqwest URL in its
        // Display impl unless our redaction actually works end to end.
        let (addr, _server) = spawn_server(move |mut stream, _head| async move {
            write_response(&mut stream, "404 Not Found", "", b"").await;
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let url = base_url(addr, "/live/produser/sup3rSecret/1.ts");
        let error = open(&url, &service, &cancel).await.unwrap_err();

        let message = error.to_string();
        assert!(
            !message.contains("sup3rSecret") && !message.contains("produser"),
            "credential leaked into a surfaced error: {message}"
        );
    }

    // -----------------------------------------------------------------
    // HLS: redirect chain followed, absolute segment paths resolved
    // against the FINAL (post-redirect) host, live playlist keeps
    // refreshing, cancellation works mid-wait and mid-fetch.
    // -----------------------------------------------------------------

    /// Builds a live (no `#EXT-X-ENDLIST`) classic HLS v3 media playlist -
    /// self-contained `.ts` segments, absolute-path URIs, matching the
    /// target provider's confirmed real shape (see
    /// `docs/symphonia-migration-progress.md`).
    fn live_playlist(media_sequence: u64, hash: &str) -> String {
        format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
             #EXT-X-MEDIA-SEQUENCE:{media_sequence}\n\
             #EXTINF:10.0,\n/hls/{hash}/seg_{media_sequence}.ts\n"
        )
    }

    struct HlsFixture {
        /// Host that only ever 302s, never serves content.
        redirect_addr: SocketAddr,
        /// Host the redirect points to - serves the playlist and segments.
        final_addr: SocketAddr,
        playlist_requests: Arc<AtomicUsize>,
        segment_requests_on_final_host: Arc<AtomicUsize>,
        segment_requests_on_redirect_host: Arc<AtomicUsize>,
        _redirect_server: tokio::task::JoinHandle<()>,
        _final_server: tokio::task::JoinHandle<()>,
    }

    fn spawn_hls_fixture(hash: &'static str) -> HlsFixture {
        let playlist_requests = Arc::new(AtomicUsize::new(0));
        let segment_requests_on_final_host = Arc::new(AtomicUsize::new(0));
        let segment_requests_on_redirect_host = Arc::new(AtomicUsize::new(0));

        let ts_segment = fixture("aac-0.ts");

        // Bind the final host first so the redirect target address is
        // known before the redirect host starts answering.
        let final_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        final_listener.set_nonblocking(true).unwrap();
        let final_addr = final_listener.local_addr().unwrap();

        let playlist_requests_final = playlist_requests.clone();
        let segment_requests_final = segment_requests_on_final_host.clone();
        let (final_addr, final_server) = {
            let listener = TcpListener::from_std(final_listener).unwrap();
            let ts_segment = ts_segment.clone();
            let join = tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    let ts_segment = ts_segment.clone();
                    let playlist_requests = playlist_requests_final.clone();
                    let segment_requests = segment_requests_final.clone();
                    tokio::spawn(async move {
                        let head = read_request_head(&mut stream).await;
                        let path = request_path(&head);
                        if path.ends_with(".m3u8") {
                            let n = playlist_requests.fetch_add(1, Ordering::SeqCst);
                            let body = live_playlist(n as u64, hash);
                            write_response(
                                &mut stream,
                                "200 OK",
                                "Content-Type: application/vnd.apple.mpegurl\r\n",
                                body.as_bytes(),
                            )
                            .await;
                        } else if path.ends_with(".ts") {
                            segment_requests.fetch_add(1, Ordering::SeqCst);
                            write_response(
                                &mut stream,
                                "200 OK",
                                "Content-Type: video/mp2t\r\n",
                                &ts_segment,
                            )
                            .await;
                        } else {
                            write_response(&mut stream, "404 Not Found", "", b"").await;
                        }
                    });
                }
            });
            (final_addr, join)
        };

        let segment_requests_redirect = segment_requests_on_redirect_host.clone();
        let (redirect_addr, redirect_server) = spawn_server(move |mut stream, head| {
            let path = request_path(&head);
            let segment_requests_redirect = segment_requests_redirect.clone();
            async move {
                if path.ends_with(".ts") {
                    // Absolute-path segment requests must NEVER land here -
                    // resolving against the originally requested host
                    // instead of the post-redirect one is exactly the bug
                    // this fixture exists to catch.
                    segment_requests_redirect.fetch_add(1, Ordering::SeqCst);
                    write_response(&mut stream, "404 Not Found", "", b"").await;
                    return;
                }
                let location = format!("http://{final_addr}/hls/{hash}/stream.m3u8");
                write_response(
                    &mut stream,
                    "302 Found",
                    &format!("Location: {location}\r\n"),
                    b"",
                )
                .await;
            }
        });

        HlsFixture {
            redirect_addr,
            final_addr,
            playlist_requests,
            segment_requests_on_final_host: segment_requests_on_final_host.clone(),
            segment_requests_on_redirect_host: segment_requests_on_redirect_host.clone(),
            _redirect_server: redirect_server,
            _final_server: final_server,
        }
    }

    #[tokio::test]
    async fn redirect_chain_is_followed_and_the_final_url_is_used_downstream() {
        let fixture = spawn_hls_fixture("abc123");
        let service = default_service();
        let cancel = CancellationToken::new();

        let source = open(
            &base_url(fixture.redirect_addr, "/request.m3u8"),
            &service,
            &cancel,
        )
        .await
        .unwrap();

        let Source::Hls(hls) = source else {
            panic!("expected the HLS path");
        };
        assert!(
            hls.playlist_url()
                .starts_with(&format!("http://{}", fixture.final_addr)),
            "HlsSource must be driven against the final, post-redirect URL, got {}",
            hls.playlist_url()
        );
    }

    #[tokio::test]
    async fn absolute_segment_paths_resolve_against_the_redirected_host_not_the_requested_one() {
        let fixture = spawn_hls_fixture("def456");
        let service = default_service();
        let cancel = CancellationToken::new();

        let mut source = open(
            &base_url(fixture.redirect_addr, "/request.m3u8"),
            &service,
            &cancel,
        )
        .await
        .unwrap();

        // Drain events until at least one access unit has come through -
        // that requires the segment fetch to have actually succeeded.
        let mut access_units = 0usize;
        for _ in 0..500 {
            match source.next_event().await.unwrap() {
                Some(SourceEvent::Access(_)) => {
                    access_units += 1;
                    if access_units >= 1 {
                        break;
                    }
                }
                Some(_) => {}
                None => break,
            }
        }

        assert!(
            access_units > 0,
            "expected the segment fetch to succeed and produce at least one access unit"
        );
        assert_eq!(
            fixture
                .segment_requests_on_redirect_host
                .load(Ordering::SeqCst),
            0,
            "the segment must never be requested from the originally requested host"
        );
        assert!(
            fixture
                .segment_requests_on_final_host
                .load(Ordering::SeqCst)
                >= 1,
            "the segment must be requested from the post-redirect host"
        );
    }

    #[tokio::test]
    async fn a_live_playlist_with_no_endlist_keeps_refreshing_rather_than_terminating() {
        let fixture = spawn_hls_fixture("ghi789");
        let service = default_service();
        let cancel = CancellationToken::new();

        let mut source = open(
            &base_url(fixture.redirect_addr, "/request.m3u8"),
            &service,
            &cancel,
        )
        .await
        .unwrap();

        // Drain a good number of events - enough for the client to have
        // exhausted the first playlist's single segment and gone back for
        // more at least once.
        let mut saw_end_of_stream = false;
        for _ in 0..2000 {
            match source.next_event().await.unwrap() {
                Some(SourceEvent::EndOfStream) => {
                    saw_end_of_stream = true;
                    break;
                }
                Some(_) => {}
                None => break,
            }
            if fixture.playlist_requests.load(Ordering::SeqCst) >= 3 {
                break;
            }
        }

        assert!(
            !saw_end_of_stream,
            "a playlist with no #EXT-X-ENDLIST must never terminate the source"
        );
        assert!(
            fixture.playlist_requests.load(Ordering::SeqCst) >= 2,
            "expected the client to reload the live playlist more than once, got {} reloads",
            fixture.playlist_requests.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn cancellation_during_a_playlist_wait_returns_promptly() {
        // An empty live playlist (no segments, no #EXT-X-ENDLIST): the
        // client's only actions are an immediate reload followed by a
        // multi-second `Action::WaitMs` (from `#EXT-X-TARGETDURATION`). The
        // reload always answers instantly, so the very first `next_event`
        // call is guaranteed to run the reload and then park in the real
        // `cancellable_sleep` wait - no draining or guesswork about how many
        // access-unit events a segment produces.
        let (addr, _server) = spawn_server(move |mut stream, head| async move {
            let path = request_path(&head);
            if path.ends_with(".m3u8") {
                write_response(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/vnd.apple.mpegurl\r\n",
                    b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
                      #EXT-X-MEDIA-SEQUENCE:0\n",
                )
                .await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/stream.m3u8"), &service, &cancel)
            .await
            .unwrap();

        let cancel_trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_trigger.cancel();
        });

        let started = Instant::now();
        let result = source.next_event().await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(SourceError::Cancelled)));
        assert!(
            elapsed < Duration::from_secs(2),
            "cancellation during a playlist wait did not return promptly: took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn cancellation_during_a_segment_fetch_returns_promptly() {
        let (addr, _server) = spawn_server(move |mut stream, head| async move {
            let path = request_path(&head);
            if path.ends_with(".m3u8") {
                write_response(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/vnd.apple.mpegurl\r\n",
                    live_playlist(0, "fetch-cancel").as_bytes(),
                )
                .await;
            } else if path.ends_with(".ts") {
                // Never respond to the segment fetch - holds the connection
                // open well past a reasonable test timeout.
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/stream.m3u8"), &service, &cancel)
            .await
            .unwrap();

        let cancel_trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_trigger.cancel();
        });

        let started = Instant::now();
        let mut result = Ok(None);
        for _ in 0..50 {
            result = source.next_event().await;
            if matches!(result, Err(SourceError::Cancelled)) {
                break;
            }
        }
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(SourceError::Cancelled)),
            "expected cancellation during the stalled segment fetch, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "cancellation during a segment fetch did not return promptly: took {elapsed:?}"
        );
    }
}
