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

use std::time::Duration;

use apogee_playback_core::detect::{
    classify_complete_hls_playlist, select_variant, Detection, Detector, SourceKind, Unsupported,
    MAX_PROBE_BYTES,
};
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

/// Upper bound on total wall-clock time spent assembling that same complete
/// initial playlist body. `ContinuousTsStream::next_chunk` already bounds
/// the gap between any two individual chunks (`continuous_ts_stall_timeout`),
/// but a origin that keeps a connection alive with a slow, steady trickle of
/// small chunks - never quite stalling, never finishing - would otherwise
/// never be bounded even though [`MAX_PROBED_PLAYLIST_BYTES`] bounds its
/// size. Real live playlists are small and arrive promptly; this exists to
/// fail loudly on a pathological connection rather than hang detection
/// indefinitely.
const PROBED_PLAYLIST_ASSEMBLY_DEADLINE: Duration = Duration::from_secs(20);

/// Errors that stop [`open`] or a source's `next_event`. Every variant is
/// actionable - none of them should ever surface to a user as silence or an
/// endless stall - and every message is already redacted (see
/// `network::redact_text`): credentials in this app live in URL path
/// segments, and any message here may have started life inside a
/// `reqwest`/`hls_runtime` error that embedded a request URL as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// Transport/timeout/redirect/body-bound failure from `NetworkService`.
    Network(String),
    /// The server answered with an unsuccessful status.
    ///
    /// Kept separate from `Network` so the status code survives: whether retrying can
    /// possibly help depends on it, and the body often carries the actual reason.
    Status { status: u16, detail: Option<String> },
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
            // Lead with the server's own reason when it gave one: "No streams assigned to
            // channel" tells the user what to fix, where "503" does not.
            Self::Status { status, detail } => match detail {
                Some(detail) => write!(f, "{detail} (HTTP {status})"),
                None => write!(f, "server returned HTTP {status}"),
            },
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
            NetworkError::Status { status, detail } => Self::Status {
                status: status.as_u16(),
                detail: detail.map(|d| redact_text(&d)),
            },
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

/// Anything [`detect_source_kind`] can pull sequential byte chunks from.
/// [`ContinuousTsStream`] is the real, production implementation; tests
/// implement this over a fixed, caller-chosen sequence of chunks instead of
/// depending on how the real network transport happens to fragment a body -
/// chunk boundaries there are an implementation detail of Reqwest/Hyper, not
/// something a test can pin to an exact byte count.
trait ChunkSource {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SourceError>;
}

impl ChunkSource for ContinuousTsStream {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SourceError> {
        Ok(ContinuousTsStream::next_chunk(self)
            .await?
            .map(|bytes| bytes.to_vec()))
    }
}

/// Runs the bounded-prefix detection loop against `source`, returning the
/// concluded [`Detection`] and, separately, every byte read while reaching
/// it - in order, regardless of [`Detector`]'s own bounded probe budget.
///
/// `Detector::push` silently caps what *it* retains once its budget is
/// full - correct for a bounded prefix decision - but the second return
/// value is what actually gets replayed into whichever path [`open`]
/// chooses, so it must never drop a byte a chunk that overruns the budget
/// still carries (finding 9).
async fn detect_source_kind<S: ChunkSource>(
    source: &mut S,
    budget: usize,
) -> Result<(Detection, Vec<u8>), SourceError> {
    let mut detector = Detector::with_budget(budget);
    let mut raw: Vec<u8> = Vec::new();

    let detection = loop {
        match detector.detect(None) {
            Detection::NeedMoreData { .. } => {
                if detector.is_full() {
                    break Detection::Unsupported(Unsupported::BudgetExhausted {
                        examined: detector.buffered().len(),
                    });
                }
                match source.next_chunk().await? {
                    Some(chunk) => {
                        raw.extend_from_slice(&chunk);
                        detector.push(&chunk);
                    }
                    None => {
                        return Err(SourceError::StreamEndedDuringDetection {
                            examined: raw.len(),
                        });
                    }
                }
            }
            other => break other,
        }
    };
    Ok((detection, raw))
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
    let (detection, raw) = detect_source_kind(&mut stream, MAX_PROBE_BYTES).await?;

    match detection {
        Detection::Identified(SourceKind::MpegTs { .. }) => {
            Ok(Source::Ts(Box::new(DirectTsSource::new(stream, raw))))
        }
        // Recognizing `#EXTM3U` only establishes that this is HLS - it does
        // not establish the subtype or encryption policy, since
        // `#EXT-X-STREAM-INF`/`#EXT-X-KEY` need not appear near the top of
        // the file. Trusting `Detector`'s guess here would reproduce
        // exactly the chunk-boundary bug finding 8 describes: whichever
        // classification the *bounded probe prefix* happened to support
        // would win, even though more of the body - carrying a
        // subtype-deciding tag - was still on the wire. So both playlist
        // kinds fall into the same path: read the complete body first
        // (bounded, cancellable, deadlined), then classify that complete
        // body from scratch and trust only that answer.
        Detection::Identified(SourceKind::HlsMediaPlaylist | SourceKind::HlsMasterPlaylist) => {
            let body = read_remaining_playlist(&mut stream, raw).await?;
            let kind = classify_complete_hls_playlist(&body)?;
            open_hls_playlist(kind, final_url, body, network, cancel).await
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

/// Routes a *completely read and freshly reclassified* HLS playlist body to
/// the media or master path. `kind` must come from
/// [`classify_complete_hls_playlist`] run against `body` in full - never
/// from a bounded prefix guess - so this never re-derives it itself.
async fn open_hls_playlist(
    kind: SourceKind,
    initial_url: url::Url,
    body: Vec<u8>,
    network: &NetworkService,
    cancel: &CancellationToken,
) -> Result<Source, SourceError> {
    match kind {
        SourceKind::HlsMediaPlaylist => {
            let source = HlsSource::from_initial_playlist(
                initial_url,
                &body,
                network.clone(),
                cancel.clone(),
            )?;
            Ok(Source::Hls(Box::new(source)))
        }
        SourceKind::HlsMasterPlaylist => {
            let master_text = std::str::from_utf8(&body).map_err(|_| {
                SourceError::Unsupported("HLS master playlist was not valid UTF-8".to_string())
            })?;
            let variant_uri = select_variant(master_text).ok_or(SourceError::NoVariants)?;
            let variant_url = resolve_relative(&initial_url, variant_uri)?;
            let fetched = network
                .fetch_hls_playlist(variant_url.as_str(), cancel)
                .await?;
            // The same full-body validation applies to a fetched variant as
            // to the initial body: it is itself a complete playlist that
            // could equally carry `#EXT-X-KEY`, or - malformed origin -
            // resolve to another master. Never hand it to `HlsSource`
            // unclassified just because it came from a "selected variant"
            // step rather than the initial probe.
            let variant_kind = classify_complete_hls_playlist(&fetched.bytes)?;
            match variant_kind {
                SourceKind::HlsMediaPlaylist => {
                    let source = HlsSource::from_initial_playlist(
                        fetched.final_url,
                        &fetched.bytes,
                        network.clone(),
                        cancel.clone(),
                    )?;
                    Ok(Source::Hls(Box::new(source)))
                }
                _ => Err(SourceError::Unsupported(
                    "selected HLS variant did not resolve to a playable media playlist".to_string(),
                )),
            }
        }
        _ => unreachable!("classify_complete_hls_playlist only ever returns an HLS SourceKind"),
    }
}

/// Continues reading off `stream` (the same connection [`Detector`] already
/// probed) until the body ends, returning the full accumulated bytes. Used
/// only for the two HLS playlist branches of [`open`] - see
/// [`MAX_PROBED_PLAYLIST_BYTES`] and [`PROBED_PLAYLIST_ASSEMBLY_DEADLINE`].
async fn read_remaining_playlist(
    stream: &mut ContinuousTsStream,
    buffer: Vec<u8>,
) -> Result<Vec<u8>, SourceError> {
    read_remaining_playlist_with_deadline(stream, buffer, PROBED_PLAYLIST_ASSEMBLY_DEADLINE).await
}

async fn read_remaining_playlist_with_deadline(
    stream: &mut ContinuousTsStream,
    mut buffer: Vec<u8>,
    deadline: Duration,
) -> Result<Vec<u8>, SourceError> {
    // Checked once up front too: a caller-supplied `buffer` (the probe's
    // already-retained prefix) could conceivably already be at the limit.
    if buffer.len() > MAX_PROBED_PLAYLIST_BYTES {
        return Err(SourceError::Unsupported(format!(
            "playlist body exceeded {MAX_PROBED_PLAYLIST_BYTES} bytes without ending"
        )));
    }
    let assembly = async {
        loop {
            match stream.next_chunk().await? {
                Some(chunk) => {
                    // Checked *before* extending, per finding 9's memory-bound
                    // rule: the buffer must never even transiently grow past
                    // the limit, and an oversized playlist must fail
                    // explicitly rather than being silently truncated.
                    let projected = buffer.len().saturating_add(chunk.len());
                    if projected > MAX_PROBED_PLAYLIST_BYTES {
                        return Err(SourceError::Unsupported(format!(
                            "playlist body exceeded {MAX_PROBED_PLAYLIST_BYTES} bytes without ending"
                        )));
                    }
                    buffer.extend_from_slice(&chunk);
                }
                None => return Ok(buffer),
            }
        }
    };
    match tokio::time::timeout(deadline, assembly).await {
        Ok(result) => result,
        Err(_elapsed) => Err(SourceError::Network(
            "timed out assembling the initial HLS playlist body".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::NetworkServiceConfig;
    use apogee_playback_core::detect::MAX_PROBE_BYTES;
    use apogee_playback_core::pipeline::{Pipeline, TsIngest};
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

    /// Like [`write_response`], but writes `body` as separate TCP writes at
    /// `piece_lens` boundaries, with a short sleep between each write. The
    /// sleep is what actually forces separate `Response::chunk()` reads on
    /// the client side - back-to-back `write_all` calls with no gap are
    /// frequently coalesced into a single read by the kernel/loopback, which
    /// would silently turn a "many chunks" test into a "one chunk" test.
    /// `piece_lens` must sum to `body.len()`.
    async fn write_response_in_pieces(
        stream: &mut TcpStream,
        status: &str,
        headers: &str,
        body: &[u8],
        piece_lens: &[usize],
    ) {
        assert_eq!(
            piece_lens.iter().sum::<usize>(),
            body.len(),
            "piece_lens must exactly cover body"
        );
        let head = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let mut offset = 0usize;
        for (i, &len) in piece_lens.iter().enumerate() {
            let _ = stream.write_all(&body[offset..offset + len]).await;
            let _ = stream.flush().await;
            offset += len;
            if i + 1 < piece_lens.len() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
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
    // Shared decode helpers: feed a `Source`/raw TS bytes all the way to
    // PCM so byte-preservation bugs at the probe boundary (finding 9) show
    // up as wrong access-unit counts or wrong decoded samples, not just a
    // classification difference.
    // -----------------------------------------------------------------

    /// Feeds `bytes` directly into a fresh `TsIngest`/`Pipeline`, with no
    /// network or probe layer at all - the ground truth the chunk-boundary
    /// tests below compare their `open`/`detect_source_kind`-driven decode
    /// against.
    fn decode_ts_bytes_directly(bytes: &[u8]) -> (usize, Vec<f32>) {
        let mut ingest = TsIngest::new();
        ingest.feed(bytes);
        ingest.finish();
        let mut pipeline = Pipeline::new();
        let mut access_units = 0usize;
        let mut pcm = Vec::new();
        while let Some(event) = ingest.poll() {
            if matches!(&event, SourceEvent::Access(_)) {
                access_units += 1;
            }
            if let Some(block) = pipeline.accept(event).unwrap() {
                pcm.extend(block.samples);
            }
        }
        (access_units, pcm)
    }

    /// Drains `source` until `#EXT-X-ENDLIST` (or an error), returning the
    /// access-unit count. Used by the master/variant chunk-boundary tests,
    /// where only the count (not full PCM equality) is what varying the
    /// split point could possibly affect.
    async fn count_access_units_until_end(source: &mut Source) -> usize {
        let mut access_units = 0usize;
        for _ in 0..500 {
            match source.next_event().await.unwrap() {
                Some(SourceEvent::Access(_)) => access_units += 1,
                Some(SourceEvent::EndOfStream) => return access_units,
                Some(_) => {}
                None => return access_units,
            }
        }
        panic!("playlist did not reach #EXT-X-ENDLIST within 500 events");
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

    // -----------------------------------------------------------------
    // Finding 9: every byte the probe consumes is preserved and replayed
    // exactly once, even when a chunk crosses (or several chunks straddle)
    // the probe budget.
    //
    // These drive `detect_source_kind` directly against a fixed,
    // caller-chosen chunk sequence rather than through a real HTTP
    // connection. Reqwest/Hyper's own internal buffering decides how a body
    // gets fragmented into `Response::chunk()` calls, and empirically
    // (verified against this harness) it does not reliably hand back a
    // single >64 KiB chunk for a >64 KiB body even from one `write_all` on
    // the wire - so a real-socket test could not reliably pin the exact
    // boundary-crossing scenario this finding is about. A fixed chunk
    // sequence pins it exactly, including the review's reported case: one
    // physical chunk larger than `Detector`'s own retained budget.
    // -----------------------------------------------------------------

    /// A deterministic [`ChunkSource`] yielding exactly the given chunks,
    /// in order, then ending the stream.
    struct FixedChunks(std::collections::VecDeque<Vec<u8>>);

    impl ChunkSource for FixedChunks {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SourceError> {
            Ok(self.0.pop_front())
        }
    }

    /// Four real fixtures concatenated - comfortably over `MAX_PROBE_BYTES`
    /// (64 KiB) but still genuine, independently decodable TS/AAC content,
    /// not padding.
    fn oversized_ts_fixture() -> Vec<u8> {
        let bytes: Vec<u8> = ["aac-0.ts", "aac-1.ts", "aac-2.ts", "aac-3.ts"]
            .iter()
            .flat_map(|name| fixture(name))
            .collect();
        assert!(
            bytes.len() > MAX_PROBE_BYTES,
            "fixture concatenation must exceed the probe budget for this test to be meaningful"
        );
        bytes
    }

    /// Runs `detect_source_kind` (with `budget`) against `chunks` and
    /// asserts: it identifies MPEG-TS, the returned raw bytes are the
    /// exact, untruncated concatenation of `chunks`, and decoding those raw
    /// bytes produces exactly what decoding `expected` directly produces.
    async fn assert_chunking_preserves_every_byte(
        chunks: Vec<Vec<u8>>,
        expected: &[u8],
        budget: usize,
    ) {
        let total: usize = chunks.iter().map(Vec::len).sum();
        assert_eq!(
            total,
            expected.len(),
            "test bug: chunks must cover expected exactly"
        );
        assert!(
            total > budget,
            "test is meaningless unless the input exceeds the probe budget"
        );

        let mut source = FixedChunks(chunks.into());
        let (detection, raw) = detect_source_kind(&mut source, budget).await.unwrap();

        assert!(
            matches!(detection, Detection::Identified(SourceKind::MpegTs { .. })),
            "expected the concatenated fixtures to be identified as MPEG-TS, got {detection:?}"
        );
        assert_eq!(
            raw, expected,
            "every consumed byte must be preserved and replayed in order, regardless of \
             Detector's own bounded probe budget"
        );

        let (expected_access_units, expected_pcm) = decode_ts_bytes_directly(expected);
        let (access_units, pcm) = decode_ts_bytes_directly(&raw);
        assert!(
            access_units > 0,
            "expected at least one decoded access unit"
        );
        assert_eq!(
            access_units, expected_access_units,
            "byte loss at the probe boundary would drop access units"
        );
        assert_eq!(
            pcm, expected_pcm,
            "byte loss at the probe boundary would corrupt or shorten decoded PCM"
        );
    }

    #[tokio::test]
    async fn a_single_oversized_chunk_preserves_every_byte_across_the_probe_boundary() {
        let ts_bytes = oversized_ts_fixture();
        // The entire body arrives as ONE chunk larger than Detector's own
        // (default, production) 64 KiB budget - exactly the review's
        // reported 93,060-byte case.
        assert_chunking_preserves_every_byte(vec![ts_bytes.clone()], &ts_bytes, MAX_PROBE_BYTES)
            .await;
    }

    #[tokio::test]
    async fn several_chunks_where_the_last_crosses_the_probe_boundary_preserve_every_byte() {
        let ts_bytes = oversized_ts_fixture();
        // A real TS prefix at phase 0 confirms `LATTICE_WEAK` (a real match,
        // per `container_probe::ts`'s design) from as few as ~3 confirmed
        // 188-byte-strided sync bytes - under 400 bytes. At the production
        // 64 KiB budget that means detection always concludes on the very
        // first chunk of any realistic multi-chunk delivery, never
        // mid-sequence. A small budget here is what makes several small
        // chunks genuinely land *before* detection can conclude, so the
        // last (and only the last, larger) one is what both supplies
        // enough evidence to identify AND crosses the budget - distinctly
        // from the single-oversized-chunk case above.
        let budget = 450;
        let piece_lens = [100usize, 100, 100, ts_bytes.len() - 300];
        let mut offset = 0usize;
        let chunks: Vec<Vec<u8>> = piece_lens
            .iter()
            .map(|&len| {
                let chunk = ts_bytes[offset..offset + len].to_vec();
                offset += len;
                chunk
            })
            .collect();

        assert_chunking_preserves_every_byte(chunks, &ts_bytes, budget).await;
    }

    #[tokio::test]
    async fn playlist_bytes_beyond_the_probe_boundary_are_retained() {
        // A single very long, unrecognized tag pads the body well past the
        // 64 KiB probe budget before the segment line that actually matters
        // - `broadcast_hls::MediaPlaylist::parse` preserves unknown `#EXT-*`
        // tags verbatim rather than rejecting them, so this stays a valid
        // playlist throughout.
        let padding = "x".repeat(70_000);
        let playlist = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
             #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-CUSTOM-PAD:{padding}\n\
             #EXTINF:10.0,\n/seg0.ts\n#EXT-X-ENDLIST\n"
        );
        assert!(
            playlist.len() > MAX_PROBE_BYTES,
            "test is meaningless unless the playlist exceeds the probe budget"
        );
        let segment = fixture("aac-0.ts");

        let (addr, _server) = spawn_server(move |mut stream, head| {
            let playlist = playlist.clone();
            let segment = segment.clone();
            async move {
                let path = request_path(&head);
                if path.ends_with(".m3u8") {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        playlist.as_bytes(),
                    )
                    .await;
                } else {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: video/mp2t\r\n",
                        &segment,
                    )
                    .await;
                }
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/live.m3u8"), &service, &cancel)
            .await
            .unwrap();
        assert!(matches!(source, Source::Hls(_)));

        let access_units = count_access_units_until_end(&mut source).await;
        assert!(
            access_units > 0,
            "the segment referenced after 64 KiB of padding must still be fetched and decoded - \
             losing bytes past the probe boundary would have dropped or corrupted its URI"
        );
    }

    fn playlist_padded_to_exact_length(total_len: usize) -> String {
        let prefix = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
                      #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-CUSTOM-PAD:";
        let suffix = "\n#EXTINF:10.0,\n/seg0.ts\n#EXT-X-ENDLIST\n";
        let fixed_len = prefix.len() + suffix.len();
        assert!(
            total_len > fixed_len,
            "requested length too small to accommodate the fixed playlist scaffolding"
        );
        let padding = "x".repeat(total_len - fixed_len);
        format!("{prefix}{padding}{suffix}")
    }

    #[tokio::test]
    async fn a_playlist_body_at_exactly_the_probed_playlist_limit_is_accepted() {
        let playlist = playlist_padded_to_exact_length(MAX_PROBED_PLAYLIST_BYTES);
        assert_eq!(playlist.len(), MAX_PROBED_PLAYLIST_BYTES);
        let segment = fixture("aac-0.ts");

        let (addr, _server) = spawn_server(move |mut stream, head| {
            let playlist = playlist.clone();
            let segment = segment.clone();
            async move {
                let path = request_path(&head);
                if path.ends_with(".m3u8") {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        playlist.as_bytes(),
                    )
                    .await;
                } else {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: video/mp2t\r\n",
                        &segment,
                    )
                    .await;
                }
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/live.m3u8"), &service, &cancel)
            .await
            .expect("a playlist body exactly at the limit must be accepted, not rejected");

        let access_units = count_access_units_until_end(&mut source).await;
        assert!(access_units > 0);
    }

    #[tokio::test]
    async fn an_oversized_playlist_body_fails_explicitly_instead_of_being_truncated() {
        let playlist = playlist_padded_to_exact_length(MAX_PROBED_PLAYLIST_BYTES + 1);
        assert_eq!(playlist.len(), MAX_PROBED_PLAYLIST_BYTES + 1);

        let (addr, _server) = spawn_server(move |mut stream, _head| {
            let playlist = playlist.clone();
            async move {
                write_response(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/vnd.apple.mpegurl\r\n",
                    playlist.as_bytes(),
                )
                .await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let error = open(&base_url(addr, "/live.m3u8"), &service, &cancel)
            .await
            .expect_err("a playlist exceeding the limit by even one byte must fail explicitly");

        assert!(matches!(error, SourceError::Unsupported(_)));
        assert!(
            error.to_string().contains("exceeded"),
            "error should explain why: {error}"
        );
    }

    #[tokio::test]
    async fn cancellation_during_playlist_assembly_returns_promptly() {
        let (addr, _server) = spawn_server(move |mut stream, _head| async move {
            // Declare a body far larger than what is ever sent, so the
            // connection looks genuinely incomplete rather than merely
            // short - the client must keep waiting for more, exactly the
            // state cancellation needs to interrupt.
            let head = "HTTP/1.1 200 OK\r\n\
                        Content-Type: application/vnd.apple.mpegurl\r\n\
                        Content-Length: 999999\r\n\r\n";
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(b"#EXTM3U\n#EXT-X-VERSION:3\n").await;
            let _ = stream.flush().await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let cancel_trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_trigger.cancel();
        });

        let started = Instant::now();
        let result = open(&base_url(addr, "/stream.m3u8"), &service, &cancel).await;
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(SourceError::Cancelled)),
            "expected cancellation during playlist assembly, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "cancellation during playlist assembly did not return promptly: took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn playlist_assembly_times_out_rather_than_hanging_forever() {
        // A connection that keeps sending (tiny) data fast enough to dodge
        // the per-chunk stall timeout, but never finishes the body - only
        // an overall deadline on the whole assembly can bound this.
        let (addr, _server) = spawn_server(move |mut stream, _head| async move {
            let head = "HTTP/1.1 200 OK\r\n\
                        Content-Type: application/vnd.apple.mpegurl\r\n\
                        Content-Length: 999999\r\n\r\n";
            let _ = stream.write_all(head.as_bytes()).await;
            loop {
                if stream.write_all(b"#EXTM3U\n").await.is_err() {
                    break;
                }
                let _ = stream.flush().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut stream = service
            .open_continuous_stream(&base_url(addr, "/stream.m3u8"), &cancel)
            .await
            .unwrap();

        let started = Instant::now();
        let result = read_remaining_playlist_with_deadline(
            &mut stream,
            Vec::new(),
            Duration::from_millis(200),
        )
        .await;
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(SourceError::Network(_))),
            "expected a deadline error, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the assembly deadline did not fire promptly: took {elapsed:?}"
        );
    }

    // -----------------------------------------------------------------
    // Finding 8: master/media routing (and encryption policy) must not
    // depend on where the transport happens to split the playlist body.
    // -----------------------------------------------------------------

    /// Serves `master` (split at `master_pieces`) at `/master.m3u8`,
    /// `media` whole at `/media.m3u8`, and `segment` for anything else.
    /// Returns the access-unit count decoded after following the master ->
    /// variant -> segment chain, or panics if the playlist never reaches
    /// `#EXT-X-ENDLIST`.
    async fn run_master_scenario(
        segment: &[u8],
        media: &str,
        master: &str,
        master_pieces: &[usize],
    ) -> usize {
        let segment = segment.to_vec();
        let media = media.to_string();
        let master = master.to_string();
        let master_pieces = master_pieces.to_vec();
        let (addr, _server) = spawn_server(move |mut stream, head| {
            let segment = segment.clone();
            let media = media.clone();
            let master = master.clone();
            let master_pieces = master_pieces.clone();
            async move {
                let path = request_path(&head);
                if path.ends_with("master.m3u8") {
                    write_response_in_pieces(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        master.as_bytes(),
                        &master_pieces,
                    )
                    .await;
                } else if path.ends_with("media.m3u8") {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        media.as_bytes(),
                    )
                    .await;
                } else {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: video/mp2t\r\n",
                        &segment,
                    )
                    .await;
                }
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/master.m3u8"), &service, &cancel)
            .await
            .expect("master playlist must resolve to its variant regardless of chunking");
        assert!(matches!(source, Source::Hls(_)));
        count_access_units_until_end(&mut source).await
    }

    fn sample_media_playlist() -> String {
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
         #EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:10.0,\n/seg0.ts\n#EXT-X-ENDLIST\n"
            .to_string()
    }

    #[tokio::test]
    async fn a_master_playlist_split_immediately_after_extm3u_still_selects_and_decodes_the_variant(
    ) {
        let segment = fixture("aac-0.ts");
        let media = sample_media_playlist();
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\n\
                      /media.m3u8\n"
            .to_string();

        // Exactly the shape the review's harness reproduced finding 8 with:
        // the first chunk is nothing but the bare header.
        let units = run_master_scenario(&segment, &media, &master, &[8, master.len() - 8]).await;
        assert!(
            units > 0,
            "expected the selected variant's segment to be fetched and decoded"
        );
    }

    #[tokio::test]
    async fn master_variant_selection_is_invariant_to_where_the_body_is_split() {
        let segment = fixture("aac-0.ts");
        let media = sample_media_playlist();
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\n\
                      /media.m3u8\n"
            .to_string();

        let baseline = run_master_scenario(&segment, &media, &master, &[master.len()]).await;
        assert!(
            baseline > 0,
            "baseline (unsplit) run must itself decode audio"
        );

        let split_points = [
            1,                                     // mid "#EXTM3U"
            7,                                     // right at its trailing newline
            8,                                     // immediately after "#EXTM3U\n"
            master.find("STREAM").unwrap() + 3,    // inside the master tag name
            master.find("BANDWIDTH").unwrap() + 4, // inside an attribute name
            master.find("128000").unwrap() + 2,    // inside an attribute value
            master.len() - 3,                      // inside the variant URI
        ];
        for split in split_points {
            let units =
                run_master_scenario(&segment, &media, &master, &[split, master.len() - split])
                    .await;
            assert_eq!(
                units, baseline,
                "splitting the master body at byte {split} changed the decoded result"
            );
        }
    }

    #[tokio::test]
    async fn an_encryption_tag_arriving_after_the_first_chunk_is_still_rejected() {
        let head_piece = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n".to_string();
        let tail_piece = "#EXT-X-KEY:METHOD=AES-128,URI=\"https://example.invalid/key\"\n\
                           #EXTINF:2.0,\nseg0.ts\n#EXT-X-ENDLIST\n"
            .to_string();

        let (addr, _server) = spawn_server(move |mut stream, _head| {
            let head_piece = head_piece.clone();
            let tail_piece = tail_piece.clone();
            async move {
                let full = format!("{head_piece}{tail_piece}");
                write_response_in_pieces(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/vnd.apple.mpegurl\r\n",
                    full.as_bytes(),
                    &[head_piece.len(), tail_piece.len()],
                )
                .await;
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let error = open(&base_url(addr, "/stream.m3u8"), &service, &cancel)
            .await
            .expect_err(
                "an encrypted playlist must never be handed to HlsSource, even when \
                 #EXT-X-KEY arrives only in a later chunk",
            );

        assert!(matches!(error, SourceError::Unsupported(_)));
        assert!(
            error.to_string().to_lowercase().contains("encrypt"),
            "error should explain why: {error}"
        );
    }

    #[tokio::test]
    async fn a_master_playlists_selected_variant_carrying_encryption_is_rejected() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000\n/media.m3u8\n".to_string();
        let media = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
                      #EXT-X-KEY:METHOD=AES-128,URI=\"https://example.invalid/key\"\n\
                      #EXTINF:10.0,\nseg0.ts\n#EXT-X-ENDLIST\n"
            .to_string();

        let (addr, _server) = spawn_server(move |mut stream, head| {
            let master = master.clone();
            let media = media.clone();
            async move {
                let path = request_path(&head);
                if path.ends_with("master.m3u8") {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        master.as_bytes(),
                    )
                    .await;
                } else {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        media.as_bytes(),
                    )
                    .await;
                }
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let error = open(&base_url(addr, "/master.m3u8"), &service, &cancel)
            .await
            .expect_err(
                "a variant carrying #EXT-X-KEY must be rejected even though the master \
                 itself carried no encryption tag",
            );

        assert!(matches!(error, SourceError::Unsupported(_)));
        assert!(
            error.to_string().to_lowercase().contains("encrypt"),
            "error should explain why: {error}"
        );
    }

    #[tokio::test]
    async fn a_reload_that_adds_an_encryption_tag_is_rejected_even_though_the_initial_playlist_was_not(
    ) {
        let segment = fixture("aac-0.ts");
        let requests = Arc::new(AtomicUsize::new(0));

        let (addr, _server) = spawn_server(move |mut stream, head| {
            let requests = requests.clone();
            let segment = segment.clone();
            async move {
                let path = request_path(&head);
                if path.ends_with(".m3u8") {
                    let n = requests.fetch_add(1, Ordering::SeqCst);
                    let body = if n == 0 {
                        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                         #EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n"
                            .to_string()
                    } else {
                        // The origin adds encryption starting with this
                        // reload - the initial fetch carried none.
                        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                         #EXT-X-MEDIA-SEQUENCE:1\n\
                         #EXT-X-KEY:METHOD=AES-128,URI=\"https://example.invalid/key\"\n\
                         #EXTINF:2.0,\nseg1.ts\n"
                            .to_string()
                    };
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/vnd.apple.mpegurl\r\n",
                        body.as_bytes(),
                    )
                    .await;
                } else {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: video/mp2t\r\n",
                        &segment,
                    )
                    .await;
                }
            }
        });

        let service = default_service();
        let cancel = CancellationToken::new();
        let mut source = open(&base_url(addr, "/stream.m3u8"), &service, &cancel)
            .await
            .unwrap();

        let mut error = None;
        for _ in 0..2000 {
            match source.next_event().await {
                Ok(Some(_)) | Ok(None) => {}
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }

        let error =
            error.expect("a reload that adds #EXT-X-KEY must eventually surface as an error");
        assert!(matches!(error, SourceError::Unsupported(_)));
        assert!(
            error.to_string().to_lowercase().contains("encrypt"),
            "error should explain why: {error}"
        );
    }
}

#[cfg(test)]
mod status_error_tests {
    use super::*;

    #[test]
    fn a_server_reason_leads_the_message() {
        // The case that cost real debugging time: Dispatcharr answers a channel with no
        // upstream configured with a 503 whose body says exactly that, and reporting only
        // "503 Service Unavailable" hid it.
        let error = SourceError::Status {
            status: 503,
            detail: Some("No streams assigned to channel".to_string()),
        };
        let message = error.to_string();
        assert!(
            message.starts_with("No streams assigned to channel"),
            "the server's reason must lead: {message}"
        );
        assert!(
            message.contains("503"),
            "the status is still useful: {message}"
        );
    }

    #[test]
    fn a_bare_status_still_reads_sensibly() {
        let error = SourceError::Status {
            status: 502,
            detail: None,
        };
        assert_eq!(error.to_string(), "server returned HTTP 502");
    }

    #[test]
    fn a_credential_in_an_error_body_never_survives() {
        let error = SourceError::from(crate::network::NetworkError::Status {
            status: reqwest::StatusCode::FORBIDDEN,
            detail: Some("denied for http://host/live/user/hunter2/1.ts".to_string()),
        });
        let message = error.to_string();
        assert!(
            !message.contains("hunter2"),
            "leaked a credential: {message}"
        );
    }
}
