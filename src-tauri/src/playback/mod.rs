//! Rust-native audio playback engine (Symphonia migration milestone M2, see
//! `SYMPHONIA_PLAYBACK_PLAN.md` sections 7-8 and
//! `docs/symphonia-migration-progress.md`).
//!
//! Turns a stream URL into a sequence of
//! `apogee_playback_core::pipeline::SourceEvent`s by identifying what the
//! bytes actually are (never the URL extension or a declared content type -
//! the target provider mislabels both) and driving the matching path:
//! direct continuous MPEG-TS, or the `hls-runtime` client for HLS. Both
//! paths reuse `apogee_playback_core`'s already-built and already-tested
//! detection, demux, and decode building blocks; this module supplies only
//! the cancellable network glue (`crate::network::NetworkService`) that
//! feeds them.
//!
//! Decoding (`apogee_playback_core::pipeline::Pipeline`) and everything
//! downstream of `SourceEvent` - retry/backoff policy, output, the state
//! machine - is milestone M3's job, not this module's. See [`source`]'s
//! module docs for the full division of responsibility.
//!
//! Not wired into any Tauri command yet - nothing in the running app calls
//! this module until M3 builds the controller on top of it.

pub mod audio_out;
pub mod commands;
pub mod device;
pub mod engine;
pub mod source;

// Not consumed by anything until M3's controller exists; see the module
// docs above.
#[allow(unused_imports)]
pub use source::{open, Source, SourceError};
