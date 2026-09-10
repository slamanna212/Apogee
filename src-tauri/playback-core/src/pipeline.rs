//! The convergence point: both source paths meet here as compressed access units.
//!
//! The plan's central invariant is **demux exactly once**. Direct MPEG-TS is demuxed by
//! `transmux::StreamingTsDemux`; HLS is demuxed inside hls-runtime by the *same* transmux
//! code, which hands back already-demuxed samples. Neither path may be demuxed again.
//!
//! Because both libraries already speak `transmux::{TrackSpec, Sample}`, that pair is the
//! narrowest possible shared representation. Inventing a parallel struct here would add a
//! translation layer that could silently diverge between the two paths, which is precisely
//! the failure the plan warns about.

use crate::decode::{AudioTrackDecoder, PcmBlock};
use broadcast_common::Unpackage;
use hls_runtime::client::Output;
use transmux::{DemuxEvent, Fmp4Demux, Sample, StreamingTsDemux, TrackSpec};

/// A source-agnostic event. Identical for direct TS and for HLS.
#[derive(Debug)]
pub enum SourceEvent {
    /// The selected audio track's configuration. Arrives before any `Access`.
    Track(TrackSpec),
    /// One compressed access unit.
    Access(Sample),
    /// A genuine discontinuity: encoding, timing, or codec may have changed.
    /// A normal segment boundary is NOT one of these.
    Discontinuity,
    EndOfStream,
}

/// Errors that stop a session. All are actionable rather than silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    NoAudioTrack,
    Decode(String),
    Demux(String),
    /// Samples arrived before any track configuration.
    SamplesBeforeTrack,
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAudioTrack => write!(f, "stream contains no supported audio track"),
            Self::Decode(e) => write!(f, "audio decode failed: {e}"),
            Self::Demux(e) => write!(f, "stream demux failed: {e}"),
            Self::SamplesBeforeTrack => {
                write!(f, "stream sent audio before declaring its format")
            }
        }
    }
}

impl std::error::Error for PipelineError {}

/// Owns exactly one decoder for the session and turns `SourceEvent`s into PCM.
///
/// The decoder is created once when the track is configured and reset only on a real
/// discontinuity, never per access unit and never per HLS segment.
#[derive(Default)]
pub struct Pipeline {
    decoder: Option<AudioTrackDecoder>,
    /// Counts decoder resets, so tests can assert that ordinary segment
    /// boundaries do not cause one.
    resets: u32,
    compressed_bytes: u64,
    decoded_frames: u64,
}

impl Pipeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn resets(&self) -> u32 {
        self.resets
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.decoder.is_some()
    }

    /// Compressed audio bytes seen so far. Deliberately counts audio access units
    /// only, never TS or HTTP overhead, so a derived bitrate is honest.
    #[must_use]
    pub fn compressed_bytes(&self) -> u64 {
        self.compressed_bytes
    }

    /// Decoded frames (per channel) so far.
    #[must_use]
    pub fn decoded_frames(&self) -> u64 {
        self.decoded_frames
    }

    /// Average audio bitrate in kbps, or `None` until there is enough evidence.
    ///
    /// Returns `None` rather than a wild early estimate; the plan requires that
    /// unknown bitrate stay unknown.
    #[must_use]
    pub fn bitrate_kbps(&self, sample_rate: u32) -> Option<u32> {
        if sample_rate == 0 || self.decoded_frames < u64::from(sample_rate) {
            return None;
        }
        let seconds = self.decoded_frames as f64 / f64::from(sample_rate);
        Some(((self.compressed_bytes as f64 * 8.0 / seconds) / 1000.0).round() as u32)
    }

    /// Apply one event. Returns PCM when the event produced audio.
    pub fn accept(&mut self, event: SourceEvent) -> Result<Option<PcmBlock>, PipelineError> {
        match event {
            SourceEvent::Track(spec) => {
                let decoder = AudioTrackDecoder::new(&spec).map_err(PipelineError::Decode)?;
                self.decoder = Some(decoder);
                Ok(None)
            }
            SourceEvent::Access(sample) => {
                let decoder = self
                    .decoder
                    .as_mut()
                    .ok_or(PipelineError::SamplesBeforeTrack)?;
                let len = sample.data.len() as u64;
                let block = decoder.decode(&sample).map_err(PipelineError::Decode)?;
                self.compressed_bytes += len;
                self.decoded_frames += (block.samples.len() / block.channels.max(1)) as u64;
                Ok(Some(block))
            }
            SourceEvent::Discontinuity => {
                if let Some(d) = self.decoder.as_mut() {
                    d.reset();
                    self.resets += 1;
                }
                Ok(None)
            }
            SourceEvent::EndOfStream => Ok(None),
        }
    }
}

/// Direct continuous MPEG-TS ingestion.
///
/// Feed arbitrary network chunks; boundaries are irrelevant because
/// `StreamingTsDemux` retains bounded partial state across them.
#[derive(Default)]
pub struct TsIngest {
    demux: StreamingTsDemux,
    selected: Option<u32>,
}

impl TsIngest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            demux: StreamingTsDemux::new(),
            selected: None,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.demux.feed(chunk);
    }

    /// Signals that the body ended, flushing any completable trailing frame.
    pub fn finish(&mut self) {
        self.demux.finish();
    }

    /// Drain one event, translating transmux's events into the shared form.
    ///
    /// The first audio track wins and later tracks are ignored, so a stream that
    /// also carries video or a second language cannot make output nondeterministic.
    pub fn poll(&mut self) -> Option<SourceEvent> {
        while let Some(event) = self.demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) => {
                    if self.selected.is_none() && AudioTrackDecoder::supports(&spec) {
                        self.selected = Some(spec.track_id);
                        return Some(SourceEvent::Track(spec));
                    }
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if Some(track_id) == self.selected {
                        return Some(SourceEvent::Access(sample));
                    }
                }
                DemuxEvent::Discontinuity { .. } => return Some(SourceEvent::Discontinuity),
                _ => {}
            }
        }
        None
    }
}

/// Translates hls-runtime client output into the shared form.
///
/// `Output::Init` carries fMP4 initialization bytes, synthesized by hls-runtime even for
/// TS segments. Only that initialization metadata is parsed here. Sample payloads from
/// `Output::Samples` are already demuxed and are forwarded untouched: passing them
/// through another demuxer would violate the demux-once invariant.
#[derive(Default)]
pub struct HlsIngest {
    selected: Option<u32>,
    inits: u32,
}

impl HlsIngest {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of initialization sections parsed. Ordinary segment boundaries must
    /// not increase this.
    #[must_use]
    pub fn inits(&self) -> u32 {
        self.inits
    }

    /// Translate one client output into zero or more shared events.
    pub fn translate(&mut self, output: Output) -> Result<Vec<SourceEvent>, PipelineError> {
        match output {
            Output::Init(bytes) => {
                let media = Fmp4Demux::new()
                    .unpackage(&bytes)
                    .map_err(|e| PipelineError::Demux(format!("{e:?}")))?;
                let track = media
                    .tracks
                    .into_iter()
                    .find(|t| AudioTrackDecoder::supports(&t.spec))
                    .ok_or(PipelineError::NoAudioTrack)?;
                debug_assert!(
                    track.samples.is_empty(),
                    "an initialization section must not carry samples"
                );
                self.inits += 1;
                self.selected = Some(track.spec.track_id);
                Ok(vec![SourceEvent::Track(track.spec)])
            }
            Output::Samples { track_id, samples } => {
                if Some(track_id) != self.selected {
                    return Ok(Vec::new());
                }
                Ok(samples.into_iter().map(SourceEvent::Access).collect())
            }
            Output::Discontinuity => Ok(vec![SourceEvent::Discontinuity]),
            Output::EndOfStream => Ok(vec![SourceEvent::EndOfStream]),
            _ => Ok(Vec::new()),
        }
    }
}
