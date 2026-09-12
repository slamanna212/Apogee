//! Platform-independent playback pipeline, validated before application cutover.

pub mod analysis;
pub mod decode;
pub mod detect;
pub mod dsp;
pub mod output;
pub mod pipeline;
pub mod session;

#[cfg(test)]
mod tests {
    use hls_runtime::client::{Action, HlsClient, Output};

    #[test]
    fn hls_client_requests_playlist_without_owning_http() {
        let mut client = HlsClient::new("https://example.invalid/live.m3u8");
        assert!(matches!(client.poll(), Some(Action::FetchPlaylist { .. })));
        assert!(client.next_output().is_none());
    }

    #[test]
    fn hls_and_direct_ts_share_the_same_sample_type() {
        let output = Output::Samples {
            track_id: 1,
            samples: Vec::<transmux::Sample>::new(),
        };
        assert!(matches!(output, Output::Samples { track_id: 1, .. }));
        let mut direct = transmux::StreamingTsDemux::new();
        direct.feed(&[]);
        assert!(direct.poll_event().is_none());
    }
}
