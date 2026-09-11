//! HLS driver: drives `hls_runtime::client::HlsClient` - a sans-IO,
//! caller-driven state machine that owns no HTTP stack of its own (see its
//! module docs) - using `crate::network::NetworkService` for every network
//! operation, and translates its output into the shared `SourceEvent` form
//! via `apogee_playback_core::pipeline::HlsIngest`.
//!
//! # Requests are serviced one at a time
//!
//! This driver never fires off more than one `Action` concurrently: each
//! `poll()` result is fully serviced (network round-trip and the matching
//! `on_playlist`/`on_resource`/`on_error` call) before the next `poll()`.
//! That alone bounds this session's outstanding requests to one; the global
//! `NetworkService::fetch_hls_segment` semaphore bounds cross-session
//! concurrency on top of it (see `network.rs` module docs). It also means
//! completed requests can never reorder playback - the client only ever
//! sees responses in the order it asked for them.
//!
//! # What this module does not do
//!
//! No retry/backoff policy and no state machine live here (that is
//! milestone M3's controller, per the plan) - a single fetch failure simply
//! surfaces as a `SourceError`. This driver is the cancellable building
//! block the controller will retry on top of, not the retry policy itself.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use apogee_playback_core::detect::{is_encrypted_playlist, Unsupported};
use apogee_playback_core::pipeline::{HlsIngest, PipelineError, SourceEvent};
use hls_runtime::client::{Action, HlsClient};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::network::{cancellable_sleep, redact_text, NetworkError, NetworkService};

use super::SourceError;

/// Drives one HLS media-playlist session end to end.
pub struct HlsSource {
    client: HlsClient,
    ingest: HlsIngest,
    network: NetworkService,
    cancel: CancellationToken,
    /// Shared events translated from one `Output` that haven't been handed
    /// to the caller yet - `HlsIngest::translate` can produce more than one
    /// `SourceEvent` per `Output`, so these are drained before polling the
    /// client again.
    pending: VecDeque<SourceEvent>,
    actions: VecDeque<Action>,
    playlist_received_at: Instant,
}

impl HlsSource {
    /// Builds a driver for the media playlist at `playlist_final_url`.
    ///
    /// `playlist_final_url` MUST already be the final, post-redirect URL -
    /// `HlsClient` resolves every segment/part/map URI in the playlist
    /// against exactly the URL it was constructed with
    /// (`hls_runtime::client::Action::FetchResource`'s `url` arrives
    /// pre-resolved against it), so constructing against the originally
    /// requested URL would send every following request to the wrong host
    /// once a redirect is in play. See
    /// `docs/symphonia-dependency-validation.md`'s gap #1 and
    /// `docs/symphonia-migration-progress.md`'s live provider re-check.
    ///
    /// `initial_body` is the playlist body the caller already fetched (the
    /// detection probe, or a master-playlist variant fetch) - it is fed
    /// straight to the client's self-seeded first `Action::FetchPlaylist`
    /// instead of triggering a second, possibly inconsistent fetch of the
    /// same *live* resource.
    pub(crate) fn from_initial_playlist(
        playlist_final_url: Url,
        initial_body: &[u8],
        network: NetworkService,
        cancel: CancellationToken,
    ) -> Result<Self, SourceError> {
        // Defense in depth: every caller of `open()` already runs the
        // initial/variant body through `classify_complete_hls_playlist`
        // before reaching here, but `HlsClient::on_playlist` itself
        // (`broadcast_hls::MediaPlaylist::parse`) has no opinion on
        // `#EXT-X-KEY` at all - it just parses the attributes. Checking
        // again at this single choke point means the policy holds even if
        // a future caller is added that skips the caller-side check.
        reject_if_encrypted(initial_body)?;
        let mut client = HlsClient::new(playlist_final_url.to_string());
        match client.poll() {
            Some(Action::FetchPlaylist { .. }) => {}
            other => {
                return Err(SourceError::Demux(format!(
                    "expected the client's seeded first action to be a playlist fetch, got {other:?}"
                )));
            }
        }
        client
            .on_playlist(initial_body)
            .map_err(|error| SourceError::Demux(redact_text(&error.to_string())))?;
        let actions = scheduled_actions(&mut client);
        Ok(Self {
            client,
            ingest: HlsIngest::new(),
            network,
            cancel,
            pending: VecDeque::new(),
            actions,
            playlist_received_at: Instant::now(),
        })
    }

    /// The playlist URL this client is following - always the final,
    /// post-redirect URL it was constructed with.
    /// Diagnostics and tests: the effective URL this source is reading.
    #[allow(dead_code)]
    pub fn playlist_url(&self) -> &str {
        self.client.playlist_url()
    }

    /// Drains the next event, performing whatever network IO is needed
    /// along the way. `Ok(None)` means nothing is available *right now*
    /// (the client has no pending action or output) - for a live playlist
    /// with no `#EXT-X-ENDLIST` this is transient, never terminal, since
    /// `on_playlist` always schedules its own next reload/wait. A genuine
    /// end (`#EXT-X-ENDLIST` fully drained) surfaces as
    /// `SourceEvent::EndOfStream` before this ever returns `Ok(None)` for
    /// that reason.
    pub async fn next_event(&mut self) -> Result<Option<SourceEvent>, SourceError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            if let Some(output) = self.client.next_output() {
                let events = self
                    .ingest
                    .translate(output)
                    .map_err(pipeline_error_to_source)?;
                self.pending.extend(events);
                continue;
            }
            match self.actions.pop_front() {
                Some(action) => self.service(action).await?,
                None => return Ok(None),
            }
        }
    }

    async fn service(&mut self, action: Action) -> Result<(), SourceError> {
        match &action {
            Action::FetchPlaylist { .. } => {
                let request_url = action
                    .playlist_request_url()
                    .expect("Action::FetchPlaylist always has a request URL");
                match self
                    .network
                    .fetch_hls_playlist(&request_url, &self.cancel)
                    .await
                {
                    Ok(body) => {
                        // A live origin can add `#EXT-X-KEY` to a reload
                        // that started out unencrypted - the *initial*
                        // playlist carrying no encryption tag says nothing
                        // about later ones (finding 8). Every reload gets
                        // the same full-body check the initial fetch did,
                        // before it ever reaches `HlsClient::on_playlist`,
                        // so a later encryption tag cannot bypass policy by
                        // arriving after startup instead of during it.
                        reject_if_encrypted(&body.bytes)?;
                        self.client
                            .on_playlist(&body.bytes)
                            .map_err(|error| SourceError::Demux(redact_text(&error.to_string())))?;
                        self.playlist_received_at = Instant::now();
                        self.actions.extend(scheduled_actions(&mut self.client));
                        Ok(())
                    }
                    Err(NetworkError::Cancelled) => Err(SourceError::Cancelled),
                    Err(error) => {
                        // hls-runtime clears its "requested" bookkeeping for
                        // this action on `on_error` so a later `on_playlist`
                        // can cleanly re-request it - it never retries on
                        // its own (no IO of its own to retry with).
                        self.client.on_error(None);
                        Err(SourceError::from(error))
                    }
                }
            }
            Action::FetchResource {
                id,
                url,
                byte_range,
            } => {
                let id = *id;
                let url = url.clone();
                let byte_range = *byte_range;
                match self.network.fetch_hls_segment(&url, &self.cancel).await {
                    Ok(body) => {
                        let bytes = match byte_range {
                            Some((offset, length)) => {
                                slice_byte_range(&body.bytes, offset, length)?
                            }
                            None => body.bytes,
                        };
                        self.client
                            .on_resource(id, &bytes)
                            .map_err(|error| SourceError::Demux(redact_text(&error.to_string())))
                    }
                    Err(NetworkError::Cancelled) => Err(SourceError::Cancelled),
                    Err(error) => {
                        self.client.on_error(Some(id));
                        Err(SourceError::from(error))
                    }
                }
            }
            Action::WaitMs(ms) => {
                // Downloading and draining segments already spends part (or all) of
                // the reload interval. Do not add that time again as a fresh sleep.
                let remaining = reload_delay(*ms, self.playlist_received_at.elapsed());
                cancellable_sleep(remaining, &self.cancel)
                    .await
                    .map_err(SourceError::from)
            }
            // `Action` is `#[non_exhaustive]`: a future variant this driver
            // does not yet know how to service is skipped, never a hard
            // failure - matches the plan's "distinguish incomplete input
            // from definitive unsupported input" spirit for the action
            // stream too.
            _ => Ok(()),
        }
    }
}

/// hls-runtime 0.6 queues resources, then FetchPlaylist, then WaitMs. Servicing
/// that literally leaves the old wait ahead of the *new* playlist's resources.
/// Pace the reload instead, so newly discovered audio is fetched immediately.
fn scheduled_actions(client: &mut HlsClient) -> VecDeque<Action> {
    let mut actions = VecDeque::new();
    while let Some(action) = client.poll() {
        if matches!(action, Action::WaitMs(_))
            && matches!(actions.back(), Some(Action::FetchPlaylist { .. }))
        {
            let reload = actions.pop_back().unwrap();
            actions.push_back(action);
            actions.push_back(reload);
        } else {
            actions.push_back(action);
        }
    }
    actions
}

fn reload_delay(interval_ms: u64, elapsed: Duration) -> Duration {
    Duration::from_millis(interval_ms).saturating_sub(elapsed)
}

/// Rejects a playlist body carrying `#EXT-X-KEY` before it can reach
/// `HlsClient::on_playlist`, which has no encryption policy of its own to
/// enforce this. See the callers in [`HlsSource::from_initial_playlist`]
/// and [`HlsSource::service`].
fn reject_if_encrypted(body: &[u8]) -> Result<(), SourceError> {
    if is_encrypted_playlist(body) {
        return Err(SourceError::from(Unsupported::EncryptedPlaylist));
    }
    Ok(())
}

fn pipeline_error_to_source(error: PipelineError) -> SourceError {
    // `PipelineError`'s messages never embed a URL (decode/demux failure
    // text, track-configuration text), so no redaction pass is needed here
    // - unlike the hls-runtime `Error`s above, which can carry a resolved
    // resource URL in `UriResolve`/`ByteRangeOverflow`.
    SourceError::Demux(error.to_string())
}

/// `byte_range` support: `NetworkService` has no partial-content (HTTP
/// `Range`) request path (see `network.rs`), so this fetches the resource
/// whole and slices locally. Correct, not bandwidth-optimal. The target
/// provider's classic (non-LL) HLS media playlists never carry
/// `EXT-X-BYTERANGE` (see `docs/symphonia-migration-progress.md`'s live
/// provider re-check), so no real fixture reaches this path. It is covered by
/// unit tests in `byte_range_tests` instead.
fn slice_byte_range(bytes: &[u8], offset: u64, length: u64) -> Result<Vec<u8>, SourceError> {
    let start = usize::try_from(offset)
        .map_err(|_| SourceError::Demux("byte-range offset overflowed usize".to_string()))?;
    let len = usize::try_from(length)
        .map_err(|_| SourceError::Demux("byte-range length overflowed usize".to_string()))?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| SourceError::Demux("byte-range end overflowed usize".to_string()))?;
    bytes
        .get(start..end)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| SourceError::Demux("byte-range exceeded the fetched resource".to_string()))
}

#[cfg(test)]
mod byte_range_tests {
    use super::*;

    #[test]
    fn a_valid_range_returns_exactly_those_bytes() {
        let data: Vec<u8> = (0..=255u8).collect();
        assert_eq!(
            slice_byte_range(&data, 10, 4).unwrap(),
            vec![10, 11, 12, 13]
        );
        assert_eq!(slice_byte_range(&data, 0, 1).unwrap(), vec![0]);
        // Exactly reaching the end is valid, not an overrun.
        assert_eq!(slice_byte_range(&data, 254, 2).unwrap(), vec![254, 255]);
    }

    #[test]
    fn a_zero_length_range_is_empty_rather_than_an_error() {
        let data = vec![1u8, 2, 3];
        assert_eq!(slice_byte_range(&data, 1, 0).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn a_range_past_the_end_is_rejected_instead_of_truncating() {
        let data = vec![1u8, 2, 3, 4];
        // Silently returning a short read here would hand the demuxer a
        // truncated access unit, which is far worse than a clear error.
        assert!(slice_byte_range(&data, 2, 5).is_err());
        assert!(slice_byte_range(&data, 4, 1).is_err());
        assert!(slice_byte_range(&data, 99, 1).is_err());
    }

    #[test]
    fn arithmetic_overflow_is_rejected_rather_than_wrapping() {
        let data = vec![1u8, 2, 3];
        assert!(slice_byte_range(&data, u64::MAX, 1).is_err());
        assert!(slice_byte_range(&data, 1, u64::MAX).is_err());
    }
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;

    fn client() -> HlsClient {
        let mut client = HlsClient::new("http://localhost/live.m3u8");
        assert!(matches!(client.poll(), Some(Action::FetchPlaylist { .. })));
        client
    }

    #[test]
    fn live_reload_wait_precedes_reload_and_never_new_segments() {
        let mut client = client();
        for sequence in 0..3 {
            let playlist = format!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:{sequence}\n#EXTINF:6,\n{sequence}.ts\n"
            );
            client.on_playlist(playlist.as_bytes()).unwrap();
            let mut actions = scheduled_actions(&mut client);
            let Some(Action::FetchResource { id, .. }) = actions.pop_front() else {
                panic!("new audio must be fetched before waiting or reloading");
            };
            client
                .on_resource(
                    id,
                    include_bytes!("../../../playback-core/tests/fixtures/aac-0.ts"),
                )
                .unwrap();
            assert!(matches!(actions.pop_front(), Some(Action::WaitMs(3000))));
            assert!(matches!(
                actions.pop_front(),
                Some(Action::FetchPlaylist { .. })
            ));
            assert!(actions.is_empty());
        }
    }

    #[test]
    fn time_spent_delivering_audio_counts_toward_the_reload_interval() {
        assert_eq!(
            reload_delay(3000, Duration::from_millis(1250)),
            Duration::from_millis(1750)
        );
        assert_eq!(reload_delay(3000, Duration::from_secs(3)), Duration::ZERO);
        assert_eq!(reload_delay(3000, Duration::from_secs(8)), Duration::ZERO);
    }

    #[test]
    fn ended_playlists_have_no_reload_or_wait() {
        let mut client = client();
        client
            .on_playlist(b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\n0.ts\n#EXT-X-ENDLIST\n")
            .unwrap();
        let mut actions = scheduled_actions(&mut client);
        assert!(matches!(
            actions.pop_front(),
            Some(Action::FetchResource { .. })
        ));
        assert!(actions.is_empty());
    }

    #[test]
    fn blocking_reload_has_no_added_wait() {
        let mut client = client();
        client.on_playlist(b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES\n#EXT-X-PART-INF:PART-TARGET=1.0\n#EXTINF:6,\n0.ts\n").unwrap();
        let actions = scheduled_actions(&mut client);
        assert!(!actions
            .iter()
            .any(|action| matches!(action, Action::WaitMs(_))));
        assert!(matches!(
            actions.back(),
            Some(Action::FetchPlaylist {
                blocking: Some(_),
                ..
            })
        ));
    }
}
