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
    // Default panic behavior prints to stderr, which is invisible once the app is
    // launched as a bundled binary (not from a terminal) - route panics into the
    // rotating log file instead so they're visible in exported logs.
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));

    let app = tauri::Builder::default()
        // Install the file logger before the other plugins and before setup so
        // failures in the rest of application initialization leave a useful
        // last-known startup marker.
        .plugin(
            tauri_plugin_log::Builder::default()
                // The dispatch filter is fixed once at build time, so this is
                // deliberately permissive (Debug) - the actual default runtime
                // level is set in setup via log::set_max_level(Info).
                .level(log::LevelFilter::Debug)
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
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
            mpv::mpv_set_equalizer,
            mpv::mpv_set_property,
            mpv::mpv_get_property,
            mpv::mpv_list_audio_devices,
            mpv::mpv_get_stderr_tail,
            waveform::waveform_set_device,
            waveform::waveform_set_active,
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
            log::set_max_level(log::LevelFilter::Info);
            log::info!(
                "startup: logger initialized; version={}, os={}, arch={}",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );

            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            log::info!("startup: desktop updater initialized");

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

            #[cfg(not(target_os = "macos"))]
            waveform::ensure_started(&app.handle());
            #[cfg(target_os = "macos")]
            log::info!("startup: macOS waveform capture deferred until first playback");

            match media_session::init(&app.handle()) {
                Ok(controls) => {
                    app.manage(media_session::MediaSessionState(std::sync::Mutex::new(
                        Some(controls),
                    )));
                    log::info!("startup: OS media session initialized");
                }
                Err(e) => {
                    log::warn!("failed to initialize OS media session: {e}");
                    app.manage(media_session::MediaSessionState::default());
                }
            }

            log::info!("startup: application setup complete");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    log::info!("startup: entering application event loop");
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            mpv::kill_on_exit(&app_handle.state::<mpv::MpvState>());
            discord_rpc::clear_on_exit(&app_handle.state::<discord_rpc::DiscordRpcState>());
        }
    });
}
