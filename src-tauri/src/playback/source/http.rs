//! Direct continuous MPEG-TS source: no playlist, no segmentation - just a
//! long-lived HTTP body demuxed incrementally by `transmux::StreamingTsDemux`
//! via `apogee_playback_core::pipeline::TsIngest`.

use apogee_playback_core::pipeline::{SourceEvent, TsIngest};
use url::Url;

use crate::network::ContinuousTsStream;

use super::SourceError;

/// Drives the direct-TS path: feeds network chunks into [`TsIngest`], drains
/// `SourceEvent`s. Chunk boundaries are irrelevant - `TsIngest`/
/// `StreamingTsDemux` retain bounded partial state across them.
pub struct DirectTsSource {
    stream: ContinuousTsStream,
    ingest: TsIngest,
    /// Set once the connection has closed cleanly. Direct TS carries no
    /// explicit end-of-stream marker of its own (unlike HLS's
    /// `#EXT-X-ENDLIST`), so a closed connection is the only signal.
    ended: bool,
}

impl DirectTsSource {
    /// `probed_bytes` are the bytes [`apogee_playback_core::detect::Detector`]
    /// already consumed from `stream` while identifying it as MPEG-TS - real
    /// stream data, not detection scratch space - so they are replayed into
    /// the demuxer here before any further chunk is read. Losing them would
    /// silently drop the start of the audio.
    pub(crate) fn new(stream: ContinuousTsStream, probed_bytes: Vec<u8>) -> Self {
        let mut ingest = TsIngest::new();
        ingest.feed(&probed_bytes);
        Self {
            stream,
            ingest,
            ended: false,
        }
    }

    /// The effective URL after redirects. Carries no credentials of its own
    /// query the app doesn't already handle via `network::redact_url`, but
    /// callers that log it must still redact it themselves - this returns
    /// the real URL, not a redacted string, since some callers need it for
    /// further requests (e.g. resolving a relative reference).
    /// Diagnostics and tests: the effective URL this source is reading.
    #[allow(dead_code)]
    pub fn final_url(&self) -> &Url {
        self.stream.final_url()
    }

    /// Drains the next event, pulling more network data as needed.
    /// `Ok(None)` means the connection closed cleanly and no further event
    /// (including any end marker, since direct TS has none) will ever come.
    pub async fn next_event(&mut self) -> Result<Option<SourceEvent>, SourceError> {
        loop {
            if let Some(event) = self.ingest.poll() {
                return Ok(Some(event));
            }
            if self.ended {
                return Ok(None);
            }
            match self.stream.next_chunk().await {
                Ok(Some(chunk)) => self.ingest.feed(&chunk),
                Ok(None) => {
                    self.ingest.finish();
                    self.ended = true;
                }
                Err(error) => return Err(SourceError::from(error)),
            }
        }
    }
}
