//! StellarTunerLog API client, as typed Tauri commands.
//!
//! The endpoints differ in authentication and that difference is deliberate, not an
//! oversight: `/nowplaying` and `/channels` are keyless, `/history` requires an API key.
//! Preserved exactly, so a missing key degrades to "no play history" rather than breaking
//! now-playing metadata for everyone.
//!
//! Responses pass through as raw JSON so the existing TypeScript types and their tests keep
//! working unchanged.

use serde_json::Value;
use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::network::{NetworkError, NetworkService};

const NOWPLAYING_URL: &str = "https://api.stellartunerlog.com/v1/nowplaying";
const CHANNELS_URL: &str = "https://api.stellartunerlog.com/v1/channels";
const HISTORY_BASE: &str = "https://api.stellartunerlog.com/v1/history/";

/// The API key travels in a header, so it never appears in a URL. Errors are still mapped
/// to fixed strings rather than formatted from the transport error.
fn describe(endpoint: &str, error: &NetworkError) -> String {
    match error {
        NetworkError::Status { status, detail } => match detail {
            Some(detail) => format!(
                "StellarTunerLog {endpoint} failed: {detail} (HTTP {})",
                status.as_u16()
            ),
            None => format!(
                "StellarTunerLog {endpoint} failed: HTTP {}",
                status.as_u16()
            ),
        },
        NetworkError::Cancelled => format!("StellarTunerLog {endpoint} was cancelled"),
        NetworkError::BodyTooLarge { .. } => {
            format!("StellarTunerLog {endpoint} failed: response was unexpectedly large")
        }
        _ => format!("StellarTunerLog {endpoint} failed: could not reach the server"),
    }
}

async fn get_json(
    network: &NetworkService,
    endpoint: &str,
    url: &str,
    api_key: Option<&str>,
) -> Result<Value, String> {
    let cancel = CancellationToken::new();
    let headers: Vec<(&str, &str)> = match api_key {
        Some(key) if !key.is_empty() => vec![("X-API-Key", key)],
        _ => Vec::new(),
    };
    let body = network
        .fetch_json_with_headers(url, &headers, &cancel)
        .await
        .map_err(|e| {
            log::warn!("stellar {endpoint} request failed: {e}");
            describe(endpoint, &e)
        })?;
    serde_json::from_slice(&body.bytes)
        .map_err(|_| format!("StellarTunerLog {endpoint} returned invalid JSON"))
}

#[tauri::command]
pub async fn stellar_now_playing(
    network: State<'_, NetworkService>,
    api_key: Option<String>,
) -> Result<Value, String> {
    get_json(&network, "/nowplaying", NOWPLAYING_URL, api_key.as_deref()).await
}

#[tauri::command]
pub async fn stellar_channels(network: State<'_, NetworkService>) -> Result<Value, String> {
    get_json(&network, "/channels", CHANNELS_URL, None).await
}

#[tauri::command]
pub async fn stellar_history(
    network: State<'_, NetworkService>,
    channel_id: String,
    api_key: String,
) -> Result<Value, String> {
    let url = history_url(&channel_id)?;
    get_json(&network, "/history", &url, Some(&api_key)).await
}

/// Builds the history URL, escaping the channel id so it cannot escape its path segment.
fn history_url(channel_id: &str) -> Result<String, String> {
    if channel_id.is_empty() {
        return Err("StellarTunerLog /history needs a channel id".to_string());
    }
    let encoded: String = channel_id
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    Ok(format!("{HISTORY_BASE}{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_id_cannot_escape_its_path_segment() {
        let url = history_url("../../admin").unwrap();
        // Dots are harmless on their own; it is the separators that would traverse, so the
        // test that matters is that no unescaped slash survives past the base.
        let tail = url
            .strip_prefix(HISTORY_BASE)
            .expect("must stay under the base path");
        assert!(!tail.contains('/'), "channel id escaped its segment: {url}");
        assert_eq!(tail, "..%2F..%2Fadmin");
    }

    #[test]
    fn a_channel_id_cannot_inject_a_query_string_or_fragment() {
        let tail = history_url("x?admin=1#frag")
            .unwrap()
            .strip_prefix(HISTORY_BASE)
            .unwrap()
            .to_string();
        assert!(!tail.contains('?') && !tail.contains('#'), "{tail}");
    }

    #[test]
    fn ordinary_channel_ids_pass_through_unchanged() {
        assert_eq!(
            history_url("siriusxmhits1").unwrap(),
            format!("{HISTORY_BASE}siriusxmhits1")
        );
    }

    #[test]
    fn an_empty_channel_id_is_rejected() {
        assert!(history_url("").is_err());
    }

    #[test]
    fn errors_never_contain_a_url_or_a_key() {
        let error = NetworkError::Transport(
            "error sending request for url (https://api.stellartunerlog.com/v1/history/x)".into(),
        );
        let message = describe("/history", &error);
        assert!(!message.contains("http"), "leaked a URL: {message}");
    }

    #[test]
    fn status_codes_are_preserved_because_401_versus_500_matters_to_the_user() {
        let error = NetworkError::Status {
            status: reqwest::StatusCode::UNAUTHORIZED,
            detail: None,
        };
        assert_eq!(
            describe("/history", &error),
            "StellarTunerLog /history failed: HTTP 401"
        );
    }
}
