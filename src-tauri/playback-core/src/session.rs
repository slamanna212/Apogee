//! Session identity, state, and retry budget.
//!
//! One controller owns playback truth. Every accepted play or stop gets a strictly
//! increasing [`Generation`], so an asynchronous completion that belongs to a session the
//! user has already moved on from can be discarded rather than allowed to emit audio or
//! overwrite state. Selecting the same station twice deliberately produces two different
//! generations: channel identity alone is not enough to tell those sessions apart.
//!
//! This module is deliberately free of I/O, Tauri, and time sources so the awkward races
//! can be tested directly rather than provoked through a live pipeline.

use serde::{Deserialize, Serialize};

/// Monotonic session identity. Never reused within a process.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize, Deserialize,
)]
pub struct Generation(u64);

impl Generation {
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Generation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gen{}", self.0)
    }
}

/// Internal playback state, per the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackState {
    Stopped,
    Connecting,
    Buffering,
    Playing,
    /// Lost a working stream and is trying to get back, with retries left.
    Recovering,
    Failed,
}

impl PlaybackState {
    /// Projection onto the existing frontend `PlayerStatus` union, so the UI contract
    /// does not change. `src/types/player.ts` defines these five strings.
    #[must_use]
    pub fn as_frontend_status(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Connecting | Self::Buffering | Self::Recovering => "loading",
            Self::Playing => "playing",
            Self::Failed => "error",
        }
    }

    /// The frontend tracks buffering separately from status.
    #[must_use]
    pub fn is_buffering(self) -> bool {
        matches!(self, Self::Buffering | Self::Recovering)
    }

    /// Whether real audio is reaching the device. Scrobbling and presence must key off
    /// this, never off "a connection succeeded".
    #[must_use]
    pub fn is_audible(self) -> bool {
        matches!(self, Self::Playing)
    }
}

/// Whether an error can be retried at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorClass {
    /// Worth another attempt: timeouts, resets, upstream station spin-up.
    Transient,
    /// Retrying cannot help: bad credentials, unsupported content, a non-media body.
    Permanent,
}

/// Why playback is not yet audible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BufferingReason {
    Connecting,
    FillingBuffer,
    Underrun,
    Reconnecting,
}

/// Retry defaults carried over from `src/stores/playerStore.ts` so behaviour is unchanged.
pub const MAX_CONNECT_ATTEMPTS: u32 = 4;
pub const RETRY_DELAY_MS: u64 = 1_500;
pub const CONNECT_TIMEOUT_MS: u64 = 20_000;

/// How long the whole connect phase may take before giving up.
///
/// The attempt count alone is not a sufficient budget. Under MPV each attempt could sit for
/// up to `CONNECT_TIMEOUT_MS` waiting for a stream that never arrived, so four attempts
/// spanned more than a minute in the worst case. An error that fails *immediately* -
/// notably a 503 while a backend is still spinning the upstream up - collapses those four
/// attempts into a few seconds and gives up long before the stream could ever have become
/// available.
///
/// This budget restores the wall-clock window independently of how fast attempts fail. It
/// is deliberately backend-agnostic: a provider reached directly and a proxy in front of one
/// both take time to bring a channel up, and the plan already records first-attempt
/// timeouts during "upstream channel spin-up" as a known transient condition.
pub const CONNECT_BUDGET_MS: u64 = 90_000;

/// Ceiling on the backoff delay between attempts.
pub const MAX_RETRY_DELAY_MS: u64 = 8_000;

/// Delay before the attempt numbered `attempt` (1-based), with exponential backoff.
///
/// Backoff matters because a fast-failing endpoint would otherwise be hammered for the whole
/// budget. Capped so recovery stays responsive once the stream does come up.
#[must_use]
pub fn retry_delay_ms(attempt: u32) -> u64 {
    let shift = attempt.saturating_sub(1).min(6);
    (RETRY_DELAY_MS.saturating_mul(1u64 << shift)).min(MAX_RETRY_DELAY_MS)
}
/// Uninterrupted audible playback that must elapse before the retry budget is refilled.
///
/// The plan is explicit that a successful HTTP response is not evidence of health; only
/// sustained real playback is. Without this, a stream that connects and dies repeatedly
/// would retry forever.
pub const STABLE_PLAY_RESET_MS: u64 = 30_000;

/// A point-in-time view for the UI. Carries both identity and ordering so a late snapshot
/// cannot overwrite a newer event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub generation: Generation,
    /// Strictly increasing across every observable change, for any generation.
    pub revision: u64,
    pub station_id: Option<String>,
    pub state: PlaybackState,
    pub buffering_reason: Option<BufferingReason>,
    pub attempt: u32,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub device: Option<String>,
    /// Already redacted. Never carries credentials.
    pub error: Option<String>,
}

/// Owns playback truth for the process.
#[derive(Debug)]
pub struct Controller {
    generation: Generation,
    revision: u64,
    state: PlaybackState,
    station_id: Option<String>,
    buffering_reason: Option<BufferingReason>,
    attempt: u32,
    bitrate_kbps: Option<u32>,
    sample_rate: Option<u32>,
    device: Option<String>,
    error: Option<String>,
    /// Monotonic milliseconds at which the current Playing stretch began.
    playing_since_ms: Option<u64>,
    /// When the current connect phase began, for the wall-clock budget.
    connect_started_ms: Option<u64>,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: Generation(0),
            revision: 0,
            state: PlaybackState::Stopped,
            station_id: None,
            buffering_reason: None,
            attempt: 0,
            bitrate_kbps: None,
            sample_rate: None,
            device: None,
            error: None,
            playing_since_ms: None,
            connect_started_ms: None,
        }
    }

    fn bump(&mut self) {
        self.revision += 1;
    }

    /// Accept a play request and start a new session.
    ///
    /// Always allocates a fresh generation, even for the station already playing, so two
    /// rapid selections of the same station cannot be confused for one another.
    ///
    /// `now_ms` starts the [`CONNECT_BUDGET_MS`] wall-clock accounting immediately, at the
    /// actual beginning of the connect phase - not lazily at the first failure, which would
    /// hand a slow-but-eventually-failing first attempt extra, unaccounted-for budget.
    pub fn play(&mut self, station_id: impl Into<String>, now_ms: u64) -> Generation {
        self.generation = Generation(self.generation.0 + 1);
        self.station_id = Some(station_id.into());
        self.state = PlaybackState::Connecting;
        self.buffering_reason = Some(BufferingReason::Connecting);
        self.attempt = 0;
        self.bitrate_kbps = None;
        self.sample_rate = None;
        self.error = None;
        self.playing_since_ms = None;
        self.connect_started_ms = Some(now_ms);
        self.bump();
        self.generation
    }

    /// Accept a stop. Invalidates the current session by taking a new generation, so work
    /// already in flight for the old one is rejected on completion.
    pub fn stop(&mut self) -> Generation {
        self.generation = Generation(self.generation.0 + 1);
        self.state = PlaybackState::Stopped;
        self.station_id = None;
        self.buffering_reason = None;
        self.attempt = 0;
        self.bitrate_kbps = None;
        self.sample_rate = None;
        self.error = None;
        self.playing_since_ms = None;
        self.connect_started_ms = None;
        self.bump();
        self.generation
    }

    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    #[must_use]
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    #[must_use]
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Whether a completion belonging to `generation` may still affect state.
    #[must_use]
    pub fn accepts(&self, generation: Generation) -> bool {
        generation == self.generation && self.state != PlaybackState::Stopped
    }

    /// Which URL extension attempt `attempt` should use.
    ///
    /// Preserves the existing alternating `.ts`, `.m3u8`, `.ts`, `.m3u8` behaviour: the
    /// provider serves usable content from both, and which one works has varied.
    #[must_use]
    pub fn extension_for_attempt(attempt: u32) -> &'static str {
        if attempt.is_multiple_of(2) {
            ".ts"
        } else {
            ".m3u8"
        }
    }

    /// Report that the transport connected but audio is not flowing yet.
    pub fn on_buffering(&mut self, generation: Generation, reason: BufferingReason) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        self.state = PlaybackState::Buffering;
        self.buffering_reason = Some(reason);
        self.playing_since_ms = None;
        self.bump();
        true
    }

    /// Report that the output device is actually consuming samples for this session.
    ///
    /// This, not a successful HTTP response and not the decoder starting, is what makes
    /// playback `Playing`.
    pub fn on_audible(&mut self, generation: Generation, now_ms: u64) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        if self.playing_since_ms.is_none() {
            self.playing_since_ms = Some(now_ms);
        }
        self.state = PlaybackState::Playing;
        self.buffering_reason = None;
        self.error = None;
        self.bump();
        true
    }

    /// Report a failure. Returns the next step.
    ///
    /// The retry budget refills only after [`STABLE_PLAY_RESET_MS`] of uninterrupted
    /// audible playback, never merely because a request succeeded.
    pub fn on_error(
        &mut self,
        generation: Generation,
        class: ErrorClass,
        redacted_message: impl Into<String>,
        now_ms: u64,
    ) -> Option<Next> {
        if !self.accepts(generation) {
            return None;
        }

        if let Some(since) = self.playing_since_ms
            && now_ms.saturating_sub(since) >= STABLE_PLAY_RESET_MS
        {
            self.attempt = 0;
            self.connect_started_ms = None;
        }
        self.playing_since_ms = None;

        if class == ErrorClass::Permanent {
            self.state = PlaybackState::Failed;
            self.buffering_reason = None;
            self.error = Some(redacted_message.into());
            self.bump();
            return Some(Next::GiveUp);
        }

        // Set by `play()` at the actual start of the connect phase; `get_or_insert` here is
        // only a defensive fallback and should never actually need to insert.
        let started = *self.connect_started_ms.get_or_insert(now_ms);
        let elapsed = now_ms.saturating_sub(started);
        self.attempt += 1;

        // Two independent limits, and giving up needs BOTH to be spent. The attempt count
        // stops a slow endpoint being retried forever; the wall-clock budget stops a
        // fast-failing one from exhausting those attempts in a couple of seconds, long
        // before a stream that is still starting up could have appeared.
        let attempts_spent = self.attempt >= MAX_CONNECT_ATTEMPTS;
        let budget_spent = elapsed >= CONNECT_BUDGET_MS;
        if attempts_spent && budget_spent {
            self.state = PlaybackState::Failed;
            self.buffering_reason = None;
            self.error = Some(redacted_message.into());
            self.bump();
            return Some(Next::GiveUp);
        }

        self.state = PlaybackState::Recovering;
        self.buffering_reason = Some(BufferingReason::Reconnecting);
        self.error = None;
        self.bump();
        Some(Next::Retry {
            attempt: self.attempt,
            delay_ms: retry_delay_ms(self.attempt),
            extension: Self::extension_for_attempt(self.attempt),
        })
    }

    pub fn set_bitrate(&mut self, generation: Generation, kbps: Option<u32>) -> bool {
        if !self.accepts(generation) || self.bitrate_kbps == kbps {
            return false;
        }
        self.bitrate_kbps = kbps;
        self.bump();
        true
    }

    pub fn set_format(&mut self, generation: Generation, sample_rate: u32) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        self.sample_rate = Some(sample_rate);
        self.bump();
        true
    }

    /// The effective output device. Tracked independently of the requested device so a
    /// temporary unplug does not erase the user's preference.
    pub fn set_device(&mut self, device: Option<String>) {
        if self.device != device {
            self.device = device;
            self.bump();
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            generation: self.generation,
            revision: self.revision,
            station_id: self.station_id.clone(),
            state: self.state,
            buffering_reason: self.buffering_reason,
            attempt: self.attempt,
            bitrate_kbps: self.bitrate_kbps,
            sample_rate: self.sample_rate,
            device: self.device.clone(),
            error: self.error.clone(),
        }
    }
}

/// What the controller wants done after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Retry {
        attempt: u32,
        delay_ms: u64,
        extension: &'static str,
    },
    GiveUp,
}
