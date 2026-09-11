//! The application's HTTP layer (see `SYMPHONIA_PLAYBACK_PLAN.md` section 6
//! and `docs/symphonia-migration-progress.md`).
//!
//! [`NetworkService`] owns one reused [`reqwest::Client`] per traffic
//! profile (JSON API / artwork / continuous TS / HLS playlist / HLS
//! segment) because their timeout, body-bound, and concurrency policies
//! genuinely differ - reusing a single client/timeout for all of them would
//! be wrong for at least one of them. All clients share TLS (Rustls, never
//! disabled), proxy, and User-Agent construction via [`base_builder`].
//!
//! This is the only HTTP stack in the app. `tauri-plugin-http` has been
//! removed, and Xtream (`xtream.rs`), StellarTunerLog (`stellar.rs`),
//! Last.fm (`lastfm.rs`), notification artwork (`notifications.rs`), the
//! GitHub release list (`updater.rs`) and the playback engine all go through
//! here. Two instances exist and they are the same configuration: one managed
//! by Tauri for command handlers, and [`NetworkService::shared`] for callers
//! that have no `State` to thread through.
//!
//! The exceptions, which are framework-owned rather than application-owned:
//! remote channel artwork loaded by `<img>` tags in the webview, and the
//! updater plugin's own download of a release artifact.
//!
//! # The most important correctness requirement here
//!
//! The continuous-TS profile (`NetworkService::open_continuous_stream`)
//! deliberately never calls Reqwest's `.timeout()`. That method is a
//! *total* request timeout that covers the entire response body, and a live
//! radio stream can legitimately stay connected for hours. Only
//! `.connect_timeout()` (bounding the TCP/TLS handshake), our own bounded
//! wait for response headers after that (bounding a server that accepts the
//! connection and then never answers - `connect_timeout` alone cannot see
//! that), and our own inter-chunk stall detection in
//! [`ContinuousTsStream::next_chunk`] apply to this profile.
//!
//! # Redaction
//!
//! This app's stream URLs carry credentials in path segments, not query
//! strings or userinfo - the Xtream form is
//! `{baseUrl}/live/{username}/{password}/{streamId}.ts`. So
//! [`redact_url`] and [`redact_text`]/[`redact_error`] never assume
//! credentials live in any particular part of the URL; they redact the
//! entire path and query of anything logged, keeping only scheme/host/port.
//! [`redact_error`] additionally sanitizes the *rendered* Reqwest error
//! text, because `reqwest::Error`'s `Display` impl embeds the request URL
//! (see `for url (...)` in its formatter) and that string can reach a log
//! line without ever going through [`redact_url`] directly.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use reqwest::redirect::{Action, Attempt, Policy};
use reqwest::{Client, ClientBuilder, StatusCode, Url};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Sent on every request this service makes, across every profile.
const USER_AGENT: &str = concat!("Apogee/", env!("CARGO_PKG_VERSION"));

/// Redirect hops any profile will follow before giving up. Enforced by our
/// custom [`scheme_restricted_redirect_policy`] rather than
/// `Policy::limited`, because that policy cannot also enforce the scheme
/// restriction below.
const MAX_REDIRECTS: usize = 10;

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Error type for every [`NetworkService`] operation. Never wraps a raw
/// [`reqwest::Error`] directly - see module docs on why its `Display`
/// cannot be trusted un-redacted. [`NetworkError::Transport`] always holds
/// an already-redacted message.
#[derive(Debug, Clone)]
pub enum NetworkError {
    /// The URL (initial request or a redirect target) used a scheme other
    /// than `http`/`https`.
    DisallowedScheme(String),
    /// The URL/URL reference could not be parsed.
    InvalidUrl(String),
    /// The caller's [`CancellationToken`] fired while a request, a chunk
    /// read, a retry sleep, or a bounded-concurrency wait was pending.
    Cancelled,
    /// A response body exceeded the profile's configured byte bound.
    BodyTooLarge { limit: usize },
    /// The response status was not successful (2xx).
    /// An unsuccessful HTTP status, with any short error message the server sent.
    ///
    /// The body matters: a proxy in front of the provider answers a request for a channel
    /// it cannot serve with a 503 whose body says *why*, e.g. "No streams assigned to
    /// channel". Discarding that turns an actionable message into an opaque status code.
    Status {
        status: StatusCode,
        detail: Option<String>,
    },
    /// The continuous-TS profile detected no data for longer than its
    /// configured stall timeout, despite the connection never closing.
    Stalled,
    /// The continuous-TS profile's initial request never received response
    /// headers before `continuous_ts_response_timeout` elapsed. Distinct
    /// from [`Self::Stalled`], which applies to gaps between chunks of an
    /// already-open body: this is a server that accepts the TCP/TLS
    /// connection (so `connect_timeout` is satisfied) and then never
    /// answers at all.
    ResponseTimedOut,
    /// Client construction or another Reqwest failure. Always pre-redacted;
    /// see [`redact_error`].
    Transport(String),
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisallowedScheme(scheme) => write!(f, "disallowed URL scheme {scheme:?}"),
            Self::InvalidUrl(message) => write!(f, "invalid URL: {message}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::BodyTooLarge { limit } => write!(f, "response body exceeded {limit} bytes"),
            Self::Status { status, detail } => match detail {
                Some(detail) => write!(f, "{detail} (HTTP {})", status.as_u16()),
                None => write!(f, "unsuccessful response status {status}"),
            },
            Self::Stalled => f.write_str("no data received before the stall timeout"),
            Self::ResponseTimedOut => {
                f.write_str("no response headers received before the timeout")
            }
            Self::Transport(message) => write!(f, "network error: {message}"),
        }
    }
}

impl std::error::Error for NetworkError {}

/// A custom [`reqwest::redirect`] policy error for a disallowed redirect
/// target scheme. Kept distinct from [`NetworkError`] because
/// `Policy::custom`'s closure must produce a boxed `std::error::Error`, not
/// our own error type, and because the resulting `reqwest::Error`'s
/// `Display` (redacted via [`redact_error`] before it reaches a caller)
/// should name the offending scheme.
#[derive(Debug)]
struct DisallowedRedirectScheme {
    scheme: String,
}

impl fmt::Display for DisallowedRedirectScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "redirect used disallowed URL scheme {:?}", self.scheme)
    }
}

impl std::error::Error for DisallowedRedirectScheme {}

// ---------------------------------------------------------------------
// Scheme restriction (initial URL and redirects)
// ---------------------------------------------------------------------

fn is_allowed_scheme(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Parses `raw` and rejects any scheme other than http/https. Deliberately
/// does not restrict host/port - local and private provider addresses (a
/// LAN Xtream panel, `127.0.0.1`, ...) must keep working; only the scheme is
/// restricted here.
fn parse_allowed_url(raw: &str) -> Result<Url, NetworkError> {
    let url = Url::parse(raw).map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
    if !is_allowed_scheme(&url) {
        return Err(NetworkError::DisallowedScheme(url.scheme().to_string()));
    }
    Ok(url)
}

/// Resolves `reference` (which may be relative or absolute) against
/// `base`, the *effective* URL of a previous response - i.e. after
/// redirects, per [`FetchedBody::final_url`] / [`ContinuousTsStream::final_url`].
/// Re-validates the resolved scheme, since an absolute reference can name
/// any scheme regardless of `base`.
pub fn resolve_relative(base: &Url, reference: &str) -> Result<Url, NetworkError> {
    let resolved = base
        .join(reference)
        .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
    if !is_allowed_scheme(&resolved) {
        return Err(NetworkError::DisallowedScheme(
            resolved.scheme().to_string(),
        ));
    }
    Ok(resolved)
}

/// Redirect policy shared by every profile: only follow http(s) redirect
/// targets, and cap the chain length (the custom-policy branch of
/// `redirect::Policy` does not do this automatically - see its docs).
fn scheme_restricted_redirect_policy() -> Policy {
    Policy::custom(|attempt: Attempt| -> Action {
        let scheme = attempt.url().scheme().to_string();
        if !matches!(scheme.as_str(), "http" | "https") {
            return attempt.error(DisallowedRedirectScheme { scheme });
        }
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.stop();
        }
        attempt.follow()
    })
}

// ---------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------

/// Redacts a URL for logging: keeps scheme/host/port (useful to identify
/// *which* provider a log line is about) and replaces the entire path and
/// query with a placeholder, since this app's credentials live in path
/// segments (see module docs) and cannot be distinguished generically from
/// any other segment.
pub fn redact_url(url: &Url) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = write!(out, "{}://", url.scheme());
    match url.host_str() {
        Some(host) => out.push_str(host),
        None => out.push_str("<no-host>"),
    }
    if let Some(port) = url.port() {
        let _ = write!(out, ":{port}");
    }
    let segment_count = url.path_segments().map(Iterator::count).unwrap_or(0);
    if segment_count > 0 {
        let _ = write!(out, "/<redacted:{segment_count}-segment-path>");
    }
    if url.query().is_some() {
        out.push_str("?<redacted-query>");
    }
    out
}

fn find_url_start(text: &str) -> Option<usize> {
    [text.find("http://"), text.find("https://")]
        .into_iter()
        .flatten()
        .min()
}

/// A URL embedded in free text ends at the first character that could not
/// be part of it in the contexts this is used for (error `Display` output,
/// log lines): whitespace or common surrounding punctuation.
fn url_span_end(candidate: &str) -> usize {
    candidate
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | ','))
        .unwrap_or(candidate.len())
}

/// Redacts every `http://`/`https://` URL found anywhere inside free text,
/// using [`redact_url`] when the span parses as a URL and a fixed
/// placeholder otherwise. This is what makes [`redact_error`] safe: Reqwest
/// error `Display` strings embed the request URL as *text*
/// (`"... for url (https://host/live/user/pass/1.ts)"`), not as a
/// separately-accessible field callers reliably use, so logging
/// `error.to_string()` (or any other free text that happens to mention a
/// URL) must go through this rather than [`redact_url`] alone.
pub fn redact_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let Some(start) = find_url_start(rest) else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let end = url_span_end(candidate);
        let span = &candidate[..end];
        match Url::parse(span) {
            Ok(url) => output.push_str(&redact_url(&url)),
            Err(_) => output.push_str("<redacted-url>"),
        }
        rest = &candidate[end..];
    }
    output
}

/// Redacts a Reqwest error for logging. See module docs and [`redact_text`].
///
/// Walks the error's `source()` chain (not just its own `Display`) before
/// redacting: Reqwest's top-level `Display` for some error kinds - a
/// redirect rejected by our own [`scheme_restricted_redirect_policy`], for
/// example - only says e.g. "error following redirect for url (...)" and
/// leaves the actually useful detail (which scheme, which URL) in the
/// wrapped source error. Both still need redaction, so both are included.
pub fn redact_error(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    redact_text(&message)
}

fn transport_error(error: reqwest::Error) -> NetworkError {
    NetworkError::Transport(redact_error(&error))
}

// ---------------------------------------------------------------------
// Cancellable waits
// ---------------------------------------------------------------------

/// Sleeps for `duration`, or returns [`NetworkError::Cancelled`] promptly if
/// `cancel` fires first. Every retry sleep built on top of
/// [`NetworkService`] should use this rather than a bare `tokio::time::sleep`.
pub async fn cancellable_sleep(
    duration: Duration,
    cancel: &CancellationToken,
) -> Result<(), NetworkError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(NetworkError::Cancelled),
        () = tokio::time::sleep(duration) => Ok(()),
    }
}

/// Only diagnostic response fields: never dump cookies, authorization, or whole headers.
fn stellar_response_headers(
    headers: &reqwest::header::HeaderMap,
    request_headers: &[(&str, &str)],
) -> String {
    [
        "cf-ray",
        "server",
        "date",
        "content-type",
        "content-length",
        "content-encoding",
        "cf-cache-status",
        "age",
        "retry-after",
        "ratelimit-limit",
        "ratelimit-remaining",
        "ratelimit-reset",
        "x-ratelimit-limit",
        "x-ratelimit-remaining",
        "x-ratelimit-reset",
        "cf-error-type",
        "cf-error-origin",
    ]
    .iter()
    .filter_map(|name| {
        let value = headers.get(*name)?.to_str().ok()?;
        Some(format!(
            "{name}={}",
            diagnostic_text(value, request_headers)
        ))
    })
    .collect::<Vec<_>>()
    .join("; ")
}

fn diagnostic_text(text: &str, request_headers: &[(&str, &str)]) -> String {
    let mut safe = text.to_owned();
    for (name, value) in request_headers {
        if name.eq_ignore_ascii_case("x-api-key") && !value.is_empty() {
            safe = safe.replace(value, "[redacted]");
        }
    }
    redact_text(&safe)
        .chars()
        .take(512)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

// ---------------------------------------------------------------------
// Client construction
// ---------------------------------------------------------------------

fn base_builder() -> ClientBuilder {
    // Rustls (the only TLS backend compiled in, per Cargo.toml's
    // `rustls-tls` feature with `default-features = false`) with its
    // default verifier - certificate verification is never disabled here.
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(scheme_restricted_redirect_policy())
}

fn build_client(builder: ClientBuilder) -> Result<Client, NetworkError> {
    builder.build().map_err(transport_error)
}

// ---------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------

/// Tunable policy values for every profile. [`NetworkServiceConfig::default`]
/// uses the migration plan's stated initial defaults where the plan gives
/// one (the 20-second connect-to-play timeout for direct TS); the rest are
/// this module's own proposed starting points, not measured provider
/// behavior, and are expected to be tuned once M2/M3 exercise them against
/// a real provider.
#[derive(Debug, Clone)]
pub struct NetworkServiceConfig {
    pub json_api_timeout: Duration,
    pub json_api_max_body_bytes: usize,
    pub artwork_timeout: Duration,
    pub artwork_max_body_bytes: usize,
    /// Bounds only the TCP/TLS connect phase. See module docs: this profile
    /// intentionally has no total request timeout.
    pub continuous_ts_connect_timeout: Duration,
    /// Bounds the wait for response headers *after* the TCP/TLS connection
    /// is established, i.e. the case `continuous_ts_connect_timeout` cannot
    /// cover: a server that accepts the connection and then never answers.
    /// Still not a total-body timeout - once headers arrive this no longer
    /// applies (see module docs on why the body itself must stay unbounded).
    pub continuous_ts_response_timeout: Duration,
    /// Maximum gap allowed between two chunks of a continuous-TS body
    /// before [`ContinuousTsStream::next_chunk`] reports
    /// [`NetworkError::Stalled`].
    pub continuous_ts_stall_timeout: Duration,
    pub hls_playlist_timeout: Duration,
    pub hls_playlist_max_body_bytes: usize,
    pub hls_segment_timeout: Duration,
    pub hls_segment_max_body_bytes: usize,
    /// Caps concurrent in-flight HLS segment fetches so a live playlist
    /// with many renditions/segments cannot grow unbounded memory/socket
    /// usage.
    pub hls_segment_max_concurrency: usize,
}

impl Default for NetworkServiceConfig {
    fn default() -> Self {
        Self {
            json_api_timeout: Duration::from_secs(10),
            json_api_max_body_bytes: 2 * 1024 * 1024,
            artwork_timeout: Duration::from_secs(15),
            // Matches the cap the notification artwork cache enforced before it moved
            // here; artwork is thumbnail-sized, and a larger body is a sign of trouble.
            artwork_max_body_bytes: 3 * 1024 * 1024,
            // Matches the plan's documented current direct-TS
            // connect-to-play default.
            continuous_ts_connect_timeout: Duration::from_secs(20),
            continuous_ts_response_timeout: Duration::from_secs(20),
            continuous_ts_stall_timeout: Duration::from_secs(15),
            hls_playlist_timeout: Duration::from_secs(10),
            hls_playlist_max_body_bytes: 512 * 1024,
            hls_segment_timeout: Duration::from_secs(20),
            hls_segment_max_body_bytes: 4 * 1024 * 1024,
            hls_segment_max_concurrency: 4,
        }
    }
}

// ---------------------------------------------------------------------
// NetworkService
// ---------------------------------------------------------------------

/// Owns one reused [`reqwest::Client`] per traffic profile. Cheap to clone
/// (all fields are already `Arc`/`Client`-internal-`Arc`-backed); construct
/// once and share it.
#[derive(Clone)]
pub struct NetworkService {
    json_api: Client,
    json_api_max_body_bytes: usize,
    artwork: Client,
    artwork_max_body_bytes: usize,
    continuous_ts: Client,
    continuous_ts_response_timeout: Duration,
    continuous_ts_stall_timeout: Duration,
    hls_playlist: Client,
    hls_playlist_max_body_bytes: usize,
    hls_segment: Client,
    hls_segment_max_body_bytes: usize,
    hls_segment_semaphore: Arc<Semaphore>,
}

/// A fully-read, bounded response body plus the metadata callers need:
/// effective URL after redirects (see [`resolve_relative`]), status, and
/// content type.
#[derive(Debug, Clone)]
pub struct FetchedBody {
    pub final_url: Url,
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// Process-wide instance.
///
/// Tauri also manages a clone for command handlers, but Last.fm and notification artwork
/// are reached from places that have no `State` to thread through. Both refer to this, so
/// there is genuinely one HTTP layer rather than one per caller.
static SHARED: std::sync::OnceLock<NetworkService> = std::sync::OnceLock::new();

impl NetworkService {
    /// The shared instance, created on first use.
    ///
    /// Falls back to a default-configured service if construction ever fails, because
    /// losing artwork or scrobbling is better than panicking during playback.
    pub fn shared() -> &'static NetworkService {
        SHARED.get_or_init(|| {
            Self::new().unwrap_or_else(|e| {
                log::warn!("network service fell back to defaults: {e}");
                Self::with_config(NetworkServiceConfig::default())
                    .expect("a default network service must be constructible")
            })
        })
    }

    /// POSTs form-encoded parameters and reads a bounded JSON response.
    ///
    /// Last.fm's API is form-encoded and signed, so it cannot use the GET path.
    pub async fn post_form(
        &self,
        url: &str,
        form: &(impl serde::Serialize + ?Sized),
        cancel: &CancellationToken,
    ) -> Result<FetchedBody, NetworkError> {
        let parsed = parse_allowed_url(url)?;
        let request = self.json_api.post(parsed).form(form);
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(NetworkError::Cancelled),
            result = request.send() => result.map_err(transport_error)?,
        };
        // Last.fm signals API errors in the body with a non-2xx status, so the body must be
        // read even when the status is unsuccessful.
        let status = response.status();
        let body =
            read_bounded_allowing_error_status(response, self.json_api_max_body_bytes, cancel)
                .await?;
        Ok(FetchedBody { status, ..body })
    }

    pub fn new() -> Result<Self, NetworkError> {
        Self::with_config(NetworkServiceConfig::default())
    }

    pub fn with_config(config: NetworkServiceConfig) -> Result<Self, NetworkError> {
        let json_api = build_client(base_builder().timeout(config.json_api_timeout))?;
        let artwork = build_client(base_builder().timeout(config.artwork_timeout))?;
        // No `.timeout()` call for this client - see module docs. Only the
        // connect phase gets a deadline; the running body is governed by
        // per-chunk stall detection in `ContinuousTsStream::next_chunk`.
        let continuous_ts =
            build_client(base_builder().connect_timeout(config.continuous_ts_connect_timeout))?;
        let hls_playlist = build_client(base_builder().timeout(config.hls_playlist_timeout))?;
        let hls_segment = build_client(base_builder().timeout(config.hls_segment_timeout))?;

        Ok(Self {
            json_api,
            json_api_max_body_bytes: config.json_api_max_body_bytes,
            artwork,
            artwork_max_body_bytes: config.artwork_max_body_bytes,
            continuous_ts,
            continuous_ts_response_timeout: config.continuous_ts_response_timeout,
            continuous_ts_stall_timeout: config.continuous_ts_stall_timeout,
            hls_playlist,
            hls_playlist_max_body_bytes: config.hls_playlist_max_body_bytes,
            hls_segment,
            hls_segment_max_body_bytes: config.hls_segment_max_body_bytes,
            hls_segment_semaphore: Arc::new(Semaphore::new(
                config.hls_segment_max_concurrency.max(1),
            )),
        })
    }

    /// JSON API profile: finite overall deadline (the client-level
    /// `.timeout()` set in [`Self::with_config`]), bounded body, clear
    /// status errors.
    pub async fn fetch_json(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<FetchedBody, NetworkError> {
        self.fetch_json_with_headers(url, &[], cancel).await
    }

    /// JSON fetch carrying extra request headers.
    ///
    /// Separate from [`Self::fetch_json`] because header values can themselves be
    /// credentials (StellarTunerLog's `X-API-Key`), and they must never reach a log or an
    /// error message. Nothing here formats the header map.
    pub async fn fetch_json_with_headers(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        cancel: &CancellationToken,
    ) -> Result<FetchedBody, NetworkError> {
        let parsed = parse_allowed_url(url)?;
        // Diagnose this API without exposing provider URLs or arbitrary request headers.
        let stellar = parsed.host_str() == Some("api.stellartunerlog.com");
        static REQUEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let request_id = REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let started = std::time::Instant::now();
        if stellar {
            log::debug!(
                "stellar HTTP request id={request_id}: method=GET endpoint={}://{}{} query_present={} user_agent={USER_AGENT} api_key_present={}",
                parsed.scheme(), parsed.host_str().unwrap_or(""), parsed.path(),
                parsed.query().is_some(),
                headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("x-api-key") && !value.is_empty()),
            );
        }
        let mut request = self.json_api.get(parsed);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(NetworkError::Cancelled),
            result = request.send() => result.map_err(transport_error),
        };
        let response = response.map_err(|error| {
            if stellar {
                log::debug!(
                    "stellar HTTP transport failure id={request_id}: elapsed_ms={} error={}",
                    started.elapsed().as_millis(),
                    diagnostic_text(&error.to_string(), headers)
                );
            }
            error
        })?;
        if stellar {
            log::debug!(
                "stellar HTTP response id={request_id}: status={} protocol={:?} headers_ms={} final_origin={} final_path={} headers={}",
                response.status().as_u16(), response.version(), started.elapsed().as_millis(),
                redact_url(response.url()),
                if response.url().host_str() == Some("api.stellartunerlog.com") { response.url().path() } else { "[redacted]" },
                stellar_response_headers(response.headers(), headers),
            );
        }
        let result = read_bounded(response, self.json_api_max_body_bytes, cancel).await;
        if stellar {
            match &result {
                Ok(body) => log::debug!(
                    "stellar HTTP complete id={request_id}: elapsed_ms={} body_bytes={}",
                    started.elapsed().as_millis(),
                    body.bytes.len()
                ),
                Err(error) => log::debug!(
                    "stellar HTTP failed id={request_id}: elapsed_ms={} error={}",
                    started.elapsed().as_millis(),
                    diagnostic_text(&error.to_string(), headers)
                ),
            }
        }
        result
    }

    /// Artwork profile: finite deadline, byte limit. Content *validation*
    /// (recognized image type) stays with the caller for now - this
    /// milestone provides the transport only; `notifications.rs` keeps its
    /// own artwork client until artwork is migrated (see migration
    /// progress log).
    pub async fn fetch_artwork(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<FetchedBody, NetworkError> {
        fetch_bounded(&self.artwork, url, self.artwork_max_body_bytes, cancel).await
    }

    /// HLS playlist profile: finite deadline, bounded body. Playlists are
    /// refetched repeatedly for live data, so callers should treat every
    /// call as a fresh fetch (no caching here).
    pub async fn fetch_hls_playlist(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<FetchedBody, NetworkError> {
        fetch_bounded(
            &self.hls_playlist,
            url,
            self.hls_playlist_max_body_bytes,
            cancel,
        )
        .await
    }

    /// HLS segment profile: finite deadline, bounded body, and bounded
    /// *concurrency* across all in-flight segment fetches on this service.
    /// The concurrency wait itself is cancellable.
    pub async fn fetch_hls_segment(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<FetchedBody, NetworkError> {
        let semaphore = self.hls_segment_semaphore.clone();
        let _permit = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(NetworkError::Cancelled),
            permit = semaphore.acquire_owned() => {
                // The semaphore is only ever closed by `Semaphore::close`,
                // which this service never calls, so `acquire_owned` should
                // not fail in practice; treat it as cancellation if it ever
                // does rather than panicking.
                permit.map_err(|_| NetworkError::Cancelled)?
            }
        };
        fetch_bounded(
            &self.hls_segment,
            url,
            self.hls_segment_max_body_bytes,
            cancel,
        )
        .await
    }

    /// Continuous-TS profile: opens a streaming GET with a connect deadline
    /// but **no total lifetime timeout** (see module docs). Returns once
    /// headers arrive and the status is successful; read the body via
    /// [`ContinuousTsStream::next_chunk`], which applies stall detection to
    /// each read.
    pub async fn open_continuous_stream(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<ContinuousTsStream, NetworkError> {
        let parsed = parse_allowed_url(url)?;
        let request = self.continuous_ts.get(parsed);
        // `connect_timeout` (set on this client, see `with_config`) only bounds the
        // TCP/TLS handshake - it says nothing about a server that accepts the connection
        // and then never answers. That case is otherwise unbounded here (deliberately, per
        // module docs, for the body once headers arrive), so the wait for headers alone
        // gets its own finite deadline.
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(NetworkError::Cancelled),
            outcome = tokio::time::timeout(self.continuous_ts_response_timeout, request.send()) => {
                match outcome {
                    Ok(result) => result.map_err(transport_error)?,
                    Err(_elapsed) => return Err(NetworkError::ResponseTimedOut),
                }
            }
        };
        let status = response.status();
        if !status.is_success() {
            return Err(NetworkError::Status {
                status,
                detail: error_detail(response).await,
            });
        }
        Ok(ContinuousTsStream {
            response,
            stall_timeout: self.continuous_ts_stall_timeout,
            cancel: cancel.clone(),
        })
    }
}

/// An open continuous-TS body. See [`NetworkService::open_continuous_stream`].
pub struct ContinuousTsStream {
    response: reqwest::Response,
    stall_timeout: Duration,
    cancel: CancellationToken,
}

impl ContinuousTsStream {
    /// The effective URL after any redirects, for resolving/logging.
    pub fn final_url(&self) -> &Url {
        self.response.url()
    }

    pub fn status(&self) -> StatusCode {
        self.response.status()
    }

    /// Waits for the next chunk. `Ok(None)` means the server closed the
    /// body normally (end of stream) - that is a real, if unusual, outcome
    /// for a live stream and is distinct from a stall. Applies stall
    /// detection to this read only; there is deliberately no bound on how
    /// many times this may be called or how long the stream may run for in
    /// total (see module docs).
    pub async fn next_chunk(&mut self) -> Result<Option<Bytes>, NetworkError> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(NetworkError::Cancelled),
            outcome = tokio::time::timeout(self.stall_timeout, self.response.chunk()) => {
                match outcome {
                    Ok(Ok(chunk)) => Ok(chunk),
                    Ok(Err(error)) => Err(transport_error(error)),
                    Err(_elapsed) => Err(NetworkError::Stalled),
                }
            }
        }
    }
}

async fn fetch_bounded(
    client: &Client,
    url: &str,
    max_bytes: usize,
    cancel: &CancellationToken,
) -> Result<FetchedBody, NetworkError> {
    let parsed = parse_allowed_url(url)?;
    let request = client.get(parsed);
    let response = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(NetworkError::Cancelled),
        result = request.send() => result.map_err(transport_error)?,
    };
    read_bounded(response, max_bytes, cancel).await
}

/// Reads a response body with a hard byte ceiling, cancellable throughout.
async fn read_bounded(
    response: reqwest::Response,
    max_bytes: usize,
    cancel: &CancellationToken,
) -> Result<FetchedBody, NetworkError> {
    let status = response.status();
    if !status.is_success() {
        return Err(NetworkError::Status {
            status,
            detail: error_detail(response).await,
        });
    }
    read_bounded_allowing_error_status(response, max_bytes, cancel).await
}

/// Extracts a short, human-readable reason from an error response body.
///
/// Deliberately bounded and best-effort: this runs on a failure path, so it must not block
/// for long or allocate much. JSON `error`/`message` fields are unwrapped because that is
/// how the proxies in front of these streams report their reason; anything else falls back
/// to a trimmed prefix of the body. Always redacted, since a body can echo the request URL.
async fn error_detail(response: reqwest::Response) -> Option<String> {
    const MAX_DETAIL_BYTES: usize = 512;

    let bytes = tokio::time::timeout(Duration::from_secs(2), response.bytes())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_DETAIL_BYTES)]);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let message = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            ["error", "message", "detail"]
                .iter()
                .find_map(|key| value.get(*key)?.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| text.chars().take(200).collect());

    let message = redact_text(message.trim());
    if message.is_empty() {
        None
    } else {
        Some(message)
    }
}

/// As [`read_bounded`], but returns the body even for an unsuccessful status.
///
/// Some APIs put their real error detail in the body of a non-2xx response; discarding it
/// would turn an actionable message into a bare status code.
async fn read_bounded_allowing_error_status(
    mut response: reqwest::Response,
    max_bytes: usize,
    cancel: &CancellationToken,
) -> Result<FetchedBody, NetworkError> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(NetworkError::BodyTooLarge { limit: max_bytes });
    }

    let final_url = response.url().clone();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(NetworkError::Cancelled),
            result = response.chunk() => result.map_err(transport_error)?,
        };
        match chunk {
            Some(chunk) => {
                if bytes.len().saturating_add(chunk.len()) > max_bytes {
                    return Err(NetworkError::BodyTooLarge { limit: max_bytes });
                }
                bytes.extend_from_slice(&chunk);
            }
            None => break,
        }
    }

    Ok(FetchedBody {
        final_url,
        status,
        content_type,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[test]
    fn stellar_diagnostics_allowlist_headers_and_redact_echoed_keys() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("cf-ray", "test-ray-IAD".parse().unwrap());
        headers.insert("server", "cloudflare".parse().unwrap());
        headers.insert("retry-after", "30".parse().unwrap());
        headers.insert("set-cookie", "private-cookie".parse().unwrap());
        headers.insert("x-api-key", "private-key".parse().unwrap());
        headers.insert("cf-error-type", "echo private-key".parse().unwrap());
        let output = stellar_response_headers(&headers, &[("X-API-Key", "private-key")]);
        assert!(output.contains("cf-ray=test-ray-IAD"));
        assert!(output.contains("server=cloudflare"));
        assert!(output.contains("retry-after=30"));
        assert!(output.contains("echo [redacted]"));
        assert!(!output.contains("private-key"));
        assert!(!output.contains("private-cookie"));
        assert!(!output.contains("set-cookie"));
    }

    #[test]
    fn stellar_diagnostic_errors_are_bounded_and_cannot_inject_log_lines() {
        let output = diagnostic_text(
            &format!(
                "key=private-key\nhttps://example.com/private/path?token=secret {}",
                "x".repeat(1000)
            ),
            &[("x-api-key", "private-key")],
        );
        assert!(output.chars().count() <= 512);
        assert!(!output.contains("private-key"));
        assert!(!output.contains("/private/path"));
        assert!(!output.contains("token=secret"));
        assert!(!output.contains('\n'));
    }

    // -----------------------------------------------------------------
    // Pure unit tests: redaction, scheme/URL validation
    // -----------------------------------------------------------------

    #[test]
    fn redact_url_strips_credential_path_segments() {
        let url =
            Url::parse("http://provider.example:8080/live/produser/sup3rSecret/42.ts").unwrap();
        let redacted = redact_url(&url);
        assert!(!redacted.contains("sup3rSecret"));
        assert!(!redacted.contains("produser"));
        assert!(redacted.contains("provider.example"));
        assert!(redacted.contains("8080"));
    }

    #[test]
    fn redact_text_finds_and_redacts_embedded_urls() {
        let text = "error sending request for url (http://host/live/produser/sup3rSecret/1.ts)";
        let redacted = redact_text(text);
        assert!(!redacted.contains("sup3rSecret"));
        assert!(!redacted.contains("produser"));
        assert!(redacted.contains("error sending request for url"));
    }

    #[tokio::test]
    async fn redact_error_removes_a_credential_from_a_real_reqwest_error_display() {
        // Port 0 is never listening, so this fails fast with a
        // connection-refused error that (per Reqwest's Display impl, see
        // module docs) embeds the request URL as text.
        let client = reqwest::Client::new();
        let error = client
            .get("http://127.0.0.1:1/live/produser/sup3rSecret/1.ts")
            .send()
            .await
            .expect_err("port 1 should refuse the connection");

        let raw = error.to_string();
        assert!(
            raw.contains("sup3rSecret"),
            "test assumption failed: reqwest's error Display did not embed the URL \
             (got: {raw:?}); rewrite this test against actual Reqwest 0.12.28 behavior"
        );

        let redacted = redact_error(&error);
        assert!(!redacted.contains("sup3rSecret"));
        assert!(!redacted.contains("produser"));
    }

    #[test]
    fn parse_allowed_url_rejects_non_http_schemes() {
        assert!(matches!(
            parse_allowed_url("file:///etc/passwd"),
            Err(NetworkError::DisallowedScheme(scheme)) if scheme == "file"
        ));
        assert!(parse_allowed_url("https://example.com/x.m3u8").is_ok());
        assert!(parse_allowed_url("http://127.0.0.1:8080/live/1.ts").is_ok());
    }

    #[test]
    fn resolve_relative_rejects_a_non_http_absolute_reference() {
        let base = Url::parse("https://example.com/live/stream.m3u8").unwrap();
        assert!(matches!(
            resolve_relative(&base, "file:///etc/passwd"),
            Err(NetworkError::DisallowedScheme(scheme)) if scheme == "file"
        ));
        let resolved = resolve_relative(&base, "seg-2.ts").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/live/seg-2.ts");
    }

    // -----------------------------------------------------------------
    // Minimal hand-rolled HTTP/1.1 test server.
    //
    // Deliberately not a mocking library: each test controls exact bytes
    // and timing on the wire (delayed/partial/never-closed bodies,
    // malformed redirects) so behavior is proven against real sockets and
    // real Reqwest parsing, not a canned "always succeeds" stub.
    // -----------------------------------------------------------------

    /// Reads (and discards) one HTTP request line + headers from `stream`.
    async fn read_request_head(stream: &mut TcpStream) {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if stream.read_exact(&mut byte).await.is_err() {
                return;
            }
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                return;
            }
        }
    }

    /// Spawns a TCP server on an ephemeral localhost port. `handler` runs
    /// once per accepted connection after the request head has already
    /// been read, and is responsible for writing a full raw HTTP response.
    fn spawn_server<F, Fut>(handler: F) -> (SocketAddr, tokio::task::JoinHandle<()>)
    where
        F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
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
                    read_request_head(&mut stream).await;
                    handler(stream).await;
                });
            }
        });
        (addr, join)
    }

    fn base_url(addr: SocketAddr, path: &str) -> String {
        format!("http://{addr}{path}")
    }

    // -----------------------------------------------------------------
    // Scheme restriction on redirect targets
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn scheme_restriction_rejects_a_redirect_to_a_non_http_scheme() {
        let (addr, _server) = spawn_server(|mut stream: TcpStream| async move {
            let response = b"HTTP/1.1 302 Found\r\nLocation: ftp://evil.example/data\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response).await;
        });

        let service = NetworkService::new().unwrap();
        let cancel = CancellationToken::new();
        let result = service
            .fetch_json(&base_url(addr, "/redirect"), &cancel)
            .await;

        match result {
            Err(NetworkError::Transport(message)) => {
                assert!(
                    message.contains("error following redirect"),
                    "unexpected transport error: {message}"
                );
                assert!(
                    message.contains("disallowed URL scheme") && message.contains("ftp"),
                    "redirect rejection reason (from the source error chain) was lost: {message}"
                );
            }
            other => panic!("expected a disallowed-scheme transport error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Body bounds
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn body_bounds_reject_an_oversized_body() {
        let body = vec![b'a'; 64 * 1024];
        let (addr, _server) = spawn_server(move |mut stream: TcpStream| {
            let body = body.clone();
            async move {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });

        let config = NetworkServiceConfig {
            json_api_max_body_bytes: 1024,
            ..NetworkServiceConfig::default()
        };
        let service = NetworkService::with_config(config).unwrap();
        let cancel = CancellationToken::new();
        let result = service.fetch_json(&base_url(addr, "/big"), &cancel).await;

        assert!(
            matches!(result, Err(NetworkError::BodyTooLarge { limit: 1024 })),
            "expected BodyTooLarge, got {result:?}"
        );
    }

    #[tokio::test]
    async fn body_bounds_accept_a_body_within_the_limit() {
        let body = b"hello world".to_vec();
        let (addr, _server) = spawn_server(move |mut stream: TcpStream| {
            let body = body.clone();
            async move {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });

        let service = NetworkService::new().unwrap();
        let cancel = CancellationToken::new();
        let fetched = service
            .fetch_json(&base_url(addr, "/small"), &cancel)
            .await
            .unwrap();

        assert_eq!(fetched.bytes, b"hello world");
        assert_eq!(fetched.status, StatusCode::OK);
        assert_eq!(fetched.content_type.as_deref(), Some("text/plain"));
    }

    // -----------------------------------------------------------------
    // Cancellation
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn cancellation_while_waiting_on_a_slow_response_returns_promptly() {
        let (addr, _server) = spawn_server(|mut stream: TcpStream| async move {
            // Never responds before the test's cancellation fires; holds
            // the connection open well past a reasonable test timeout to
            // prove cancellation - not a fast server response - is what
            // ends the wait.
            tokio::time::sleep(Duration::from_secs(30)).await;
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        });

        let service = NetworkService::new().unwrap();
        let cancel = CancellationToken::new();
        let cancel_trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_trigger.cancel();
        });

        let started = Instant::now();
        let result = service.fetch_json(&base_url(addr, "/slow"), &cancel).await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(NetworkError::Cancelled)));
        assert!(
            elapsed < Duration::from_secs(2),
            "cancellation did not return promptly: took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn cancellable_sleep_returns_promptly_when_cancelled() {
        let cancel = CancellationToken::new();
        let cancel_trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_trigger.cancel();
        });

        let started = Instant::now();
        let result = cancellable_sleep(Duration::from_secs(30), &cancel).await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(NetworkError::Cancelled)));
        assert!(elapsed < Duration::from_secs(1));
    }

    // -----------------------------------------------------------------
    // Continuous-TS profile: no total lifetime timeout, but stalls ARE
    // detected.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn continuous_ts_profile_does_not_terminate_a_slow_but_alive_body() {
        let (addr, _server) = spawn_server(|mut stream: TcpStream| async move {
            let header = b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = stream.write_all(header).await;
            // 8 chunks, 200ms apart (each gap well under the stall
            // timeout below) for a total of ~1.6s - deliberately longer
            // than `json_api_timeout` configured on the very same service
            // below, to prove this profile is not sharing that deadline.
            for _ in 0..8 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let chunk = b"tsdata";
                let framed = format!("{:x}\r\n", chunk.len());
                if stream.write_all(framed.as_bytes()).await.is_err() {
                    return;
                }
                if stream.write_all(chunk).await.is_err() {
                    return;
                }
                if stream.write_all(b"\r\n").await.is_err() {
                    return;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        });

        let config = NetworkServiceConfig {
            // A finite-deadline profile with a much shorter budget than
            // the continuous body below will actually take - proves the
            // continuous profile is not reusing this (or any) total
            // deadline.
            json_api_timeout: Duration::from_millis(800),
            continuous_ts_stall_timeout: Duration::from_millis(700),
            ..NetworkServiceConfig::default()
        };
        let service = NetworkService::with_config(config).unwrap();
        let cancel = CancellationToken::new();

        let started = Instant::now();
        let mut stream = service
            .open_continuous_stream(&base_url(addr, "/live.ts"), &cancel)
            .await
            .unwrap();

        let mut total = Vec::new();
        while let Some(chunk) = stream.next_chunk().await.unwrap() {
            total.extend_from_slice(&chunk);
        }
        let elapsed = started.elapsed();

        assert_eq!(
            total,
            b"tsdatatsdatatsdatatsdatatsdatatsdatatsdatatsdata".to_vec()
        );
        assert!(
            elapsed >= Duration::from_millis(1400),
            "expected the full slow body to be read (~1.6s), got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn continuous_ts_profile_bounds_the_wait_for_response_headers() {
        // Accepts the TCP connection (so `connect_timeout` is satisfied) and then never
        // writes a single byte - the case `connect_timeout` cannot cover.
        let (addr, _server) = spawn_server(|stream: TcpStream| async move {
            // Hold the connection open (never read or write another byte) for the whole
            // sleep. `stream` must be kept alive here, not just accepted and implicitly
            // dropped - dropping it would close the connection immediately and produce a
            // connection-reset error rather than the never-answers scenario under test.
            let _stream = stream;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let config = NetworkServiceConfig {
            continuous_ts_response_timeout: Duration::from_millis(300),
            ..NetworkServiceConfig::default()
        };
        let service = NetworkService::with_config(config).unwrap();
        let cancel = CancellationToken::new();

        let started = Instant::now();
        let result = service
            .open_continuous_stream(&base_url(addr, "/live.ts"), &cancel)
            .await;
        let elapsed = started.elapsed();

        match result {
            Err(NetworkError::ResponseTimedOut) => {}
            Err(other) => panic!("expected ResponseTimedOut, got {other:?}"),
            Ok(_) => panic!("expected the header wait to time out"),
        }
        assert!(
            elapsed < Duration::from_secs(2),
            "a server that never answers must fail fast, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn continuous_ts_profile_detects_a_genuine_stall() {
        let (addr, _server) = spawn_server(|mut stream: TcpStream| async move {
            let header = b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nfirst\r\n";
            let _ = stream.write_all(header).await;
            // Then just hold the connection open forever without sending
            // more data or closing - a genuine stall, not a clean EOF.
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let config = NetworkServiceConfig {
            continuous_ts_stall_timeout: Duration::from_millis(300),
            ..NetworkServiceConfig::default()
        };
        let service = NetworkService::with_config(config).unwrap();
        let cancel = CancellationToken::new();

        let mut stream = service
            .open_continuous_stream(&base_url(addr, "/live.ts"), &cancel)
            .await
            .unwrap();

        let first = stream.next_chunk().await.unwrap();
        assert_eq!(first.as_deref(), Some(&b"first"[..]));

        let started = Instant::now();
        let second = stream.next_chunk().await;
        let elapsed = started.elapsed();

        assert!(matches!(second, Err(NetworkError::Stalled)));
        assert!(
            elapsed < Duration::from_secs(2),
            "stall was not detected promptly: took {elapsed:?}"
        );
    }

    // -----------------------------------------------------------------
    // HLS segment bounded concurrency
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn hls_segment_fetches_are_bounded_in_concurrency() {
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let active_for_server = active.clone();
        let peak_for_server = peak.clone();

        let (addr, _server) = spawn_server(move |mut stream: TcpStream| {
            let active = active_for_server.clone();
            let peak = peak_for_server.clone();
            async move {
                let now = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(150)).await;
                active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                let body = b"segment";
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body).await;
            }
        });

        let config = NetworkServiceConfig {
            hls_segment_max_concurrency: 2,
            ..NetworkServiceConfig::default()
        };
        let service = Arc::new(NetworkService::with_config(config).unwrap());
        let cancel = CancellationToken::new();

        let mut tasks = Vec::new();
        for _ in 0..6 {
            let service = service.clone();
            let cancel = cancel.clone();
            let url = base_url(addr, "/segment.ts");
            tasks.push(tokio::spawn(async move {
                service.fetch_hls_segment(&url, &cancel).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }

        assert!(
            peak.load(std::sync::atomic::Ordering::SeqCst) <= 2,
            "expected at most 2 concurrent segment fetches, saw {}",
            peak.load(std::sync::atomic::Ordering::SeqCst)
        );
    }
}
