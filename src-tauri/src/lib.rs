mod logs;
mod media_session;
mod mpv;
mod secrets;
mod updater;
mod waveform;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_http::init())
    .plugin(tauri_plugin_store::Builder::default().build())
    .plugin(tauri_plugin_process::init())
    .plugin(tauri_plugin_dialog::init())
    .manage(mpv::MpvState::default())
    .invoke_handler(tauri::generate_handler![
      mpv::mpv_load,
      mpv::mpv_stop,
      mpv::mpv_set_volume,
      mpv::mpv_get_property,
      mpv::mpv_get_stderr_tail,
      secrets::secrets_set,
      secrets::secrets_get,
      secrets::secrets_delete,
      media_session::media_session_set_metadata,
      media_session::media_session_set_playback,
      updater::check_update_at_endpoint,
      updater::download_and_install_update,
      logs::export_log_file,
    ])
    .setup(|app| {
      app.handle().plugin(
        tauri_plugin_log::Builder::default()
          .level(log::LevelFilter::Info)
          .build(),
      )?;

      #[cfg(desktop)]
      app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;

      // Ensures mpv can never outlive Apogee on Windows, regardless of how
      // this process exits (crash, force-quit, or the updater's
      // `std::process::exit`) - see `mpv::create_process_job`. The returned
      // `Job` must be kept alive for the process lifetime, so it's stashed in
      // managed state rather than dropped at the end of this closure.
      // macOS has no equivalent primitive - see the comment in
      // `mpv::spawn_mpv` for why that gap is accepted rather than solved.
      #[cfg(windows)]
      if let Some(job) = mpv::create_process_job() {
        app.manage(job);
      }

      waveform::ensure_started(&app.handle());

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
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(|app_handle, event| {
      if let tauri::RunEvent::Exit = event {
        mpv::kill_on_exit(&app_handle.state::<mpv::MpvState>());
      }
    });
}
