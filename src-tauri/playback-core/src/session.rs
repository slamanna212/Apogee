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
        }
    }

    fn bump(&mut self) {
        self.revision += 1;
    }

    /// Accept a play request and start a new session.
    ///
    /// Always allocates a fresh generation, even for the station already playing, so two
    /// rapid selections of the same station cannot be confused for one another.
    pub fn play(&mut self, station_id: impl Into<String>) -> Generation {
        self.generation = Generation(self.generation.0 + 1);
        self.station_id = Some(station_id.into());
        self.state = PlaybackState::Connecting;
        self.buffering_reason = Some(BufferingReason::Connecting);
        self.attempt = 0;
        self.bitrate_kbps = None;
        self.sample_rate = None;
        self.error = None;
        self.playing_since_ms = None;
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
        }
        self.playing_since_ms = None;

        if class == ErrorClass::Permanent {
            self.state = PlaybackState::Failed;
            self.buffering_reason = None;
            self.error = Some(redacted_message.into());
            self.bump();
            return Some(Next::GiveUp);
        }

        self.attempt += 1;
        if self.attempt >= MAX_CONNECT_ATTEMPTS {
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
            delay_ms: RETRY_DELAY_MS,
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
