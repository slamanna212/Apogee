//! Tauri adapter: typed commands and event translation.
//!
//! The engine and controller know nothing about Tauri. This module is the only place that
//! touches `AppHandle`, converting domain events into the `player-snapshot` event the
//! frontend consumes.
//!
//! Commands are typed rather than arbitrary property strings, so the frontend can no longer
//! reach into player internals the way it did with MPV's property protocol.

use std::sync::{Arc, Mutex};

use apogee_playback_core::session::{Controller, Generation, Next, Snapshot};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use super::device::{list_output_devices, migrate_mpv_device, DeviceDescriptor, DeviceRequest};
use super::engine::{start_session, AudioSettings, EngineEvent, EventSink, Session};
use crate::network::NetworkService;

/// Enough for Rust to build both URL candidates itself. Credentials live here and in the
/// keyring only; they never appear in a snapshot or an event.
///
/// Field names are camelCase on the wire to match the TypeScript caller. Without the
/// rename, Tauri rejects the whole call with a "missing field" error before any of this
/// module runs, so the shape is pinned by a test below.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationRequest {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub stream_id: i64,
    /// Shown in diagnostics and used as the snapshot's station identity.
    pub station_id: String,
}

impl StationRequest {
    /// Builds the stream URL for one attempt, preserving the existing provider path shape
    /// and the alternating extension behaviour.
    fn url_for(&self, extension: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!(
            "{base}/live/{}/{}/{}{extension}",
            urlencoding(&self.username),
            urlencoding(&self.password),
            self.stream_id
        )
    }
}

/// Percent-encodes a path segment. Provider credentials can contain characters that would
/// otherwise change the URL's structure.
fn urlencoding(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

struct Inner {
    controller: Controller,
    session: Option<Session>,
    settings: AudioSettings,
    device: DeviceRequest,
    request: Option<StationRequest>,
}

/// Tauri-managed playback state.
pub struct PlayerState {
    inner: Mutex<Inner>,
    network: NetworkService,
}

impl PlayerState {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            inner: Mutex::new(Inner {
                controller: Controller::new(),
                session: None,
                settings: AudioSettings::default(),
                device: DeviceRequest::SystemDefault,
                request: None,
            }),
            network: NetworkService::new().map_err(|e| e.to_string())?,
        })
    }
}

/// Translates engine events into controller state and a Tauri event.
struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    fn publish(&self, snapshot: Snapshot) {
        // A failed emit means the window is gone; nothing to recover.
        let _ = self.app.emit("player-snapshot", snapshot);
    }
}

impl EventSink for TauriSink {
    fn emit(&self, event: EngineEvent) {
        let state = self.app.state::<PlayerState>();
        let mut retry: Option<(Generation, String)> = None;

        let snapshot = {
            let mut inner = state.inner.lock().expect("player state poisoned");
            match event {
                EngineEvent::Buffering { generation, reason } => {
                    inner.controller.on_buffering(generation, reason);
                }
                EngineEvent::Playing { generation } => {
                    inner.controller.on_audible(generation, now_ms());
                }
                EngineEvent::Format {
                    generation,
                    sample_rate,
                    ..
                } => {
                    inner.controller.set_format(generation, sample_rate);
                }
                EngineEvent::Bitrate { generation, kbps } => {
                    inner.controller.set_bitrate(generation, kbps);
                }
                EngineEvent::Spectrum { levels } => {
                    // Emitted directly rather than through the snapshot: it changes far too
                    // often to belong in playback state, and dropping one is harmless.
                    let _ = self.app.emit("waveform-levels", levels);
                    return;
                }
                EngineEvent::Underrun { .. } => {
                    // Observability only; the ring already emitted silence. Not a state change.
                }
                EngineEvent::Failed {
                    generation,
                    class,
                    message,
                } => {
                    let next = inner
                        .controller
                        .on_error(generation, class, message, now_ms());
                    if let (Some(Next::Retry { extension, .. }), Some(request)) =
                        (next, inner.request.clone())
                    {
                        retry = Some((generation, request.url_for(extension)));
                    }
                }
            }
            inner.controller.snapshot()
        };
        self.publish(snapshot);

        if let Some((generation, url)) = retry {
            schedule_retry(self.app.clone(), generation, url);
        }
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Restarts a failed attempt after the retry delay, if the session is still current.
fn schedule_retry(app: AppHandle, generation: Generation, url: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(
            apogee_playback_core::session::RETRY_DELAY_MS,
        ))
        .await;

        let state = app.state::<PlayerState>();

        // Take the failed session out and tear it down BEFORE reconnecting. The provider
        // allows only a small number of concurrent connections per account, so opening the
        // retry while the previous attempt is still holding one earns a 503 rather than a
        // stream. Dropping happens outside the lock: teardown joins the decode thread, and
        // that thread's event sink takes this same mutex.
        let (previous, settings, device, network) = {
            let mut inner = state.inner.lock().expect("player state poisoned");
            if !inner.controller.accepts(generation) {
                return; // The user moved on; abandon this attempt.
            }
            (
                inner.session.take(),
                inner.settings.clone(),
                inner.device.clone(),
                state.network.clone(),
            )
        };
        drop(previous);

        let sink = Arc::new(TauriSink { app: app.clone() }) as Arc<dyn EventSink>;
        match start_session(generation, url, &device, settings, network, sink) {
            Ok(session) => {
                let mut inner = state.inner.lock().expect("player state poisoned");
                if inner.controller.accepts(generation) {
                    inner.controller.set_device(session.device_name());
                    inner.session = Some(session);
                } // else: `session` drops here, tearing straight back down.
            }
            Err(e) => log::warn!("retry failed to start: {e}"),
        }
    });
}

#[tauri::command]
pub async fn player_play(
    app: AppHandle,
    state: State<'_, PlayerState>,
    request: StationRequest,
) -> Result<Snapshot, String> {
    // Take the previous session out under the lock, but drop it outside: teardown joins
    // threads and must never happen while holding the state lock.
    let (previous, generation, url, settings, device, network, snapshot) = {
        let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
        let previous = inner.session.take();
        let generation = inner.controller.play(request.station_id.clone());
        let url = request.url_for(Controller::extension_for_attempt(0));
        inner.request = Some(request);
        (
            previous,
            generation,
            url,
            inner.settings.clone(),
            inner.device.clone(),
            state.network.clone(),
            inner.controller.snapshot(),
        )
    };
    drop(previous);

    let sink = Arc::new(TauriSink { app }) as Arc<dyn EventSink>;
    let session = start_session(generation, url, &device, settings, network, sink)?;

    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    if inner.controller.accepts(generation) {
        // Track the device actually opened, which may differ from the one requested when
        // a saved device has disappeared.
        inner.controller.set_device(session.device_name());
        inner.session = Some(session);
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn player_stop(state: State<'_, PlayerState>) -> Result<Snapshot, String> {
    let (previous, snapshot) = {
        let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
        let previous = inner.session.take();
        inner.controller.stop();
        inner.request = None;
        (previous, inner.controller.snapshot())
    };
    drop(previous);
    Ok(snapshot)
}

#[tauri::command]
pub async fn player_set_volume(state: State<'_, PlayerState>, volume: u8) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.volume = volume.min(100);
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_set_muted(state: State<'_, PlayerState>, muted: bool) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.muted = muted;
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_set_equalizer(
    state: State<'_, PlayerState>,
    enabled: bool,
    gains: Vec<f64>,
) -> Result<(), String> {
    if gains.len() != apogee_playback_core::dsp::BANDS.len() {
        return Err(format!(
            "equalizer requires exactly {} bands",
            apogee_playback_core::dsp::BANDS.len()
        ));
    }
    let mut array = [0.0f64; 10];
    array.copy_from_slice(&gains);

    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.equalizer_enabled = enabled;
    inner.settings.equalizer_gains = array;
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_list_devices() -> Result<Vec<DeviceDescriptor>, String> {
    list_output_devices().map_err(|e| e.to_string())
}

/// Select an output device. `None` follows the system default, including later changes.
#[tauri::command]
pub async fn player_set_device(
    state: State<'_, PlayerState>,
    device_id: Option<String>,
) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.device = match device_id {
        Some(id) if !id.is_empty() => DeviceRequest::Specific(id),
        _ => DeviceRequest::SystemDefault,
    };
    Ok(())
}

/// Result of migrating a saved MPV device identifier.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMigration {
    /// The CPAL device id to persist, or `None` to follow the system default.
    ///
    /// The caller **must** persist this. Returning only an explanation would leave the
    /// engine using a migrated device while settings still said "system default", so the
    /// migration would silently undo itself on the next launch.
    pub device_id: Option<String>,
    /// Why the stored value could not be honoured, when it could not.
    pub notice: Option<String>,
}

/// Migrate a saved MPV device identifier to a CPAL device.
#[tauri::command]
pub async fn player_migrate_device(
    state: State<'_, PlayerState>,
    stored: Option<String>,
) -> Result<DeviceMigration, String> {
    let available = list_output_devices().unwrap_or_default();
    let (request, notice) = migrate_mpv_device(stored.as_deref(), &available);
    let device_id = match &request {
        DeviceRequest::Specific(id) => Some(id.clone()),
        DeviceRequest::SystemDefault => None,
    };
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.device = request;
    Ok(DeviceMigration { device_id, notice })
}

/// Enable or disable the spectrum visualiser. Disabled means no FFT work is performed.
#[tauri::command]
pub async fn player_set_visualizer(
    state: State<'_, PlayerState>,
    enabled: bool,
) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.visualizer_enabled = enabled;
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_get_snapshot(state: State<'_, PlayerState>) -> Result<Snapshot, String> {
    let inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    Ok(inner.controller.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> StationRequest {
        StationRequest {
            base_url: "https://example.invalid/".to_string(),
            username: "user name".to_string(),
            password: "p@ss/word".to_string(),
            stream_id: 1_080_037,
            station_id: "1080037".to_string(),
        }
    }

    /// Pins the exact JSON `src/lib/playerClient.ts` sends. A rename on either side
    /// breaks this rather than failing at runtime with "missing field".
    #[test]
    fn the_request_deserialises_from_the_json_the_frontend_actually_sends() {
        let wire = serde_json::json!({
            "baseUrl": "https://example.invalid",
            "username": "alice",
            "password": "secret",
            "streamId": 1_080_037_i64,
            "stationId": "1080037",
        });
        let parsed: StationRequest =
            serde_json::from_value(wire).expect("frontend JSON must deserialise");
        assert_eq!(parsed.base_url, "https://example.invalid");
        assert_eq!(parsed.username, "alice");
        assert_eq!(parsed.stream_id, 1_080_037);
        assert_eq!(parsed.station_id, "1080037");
    }

    #[test]
    fn snake_case_field_names_are_rejected_so_the_contract_stays_unambiguous() {
        let wire = serde_json::json!({
            "base_url": "https://example.invalid",
            "username": "alice",
            "password": "secret",
            "stream_id": 1_i64,
            "station_id": "1",
        });
        assert!(
            serde_json::from_value::<StationRequest>(wire).is_err(),
            "only the camelCase wire shape is supported"
        );
    }

    #[test]
    fn stream_urls_preserve_the_provider_path_shape() {
        let r = request();
        assert_eq!(
            r.url_for(".ts"),
            "https://example.invalid/live/user%20name/p%40ss%2Fword/1080037.ts"
        );
    }

    #[test]
    fn credentials_are_percent_encoded_so_they_cannot_change_the_url_structure() {
        let r = request();
        let url = r.url_for(".ts");
        // A raw slash in the password would otherwise invent an extra path segment.
        assert!(
            !url.contains("p@ss/word"),
            "credentials must be escaped: {url}"
        );
        assert_eq!(
            url.matches('/').count(),
            6,
            "unexpected path depth in {url}"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_up() {
        let mut r = request();
        r.base_url = "https://example.invalid///".to_string();
        assert!(r
            .url_for(".ts")
            .starts_with("https://example.invalid/live/"));
    }

    #[test]
    fn both_extension_candidates_are_reachable_from_the_same_request() {
        let r = request();
        assert!(r
            .url_for(Controller::extension_for_attempt(0))
            .ends_with(".ts"));
        assert!(r
            .url_for(Controller::extension_for_attempt(1))
            .ends_with(".m3u8"));
    }
}
