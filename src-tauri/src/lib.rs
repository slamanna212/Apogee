mod discord_rpc;
mod lastfm;
mod logs;
mod media_session;
// Shared networking foundation for the in-process audio engine (Symphonia
// migration M1). Not wired into any Tauri command yet - nothing outside its
// own tests calls it until M2 builds the playback engine on top of it, so
// its public API is intentionally unused for now.
#[allow(dead_code)]
mod network;
mod notifications;
// Rust-native audio source pipeline (Symphonia migration M2), built on top
// of `network`. Not wired into any Tauri command yet - nothing outside its
// own tests calls it until M3 builds the controller on top of it.
mod playback;
mod secrets;
mod stellar;
mod updater;
mod window_state;
mod xtream;

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
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(
            playback::commands::PlayerState::new().expect("audio playback state should initialise"),
        )
        // One HTTP layer for the whole app. Shared with the playback engine so stream and
        // API traffic use the same TLS, redirect and redaction policy.
        .manage(network::NetworkService::new().expect("network service should initialise"))
        .manage(discord_rpc::DiscordRpcState::default())
        .invoke_handler(tauri::generate_handler![
            playback::commands::player_play,
            playback::commands::player_stop,
            playback::commands::player_set_volume,
            playback::commands::player_set_muted,
            playback::commands::player_set_equalizer,
            playback::commands::player_list_devices,
            playback::commands::player_set_device,
            playback::commands::player_migrate_device,
            playback::commands::player_set_visualizer,
            playback::commands::player_get_snapshot,
            xtream::xtream_get_live_categories,
            xtream::xtream_get_live_streams,
            stellar::stellar_now_playing,
            stellar::stellar_channels,
            stellar::stellar_history,
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
            updater::github_releases,
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

            // No Windows Job Object any more. It existed solely to guarantee the mpv
            // subprocess died with Apogee; audio is now decoded in-process, so there is
            // no child process to outlive us and nothing to contain.

            match media_session::init(app.handle()) {
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
            // Playback owns no subprocess now; dropping the player state stops the audio
            // thread and cancels in-flight network work on its own.
            discord_rpc::clear_on_exit(&app_handle.state::<discord_rpc::DiscordRpcState>());
        }
    });
}
