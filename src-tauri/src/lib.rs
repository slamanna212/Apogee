mod discord_rpc;
mod lastfm;
mod logs;
mod media_session;
mod mpv;
mod notifications;
mod secrets;
mod updater;
mod waveform;
mod window_state;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(mpv::MpvState::default())
        .manage(discord_rpc::DiscordRpcState::default())
        .invoke_handler(tauri::generate_handler![
            mpv::mpv_load,
            mpv::mpv_stop,
            mpv::mpv_set_volume,
            mpv::mpv_set_property,
            mpv::mpv_get_property,
            mpv::mpv_list_audio_devices,
            mpv::mpv_get_stderr_tail,
            waveform::waveform_set_device,
            secrets::secrets_set,
            secrets::secrets_get,
            secrets::secrets_delete,
            secrets::secrets_get_builtin_stellar_key,
            lastfm::lastfm_connection_status,
            lastfm::lastfm_begin_auth,
            lastfm::lastfm_complete_auth,
            lastfm::lastfm_disconnect,
            lastfm::lastfm_update_now_playing,
            lastfm::lastfm_scrobble,
            media_session::media_session_set_metadata,
            media_session::media_session_set_playback,
            media_session::media_session_set_volume,
            notifications::ensure_os_notification_permission,
            notifications::send_os_notification,
            discord_rpc::discord_rpc_connect,
            discord_rpc::discord_rpc_set_activity,
            discord_rpc::discord_rpc_clear_activity,
            discord_rpc::discord_rpc_disconnect,
            updater::check_update_at_endpoint,
            updater::download_and_install_update,
            logs::export_log_file,
            logs::set_log_level,
            window_state::set_window_bounds,
        ])
        .setup(|app| {
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    // The dispatch filter is fixed once at build time, so this is
                    // deliberately permissive (Debug) - the actual default runtime
                    // level is set right below via log::set_max_level(Info), and
                    // logs::set_log_level flips that global gate at runtime when the
                    // user turns on verbose logging in Settings to chase down
                    // intermittent playback issues (e.g. on Mac).
                    .level(log::LevelFilter::Debug)
                    // Defaults (40 KB / KeepOne) rotate by deleting the log outright,
                    // which can wipe the exact window a playback error happened in
                    // before a user gets a chance to download it. Keep more headroom
                    // and history instead.
                    .max_file_size(5 * 1024 * 1024)
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                    .build(),
            )?;
            log::set_max_level(log::LevelFilter::Info);

            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

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
                    app.manage(media_session::MediaSessionState(std::sync::Mutex::new(
                        Some(controls),
                    )));
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
                discord_rpc::clear_on_exit(&app_handle.state::<discord_rpc::DiscordRpcState>());
            }
        });
}
