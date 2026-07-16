use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::secrets;

const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
const AUTH_URL: &str = "https://www.last.fm/api/auth/";
// Last.fm API keys are public application identifiers and are included in
// every request and browser authorization URL. Only the shared secret is
// supplied through the build environment.
const API_KEY: &str = "f9fb01bda540720fa7339cc5e0a7d5b5";
const SESSION_KEYRING_KEY: &str = "lastfm_session_key";
const USERNAME_KEYRING_KEY: &str = "lastfm_username";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFmError {
    code: Option<i64>,
    message: String,
    retryable: bool,
}

impl LastFmError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            retryable: false,
        }
    }

    fn transport(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            retryable: true,
        }
    }

    fn api(code: i64, message: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            message: message.into(),
            retryable: code == 11 || code == 16,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFmConnectionStatus {
    available: bool,
    connected: bool,
    username: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFmAuthStart {
    token: String,
    authorization_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFmTrack {
    artist: String,
    title: String,
    album: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFmScrobble {
    artist: String,
    title: String,
    album: Option<String>,
    started_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFmScrobbleResult {
    accepted: bool,
    ignored_code: i64,
    ignored_message: Option<String>,
}

fn shared_secret() -> Result<&'static str, LastFmError> {
    option_env!("LASTFM_SHARED_SECRET")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LastFmError::unavailable("Last.fm support is not configured in this build"))
}

fn signature(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut input = String::new();
    for (name, value) in params {
        input.push_str(name);
        input.push_str(value);
    }
    input.push_str(secret);
    format!("{:x}", md5::compute(input.as_bytes()))
}

fn keyring_error(error: String) -> LastFmError {
    LastFmError::unavailable(format!(
        "Could not access secure Last.fm credentials: {error}"
    ))
}

fn api_error(value: &Value) -> Option<LastFmError> {
    let code = value.get("error")?.as_i64()?;
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Last.fm rejected the request");
    Some(LastFmError::api(code, message))
}

async fn call(mut params: BTreeMap<String, String>) -> Result<Value, LastFmError> {
    let secret = shared_secret()?;
    params.insert("api_key".into(), API_KEY.into());
    let api_sig = signature(&params, secret);
    params.insert("api_sig".into(), api_sig);
    params.insert("format".into(), "json".into());

    let response = reqwest::Client::new()
        .post(API_URL)
        .form(&params)
        .send()
        .await
        .map_err(|error| LastFmError::transport(format!("Could not reach Last.fm: {error}")))?;
    let status = response.status();
    let value = response.json::<Value>().await.map_err(|_| {
        LastFmError::transport(format!(
            "Last.fm returned an unreadable response ({status})"
        ))
    })?;

    if let Some(error) = api_error(&value) {
        return Err(error);
    }
    if !status.is_success() {
        return Err(LastFmError::transport(format!(
            "Last.fm returned HTTP {status}"
        )));
    }
    Ok(value)
}

async fn authenticated_call(mut params: BTreeMap<String, String>) -> Result<Value, LastFmError> {
    let session = secrets::get(SESSION_KEYRING_KEY)
        .map_err(keyring_error)?
        .ok_or_else(|| LastFmError::api(9, "Connect a Last.fm account first"))?;
    params.insert("sk".into(), session);
    let result = call(params).await;
    if matches!(&result, Err(error) if error.code == Some(9)) {
        let _ = secrets::delete(SESSION_KEYRING_KEY);
        let _ = secrets::delete(USERNAME_KEYRING_KEY);
    }
    result
}

fn track_params(method: &str, track: LastFmTrack) -> BTreeMap<String, String> {
    let mut params = BTreeMap::from([
        ("method".into(), method.into()),
        ("artist".into(), track.artist),
        ("track".into(), track.title),
    ]);
    if let Some(album) = track.album.filter(|value| !value.trim().is_empty()) {
        params.insert("album".into(), album);
    }
    params
}

#[tauri::command]
pub fn lastfm_connection_status() -> Result<LastFmConnectionStatus, LastFmError> {
    let available = shared_secret().is_ok();
    let session = secrets::get(SESSION_KEYRING_KEY).map_err(keyring_error)?;
    let username = secrets::get(USERNAME_KEYRING_KEY).map_err(keyring_error)?;
    Ok(LastFmConnectionStatus {
        available,
        connected: session.is_some(),
        username,
    })
}

#[tauri::command]
pub async fn lastfm_begin_auth() -> Result<LastFmAuthStart, LastFmError> {
    let params = BTreeMap::from([("method".into(), "auth.getToken".into())]);
    let response = call(params).await?;
    let token = response
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| LastFmError::transport("Last.fm did not return an authorization token"))?
        .to_string();
    let mut url = url::Url::parse(AUTH_URL).expect("constant Last.fm auth URL must be valid");
    url.query_pairs_mut()
        .append_pair("api_key", API_KEY)
        .append_pair("token", &token);
    Ok(LastFmAuthStart {
        token,
        authorization_url: url.into(),
    })
}

#[tauri::command]
pub async fn lastfm_complete_auth(token: String) -> Result<LastFmConnectionStatus, LastFmError> {
    let params = BTreeMap::from([
        ("method".into(), "auth.getSession".into()),
        ("token".into(), token),
    ]);
    let response = call(params).await?;
    let session = response
        .get("session")
        .ok_or_else(|| LastFmError::transport("Last.fm did not return a session"))?;
    let username = session
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| LastFmError::transport("Last.fm did not return an account name"))?;
    let key = session
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| LastFmError::transport("Last.fm did not return a session key"))?;

    secrets::set(SESSION_KEYRING_KEY, key).map_err(keyring_error)?;
    secrets::set(USERNAME_KEYRING_KEY, username).map_err(keyring_error)?;
    Ok(LastFmConnectionStatus {
        available: true,
        connected: true,
        username: Some(username.to_string()),
    })
}

#[tauri::command]
pub fn lastfm_disconnect() -> Result<(), LastFmError> {
    secrets::delete(SESSION_KEYRING_KEY).map_err(keyring_error)?;
    secrets::delete(USERNAME_KEYRING_KEY).map_err(keyring_error)
}

#[tauri::command]
pub async fn lastfm_update_now_playing(track: LastFmTrack) -> Result<(), LastFmError> {
    authenticated_call(track_params("track.updateNowPlaying", track)).await?;
    Ok(())
}

#[tauri::command]
pub async fn lastfm_scrobble(
    scrobble: LastFmScrobble,
) -> Result<LastFmScrobbleResult, LastFmError> {
    let LastFmScrobble {
        artist,
        title,
        album,
        started_at,
    } = scrobble;
    let mut params = track_params(
        "track.scrobble",
        LastFmTrack {
            artist,
            title,
            album,
        },
    );
    params.insert("timestamp".into(), started_at.to_string());
    params.insert("chosenByUser".into(), "0".into());
    let response = authenticated_call(params).await?;
    let scrobble = response
        .pointer("/scrobbles/scrobble")
        .ok_or_else(|| LastFmError::transport("Last.fm did not return a scrobble result"))?;
    let ignored = scrobble
        .get("ignoredMessage")
        .or_else(|| scrobble.get("ignoredmessage"));
    let ignored_code = ignored
        .and_then(|value| value.get("code"))
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0);
    let ignored_message = ignored
        .and_then(|value| value.get("#text"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(LastFmScrobbleResult {
        accepted: ignored_code == 0,
        ignored_code,
        ignored_message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_sorts_parameters_and_appends_secret() {
        let params = BTreeMap::from([
            ("method".into(), "auth.getSession".into()),
            ("token".into(), "token".into()),
            ("api_key".into(), "key".into()),
        ]);
        assert_eq!(
            signature(&params, "secret"),
            format!(
                "{:x}",
                md5::compute(b"api_keykeymethodauth.getSessiontokentokensecret")
            )
        );
    }

    #[test]
    fn retryability_follows_lastfm_error_codes() {
        assert!(LastFmError::api(11, "offline").retryable);
        assert!(LastFmError::api(16, "temporary").retryable);
        assert!(!LastFmError::api(9, "session").retryable);
        assert!(!LastFmError::api(6, "parameters").retryable);
    }
}
