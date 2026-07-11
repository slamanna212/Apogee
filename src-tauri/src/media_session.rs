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
    display_name: "Pulsar",
    dbus_name: "pulsar",
    hwnd,
  };

  let mut controls = MediaControls::new(config).map_err(|e| e.to_string())?;

  let app_handle = app.clone();
  controls
    .attach(move |event: MediaControlEvent| {
      let kind = match event {
        MediaControlEvent::Play => Some("play"),
        MediaControlEvent::Pause => Some("pause"),
        MediaControlEvent::Toggle => Some("toggle"),
        // Next/Previous/Seek/etc. have no meaning for a live radio tuner - ignored intentionally.
        _ => None,
      };
      if let Some(kind) = kind {
        let _ = app_handle.emit("media-control-event", MediaEventPayload { kind });
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
