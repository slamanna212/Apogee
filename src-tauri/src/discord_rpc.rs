use discord_rich_presence::activity::{Activity, ActivityType, Assets, Button, StatusDisplayType};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use std::sync::{Arc, Mutex};
use tauri::State;

/// Apogee's Discord Application (Rich Presence) client ID. Public, not a secret.
const DISCORD_CLIENT_ID: &str = "1526615639786258593";
const GITHUB_URL: &str = "https://github.com/slamanna212/Apogee";
const APP_ICON_URL: &str =
    "https://raw.githubusercontent.com/slamanna212/Apogee/main/src-tauri/icons/icon.png";

#[derive(Clone, Default)]
pub struct DiscordRpcState(pub Arc<Mutex<Option<DiscordIpcClient>>>);

/// Lazily connects if not already connected. Blocking - callers must invoke
/// this from within `spawn_blocking`, mirroring `mpv.rs`'s `ensure_started`.
fn ensure_connected(inner: &Mutex<Option<DiscordIpcClient>>) -> Result<(), String> {
    let mut guard = inner.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(());
    }
    let mut client = DiscordIpcClient::new(DISCORD_CLIENT_ID);
    client.connect().map_err(|e| e.to_string())?;
    *guard = Some(client);
    Ok(())
}

#[tauri::command]
pub async fn discord_rpc_connect(state: State<'_, DiscordRpcState>) -> Result<(), String> {
    let inner = state.0.clone();
    tokio::task::spawn_blocking(move || ensure_connected(&inner))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn discord_rpc_set_activity(
    state: State<'_, DiscordRpcState>,
    details: String,
    activity_state: Option<String>,
    large_image_url: Option<String>,
    large_text: Option<String>,
) -> Result<(), String> {
    let inner = state.0.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        ensure_connected(&inner)?;
        let mut guard = inner.lock().map_err(|e| e.to_string())?;
        let client = guard.as_mut().ok_or_else(|| "not connected".to_string())?;

        let mut assets = Assets::new();
        if let Some(url) = large_image_url.as_deref().filter(|s| !s.is_empty()) {
            assets = assets.large_image(url);
            if let Some(text) = large_text.as_deref().filter(|s| !s.is_empty()) {
                assets = assets.large_text(text);
            }
            // Badges the app's own logo onto the corner of the large image (the
            // track/channel art) - only meaningful paired with a large image.
            assets = assets.small_image(APP_ICON_URL).small_text("Apogee");
        }

        let mut activity = Activity::new()
            .details(&details)
            .assets(assets)
            .activity_type(ActivityType::Listening)
            .status_display_type(StatusDisplayType::State)
            .buttons(vec![Button::new("View on GitHub", GITHUB_URL)]);
        if let Some(s) = activity_state.as_deref().filter(|s| !s.is_empty()) {
            activity = activity.state(s);
        }
        // Deliberately no `.timestamps(...)` - track start/duration isn't known
        // and varies wildly by provider, so no elapsed-time indicator is shown.

        let set_result = client.set_activity(activity);
        if set_result.is_err() {
            // Connection likely dropped (Discord closed) - drop it so the next
            // call reconnects instead of repeatedly failing silently forever.
            *guard = None;
        }
        set_result.map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;

    if let Err(ref e) = result {
        log::warn!("discord rpc: failed to set activity: {e}");
    }
    result
}

#[tauri::command]
pub async fn discord_rpc_clear_activity(state: State<'_, DiscordRpcState>) -> Result<(), String> {
    let inner = state.0.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = inner.lock().map_err(|e| e.to_string())?;
        if let Some(client) = guard.as_mut() {
            let _ = client.clear_activity();
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn discord_rpc_disconnect(state: State<'_, DiscordRpcState>) -> Result<(), String> {
    let inner = state.0.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = inner.lock().map_err(|e| e.to_string())?;
        if let Some(mut client) = guard.take() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Best-effort, synchronous, fire-and-forget teardown for `RunEvent::Exit`,
/// mirroring `mpv::kill_on_exit`'s `try_lock` pattern.
pub fn clear_on_exit(state: &DiscordRpcState) {
    if let Ok(mut guard) = state.0.try_lock() {
        if let Some(mut client) = guard.take() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
    }
}
