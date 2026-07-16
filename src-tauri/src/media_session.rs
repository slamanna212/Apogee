use serde::Serialize;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};
use std::sync::Mutex;
#[cfg(target_os = "windows")]
use tauri::Manager;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct MediaSessionState(pub Mutex<Option<MediaControls>>);

#[derive(Clone, Serialize)]
struct MediaEventPayload {
    kind: &'static str,
    value: Option<f64>,
}

pub fn init(app: &AppHandle) -> Result<MediaControls, String> {
    #[cfg(target_os = "windows")]
    let hwnd: Option<*mut std::ffi::c_void> = {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "missing main window".to_string())?;
        let raw = window.hwnd().map_err(|e| e.to_string())?;
        Some(raw.0 as *mut std::ffi::c_void)
    };
    #[cfg(not(target_os = "windows"))]
    let hwnd: Option<*mut std::ffi::c_void> = None;

    let config = PlatformConfig {
        display_name: "Apogee",
        dbus_name: "apogee",
        hwnd,
    };

    let mut controls = MediaControls::new(config).map_err(|e| e.to_string())?;

    let app_handle = app.clone();
    controls
        .attach(move |event: MediaControlEvent| {
            let payload = match event {
                MediaControlEvent::Play => Some(MediaEventPayload { kind: "play", value: None }),
                MediaControlEvent::Pause => Some(MediaEventPayload { kind: "pause", value: None }),
                MediaControlEvent::Toggle => Some(MediaEventPayload { kind: "toggle", value: None }),
                MediaControlEvent::SetVolume(v) => Some(MediaEventPayload { kind: "volume", value: Some(v) }),
                // Next/Previous/Seek/etc. have no meaning for a live radio tuner - ignored intentionally.
                _ => None,
            };
            if let Some(payload) = payload {
                let _ = app_handle.emit("media-control-event", payload);
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(controls)
}

#[tauri::command]
pub fn media_session_set_metadata(
    state: State<'_, MediaSessionState>,
    title: String,
    artist: String,
    album: Option<String>,
    cover_url: Option<String>,
) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(controls) = guard.as_mut() {
        controls
            .set_metadata(MediaMetadata {
                title: Some(&title),
                artist: Some(&artist),
                album: album.as_deref(),
                cover_url: cover_url.as_deref(),
                duration: None,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// souvlaki's MediaControls::set_volume only exists on the Linux/mpris
// platform backend (macOS/Windows/empty backends have no such method) - the
// MPRIS spec requires the app to echo volume changes back to the media
// widget after every change, not just ones that originated from it.
#[cfg(target_os = "linux")]
#[tauri::command]
pub fn media_session_set_volume(state: State<'_, MediaSessionState>, volume: f64) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(controls) = guard.as_mut() {
        controls.set_volume(volume).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub fn media_session_set_volume(_state: State<'_, MediaSessionState>, _volume: f64) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub fn media_session_set_playback(
    state: State<'_, MediaSessionState>,
    playing: bool,
) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(controls) = guard.as_mut() {
        let playback = if playing {
            MediaPlayback::Playing { progress: None }
        } else {
            MediaPlayback::Stopped
        };
        controls.set_playback(playback).map_err(|e| e.to_string())?;
    }
    Ok(())
}
