mod media_session;
mod mpv;
mod secrets;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_http::init())
    .plugin(tauri_plugin_store::Builder::default().build())
    .manage(mpv::MpvState::default())
    .invoke_handler(tauri::generate_handler![
      mpv::mpv_load,
      mpv::mpv_stop,
      mpv::mpv_set_volume,
      mpv::mpv_get_property,
      secrets::secrets_set,
      secrets::secrets_get,
      secrets::secrets_delete,
      media_session::media_session_set_metadata,
      media_session::media_session_set_playback,
    ])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }

      match media_session::init(&app.handle()) {
        Ok(controls) => {
          app.manage(media_session::MediaSessionState(std::sync::Mutex::new(Some(
            controls,
          ))));
        }
        Err(e) => {
          log::warn!("failed to initialize OS media session: {e}");
          app.manage(media_session::MediaSessionState::default());
        }
      }

      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
