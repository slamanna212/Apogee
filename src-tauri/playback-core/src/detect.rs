//! Bounded source detection: decide what a stream actually is from its first bytes.
//!
//! Extensions and MIME types are hints, never decisions. The provider this app targets
//! serves raw MPEG-TS from both `.ts` and `.m3u8` URLs, so routing on the URL is wrong.
//! Every byte examined here is retained and replayed into the chosen path.

use container_probe::{Format, Probe, probe_with_budget};

/// Upper bound on retained probe bytes. Matches `container_probe::DEFAULT_BUDGET`.
pub const MAX_PROBE_BYTES: usize = 64 * 1024;

/// What the bytes turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Continuous MPEG-TS, the direct path. `stride` is the measured packet size.
    MpegTs { stride: u16 },
    /// An HLS media playlist, drivable by `HlsClient`.
    HlsMediaPlaylist,
    /// An HLS master playlist. `HlsClient` cannot parse these; a variant must be
    /// selected and fetched first. See `select_variant`.
    HlsMasterPlaylist,
    /// Raw ADTS AAC elementary stream.
    AdtsAac,
    /// Raw MPEG-1/2 Layer III elementary stream.
    Mp3,
    /// ISO base media / fragmented MP4.
    Isobmff,
}

/// Why a source cannot be played. Every variant is actionable: none of them should
/// ever surface to a user as silence or as a generic parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    /// The origin returned an HTML or JSON body, typically an error page served
    /// with HTTP 200. Endless decoder probing on this is a bug.
    NotMedia { preview: String },
    /// An HLS playlist carrying `#EXT-X-KEY`. hls-runtime has no decryption
    /// support whatsoever, so this is a hard stop rather than a degraded mode.
    EncryptedPlaylist,
    /// Bytes were conclusive and are not a format this app handles.
    UnknownFormat,
    /// Several formats tied and the content is genuinely ambiguous.
    Ambiguous { candidates: Vec<Format> },
    /// The probe budget was exhausted without reaching a conclusion.
    BudgetExhausted { examined: usize },
}

/// Outcome of a detection attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    Identified(SourceKind),
    /// Inconclusive so far, but more bytes could decide it. Not an error.
    NeedMoreData {
        need_at_least: usize,
    },
    Unsupported(Unsupported),
}

/// Accumulates a bounded prefix and decides what it is.
///
/// Bytes are retained so the caller can replay them into the selected demuxer;
/// detection must never consume input that playback still needs.
#[derive(Debug)]
pub struct Detector {
    buf: Vec<u8>,
    budget: usize,
}

impl Default for Detector {
    fn default() -> Self {
        Self::with_budget(MAX_PROBE_BYTES)
    }
}

impl Detector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_budget(budget: usize) -> Self {
        Self {
            buf: Vec::with_capacity(budget.min(8192)),
            budget,
        }
    }

    /// Append a network chunk. Retains at most `budget` bytes; detection is a
    /// prefix decision, so later bytes cannot change it.
    pub fn push(&mut self, chunk: &[u8]) {
        if self.buf.len() >= self.budget {
            return;
        }
        let room = self.budget - self.buf.len();
        let take = room.min(chunk.len());
        self.buf.extend_from_slice(&chunk[..take]);
    }

    /// Every byte examined so far, for replay into the selected path.
    #[must_use]
    pub fn buffered(&self) -> &[u8] {
        &self.buf
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.buf.len() >= self.budget
    }

    /// Decide, using the accumulated prefix.
    ///
    /// `content_type` is consulted only to break a genuine tie. It never overrides
    /// what the bytes say, because the target provider mislabels its responses.
    #[must_use]
    pub fn detect(&self, content_type: Option<&str>) -> Detection {
        let body = strip_bom(&self.buf);
        let trimmed = trim_ascii_start(body);

        // Text formats first: container-probe deals in binary containers and
        // would report Unknown for a playlist.
        if let Some(d) = detect_playlist(trimmed) {
            return d;
        }
        if let Some(preview) = non_media_preview(trimmed) {
            return Detection::Unsupported(Unsupported::NotMedia { preview });
        }
        // A short prefix that is still plausibly a playlist must not be judged yet.
        if !trimmed.is_empty() && is_playlist_prefix(trimmed) {
            return Detection::NeedMoreData {
                need_at_least: EXTM3U.len(),
            };
        }

        match probe_with_budget(&self.buf, self.budget) {
            Probe::Identified { format, detail, .. } => match map_format(format, detail) {
                Some(kind) => Detection::Identified(kind),
                None => Detection::Unsupported(Unsupported::UnknownFormat),
            },
            Probe::Ambiguous { candidates, .. } => {
                // A declared content type may legitimately break a tie.
                if let Some(ct) = content_type {
                    let hinted = hint_format(ct);
                    if let Some(c) = candidates.iter().find(|c| Some(c.format) == hinted)
                        && let Some(kind) = map_format(c.format, c.detail)
                    {
                        return Detection::Identified(kind);
                    }
                }
                Detection::Unsupported(Unsupported::Ambiguous {
                    candidates: candidates.iter().map(|c| c.format).collect(),
                })
            }
            Probe::Insufficient { need_at_least, .. } => {
                if self.is_full() {
                    Detection::Unsupported(Unsupported::BudgetExhausted {
                        examined: self.buf.len(),
                    })
                } else {
                    Detection::NeedMoreData { need_at_least }
                }
            }
            Probe::Unknown => Detection::Unsupported(Unsupported::UnknownFormat),
            _ => Detection::Unsupported(Unsupported::UnknownFormat),
        }
    }
}

const EXTM3U: &[u8] = b"#EXTM3U";

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)
}

fn trim_ascii_start(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

/// True when the prefix is a strict, still-incomplete prefix of `#EXTM3U`.
fn is_playlist_prefix(b: &[u8]) -> bool {
    b.len() < EXTM3U.len() && EXTM3U.starts_with(b)
}

fn detect_playlist(b: &[u8]) -> Option<Detection> {
    if !b.starts_with(EXTM3U) {
        return None;
    }
    // Tag scan is whole-buffer: these tags are not required to be near the top.
    if contains_tag(b, b"#EXT-X-KEY") {
        return Some(Detection::Unsupported(Unsupported::EncryptedPlaylist));
    }
    if contains_tag(b, b"#EXT-X-STREAM-INF") {
        return Some(Detection::Identified(SourceKind::HlsMasterPlaylist));
    }
    Some(Detection::Identified(SourceKind::HlsMediaPlaylist))
}

fn contains_tag(haystack: &[u8], tag: &[u8]) -> bool {
    haystack
        .windows(tag.len())
        .any(|w| w.eq_ignore_ascii_case(tag))
}

/// Detects an HTML or JSON body, which origins sometimes serve with HTTP 200.
fn non_media_preview(b: &[u8]) -> Option<String> {
    let first = *b.first()?;
    if first != b'<' && first != b'{' && first != b'[' {
        return None;
    }
    let take = b.len().min(120);
    Some(
        String::from_utf8_lossy(&b[..take])
            .replace(['\n', '\r'], " ")
            .trim()
            .to_string(),
    )
}

fn hint_format(content_type: &str) -> Option<Format> {
    let ct = content_type.to_ascii_lowercase();
    let ct = ct.split(';').next()?.trim();
    match ct {
        "video/mp2t" | "audio/mp2t" => Some(Format::MpegTs),
        "audio/aac" | "audio/aacp" => Some(Format::AdtsAac),
        "audio/mpeg" | "audio/mp3" => Some(Format::Mp3),
        "video/mp4" | "audio/mp4" => Some(Format::Isobmff),
        _ => None,
    }
}

fn map_format(format: Format, detail: container_probe::Detail) -> Option<SourceKind> {
    match format {
        Format::MpegTs => {
            let stride = match detail {
                container_probe::Detail::Ts { stride, .. } => stride,
                _ => 188,
            };
            Some(SourceKind::MpegTs { stride })
        }
        Format::AdtsAac => Some(SourceKind::AdtsAac),
        Format::Mp3 => Some(SourceKind::Mp3),
        Format::Isobmff => Some(SourceKind::Isobmff),
        _ => None,
    }
}

/// Picks a media playlist URI from a master playlist.
///
/// Deterministic by construction: highest advertised `BANDWIDTH`, ties broken by the
/// URI itself, so the same master always yields the same choice. Returns the raw URI,
/// which the caller must resolve against the effective post-redirect playlist URL.
#[must_use]
pub fn select_variant(master: &str) -> Option<&str> {
    let mut best: Option<(u64, &str)> = None;
    let mut pending_bandwidth: Option<u64> = None;

    for line in master.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_bandwidth = Some(parse_bandwidth(attrs));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(bw) = pending_bandwidth.take() {
            best = match best {
                Some((best_bw, best_uri)) if (best_bw, best_uri) >= (bw, line) => {
                    Some((best_bw, best_uri))
                }
                _ => Some((bw, line)),
            };
        }
    }
    best.map(|(_, uri)| uri)
}

fn parse_bandwidth(attrs: &str) -> u64 {
    // Quoted attribute values may contain commas (e.g. CODECS="mp4a.40.2,avc1").
    let mut in_quotes = false;
    let mut start = 0usize;
    let bytes = attrs.as_bytes();
    let mut fields: Vec<&str> = Vec::new();
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'"' => in_quotes = !in_quotes,
            b',' if !in_quotes => {
                fields.push(&attrs[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    fields.push(&attrs[start..]);

    for f in fields {
        let f = f.trim();
        if let Some(v) = f.strip_prefix("BANDWIDTH=") {
            return v.trim().parse().unwrap_or(0);
        }
    }
    0
}
