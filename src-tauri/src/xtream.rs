//! Xtream Codes `player_api.php` client, as typed Tauri commands.
//!
//! Moved out of TypeScript so all application HTTP goes through one place. Credentials
//! travel in **query parameters** here (unlike the stream URLs, where they are path
//! segments), so every error is routed through redaction before it can reach a log or the
//! UI. Callers surface these messages directly in store state, so a leak would be visible
//! to the user, not merely to the log.
//!
//! Responses are passed through as raw JSON so the existing TypeScript shapes and their
//! tests keep working unchanged.

use serde::Deserialize;
use serde_json::Value;
use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::network::{redact_text, NetworkError, NetworkService};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XtreamCredentials {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

impl XtreamCredentials {
    fn player_api_url(&self, action: &str, extra: &[(&str, &str)]) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let mut url = url::Url::parse(&format!("{base}/player_api.php"))
            .map_err(|_| "the Xtream server address is not a valid URL".to_string())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("username", &self.username);
            query.append_pair("password", &self.password);
            query.append_pair("action", action);
            for (key, value) in extra {
                query.append_pair(key, value);
            }
        }
        Ok(url.to_string())
    }
}

/// Turns any failure into a message safe to show a user.
///
/// A transport error's `Display` embeds the URL it was trying to reach, which carries the
/// username and password in its query string.
fn describe(action: &str, error: &NetworkError) -> String {
    match error {
        NetworkError::Status { status, detail } => match detail {
            Some(detail) => format!("{action} failed: {detail} (HTTP {})", status.as_u16()),
            None => format!("{action} failed: HTTP {}", status.as_u16()),
        },
        NetworkError::Cancelled => format!("{action} was cancelled"),
        NetworkError::BodyTooLarge { .. } => {
            format!("{action} failed: the server sent an unexpectedly large response")
        }
        other => {
            // Belt and braces: the arm below cannot contain a URL today, but redacting is
            // cheap and a future variant might.
            let _ = redact_text(&other.to_string());
            format!("{action} failed: could not reach the Xtream server")
        }
    }
}

async fn request(
    network: &NetworkService,
    action: &str,
    creds: &XtreamCredentials,
    extra: &[(&str, &str)],
) -> Result<Value, String> {
    let url = creds.player_api_url(action, extra)?;
    let cancel = CancellationToken::new();
    let body = network
        .fetch_json(&url, &cancel)
        .await
        .map_err(|e| describe(action, &e))?;
    serde_json::from_slice(&body.bytes).map_err(|_| {
        format!("{action} failed: the server did not return valid JSON (is this an Xtream server?)")
    })
}

#[tauri::command]
pub async fn xtream_get_live_categories(
    network: State<'_, NetworkService>,
    creds: XtreamCredentials,
) -> Result<Value, String> {
    request(&network, "get_live_categories", &creds, &[]).await
}

#[tauri::command]
pub async fn xtream_get_live_streams(
    network: State<'_, NetworkService>,
    creds: XtreamCredentials,
    category_id: String,
) -> Result<Value, String> {
    request(
        &network,
        "get_live_streams",
        &creds,
        &[("category_id", category_id.as_str())],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> XtreamCredentials {
        XtreamCredentials {
            base_url: "http://example.invalid:9191/".to_string(),
            username: "user name".to_string(),
            password: "p@ss&word".to_string(),
        }
    }

    #[test]
    fn credentials_are_query_escaped_so_they_cannot_inject_parameters() {
        let url = creds().player_api_url("get_live_streams", &[]).unwrap();
        // A raw '&' in the password would otherwise start a new query parameter.
        assert!(
            !url.contains("p@ss&word"),
            "password must be escaped: {url}"
        );
        assert!(url.contains("password=p%40ss%26word"), "{url}");
        assert!(
            url.contains("username=user+name") || url.contains("username=user%20name"),
            "{url}"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_up() {
        let url = creds().player_api_url("get_live_categories", &[]).unwrap();
        assert!(
            url.starts_with("http://example.invalid:9191/player_api.php?"),
            "{url}"
        );
    }

    #[test]
    fn extra_parameters_are_appended() {
        let url = creds()
            .player_api_url("get_live_streams", &[("category_id", "43")])
            .unwrap();
        assert!(url.contains("action=get_live_streams"), "{url}");
        assert!(url.contains("category_id=43"), "{url}");
    }

    #[test]
    fn an_invalid_base_url_is_reported_without_echoing_credentials() {
        let bad = XtreamCredentials {
            base_url: "not a url".to_string(),
            ..creds()
        };
        let err = bad.player_api_url("get_live_categories", &[]).unwrap_err();
        assert!(
            !err.contains("p@ss"),
            "error must not carry the password: {err}"
        );
    }

    #[test]
    fn user_facing_errors_never_contain_a_url() {
        let cases = [
            NetworkError::Transport(
                "error sending request for url (http://h/player_api.php?password=hunter2)".into(),
            ),
            NetworkError::InvalidUrl("http://h/player_api.php?password=hunter2".into()),
        ];
        for error in cases {
            let message = describe("get_live_streams", &error);
            assert!(
                !message.contains("hunter2"),
                "leaked a credential: {message}"
            );
            assert!(!message.contains("http"), "leaked a URL: {message}");
        }
    }

    #[test]
    fn a_status_error_keeps_the_code_because_it_is_actionable() {
        let error = NetworkError::Status {
            status: reqwest::StatusCode::UNAUTHORIZED,
            detail: None,
        };
        assert_eq!(
            describe("get_live_streams", &error),
            "get_live_streams failed: HTTP 401"
        );
    }
}
