#[cfg(windows)]
use crate::mpv::{self, MpvState};
use serde::Serialize;
use tauri::{ipc::Channel, Manager, ResourceId, Runtime, Webview};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    rid: ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    raw_json: serde_json::Value,
}

/// Checks a single, explicitly-provided `latest.json` URL rather than the
/// static endpoint(s) configured in `tauri.conf.json`, so the frontend can
/// resolve which GitHub release to check against per update channel
/// (stable vs. beta) and still reuse the plugin's own signature
/// verification / download / install machinery via the returned resource id.
/// The frontend only ever passes a `browser_download_url` resolved from the
/// GitHub releases API (see `fetchQualifyingReleases`/`findLatestJsonAsset`
/// in src/stores/updateStore.ts), which always resolves to github.com - so
/// there's a fixed host to check even though the exact URL (which release,
/// which channel) is chosen by the frontend at runtime.
const ALLOWED_UPDATE_ENDPOINT_HOST: &str = "github.com";

#[tauri::command]
pub async fn check_update_at_endpoint<R: Runtime>(
    webview: Webview<R>,
    url: String,
) -> Result<Option<Metadata>, String> {
    let endpoint = Url::parse(&url).map_err(|e| e.to_string())?;
    if endpoint.host_str() != Some(ALLOWED_UPDATE_ENDPOINT_HOST) {
        return Err(format!(
            "update endpoint host must be {ALLOWED_UPDATE_ENDPOINT_HOST}"
        ));
    }

    let updater = webview
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;

    let update = updater.check().await.map_err(|e| e.to_string())?;

    let Some(update) = update else {
        return Ok(None);
    };

    let formatted_date = if let Some(date) = update.date {
        Some(
            date.format(&time::format_description::well_known::Rfc3339)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    let metadata = Metadata {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date: formatted_date,
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        rid: webview.resources_table().add(update),
    };

    Ok(Some(metadata))
}

/// Mirrors `tauri_plugin_updater::DownloadEvent`'s wire shape exactly (that
/// type isn't exported by the crate), so the frontend's existing progress
/// handling - written against the plugin's own JS `DownloadEvent` - keeps
/// working unchanged even though the download is now driven from here
/// instead of the plugin's own `downloadAndInstall` command.
#[derive(Serialize, Clone)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum DownloadEvent {
    Started { content_length: Option<u64> },
    Progress { chunk_length: usize },
    Finished,
}

/// Downloads the update identified by `rid` (obtained from
/// `check_update_at_endpoint`) and installs it.
///
/// On every platform except Windows this just downloads the bytes and hands
/// them to the plugin's own `Update::install`, unchanged. On Windows it
/// deliberately bypasses the plugin's `Update::install`/`install_inner`:
/// that installs by calling `ShellExecuteW` (ignoring its return value) and
/// then unconditionally calling `std::process::exit(0)` right after - and
/// because Apogee puts its own process in a Windows Job Object configured to
/// kill everything in the job when the job's last handle closes (see
/// `mpv::create_process_job`), the installer process it just launched joins
/// that same job by default and gets killed in the same instant Apogee exits,
/// before it can even show its progress window. See `install_windows` below
/// for the fix.
#[tauri::command]
pub async fn download_and_install_update<R: Runtime>(
    app_handle: tauri::AppHandle<R>,
    webview: Webview<R>,
    rid: ResourceId,
    on_event: Channel<DownloadEvent>,
) -> Result<(), String> {
    let update = webview
        .resources_table()
        .get::<tauri_plugin_updater::Update>(rid)
        .map_err(|e| e.to_string())?;

    // Mirrors the plugin's own `commands::download` exactly: the crate's
    // `Update::download` calls `on_chunk` for every chunk (not just the
    // first), so `Started` has to be synthesized here on the first call.
    let mut first_chunk = true;
    let bytes = update
        .download(
            |chunk_length, content_length| {
                if first_chunk {
                    first_chunk = false;
                    let _ = on_event.send(DownloadEvent::Started { content_length });
                }
                let _ = on_event.send(DownloadEvent::Progress { chunk_length });
            },
            || {
                let _ = on_event.send(DownloadEvent::Finished);
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        // install_windows does synchronous std::fs::create_dir_all/write of a
        // potentially multi-MB installer - run it on a blocking-pool thread
        // instead of the async runtime's worker thread, matching the
        // spawn_blocking pattern already used for blocking IPC in
        // discord_rpc.rs.
        let version = update.version.clone();
        tokio::task::spawn_blocking(move || install_windows(&app_handle, &version, &bytes))
            .await
            .map_err(|e| format!("update installer task panicked: {e}"))?
    }
    #[cfg(not(windows))]
    {
        let _ = app_handle;
        update.install(bytes).map_err(|e| e.to_string())
    }
}

/// Writes the downloaded installer to a temp file and launches it directly
/// (instead of via the plugin's `ShellExecuteW` call), passing
/// `CREATE_BREAKAWAY_FROM_JOB` so the installer process doesn't join
/// Apogee's Job Object and can survive Apogee's exit - see
/// `mpv::create_process_job` for the other half of this fix (the job must
/// also be created with `JOB_OBJECT_LIMIT_BREAKAWAY_OK` for this flag to do
/// anything).
///
/// `/P /R` reproduces the plugin's documented default `Passive` install mode
/// (Apogee doesn't override `plugins.updater.windows.installMode`); `/UPDATE`
/// is what the plugin adds unconditionally too. Only kills mpv and exits
/// Apogee *after* confirming the installer process actually started - unlike
/// the plugin's own path, a launch failure here is returned as a normal
/// error instead of the app silently vanishing.
#[cfg(windows)]
fn install_windows<R: Runtime>(
    app_handle: &tauri::AppHandle<R>,
    version: &str,
    bytes: &[u8],
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let temp_dir = std::env::temp_dir().join(format!("apogee-updater-{version}"));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("couldn't create temp dir for the update installer: {e}"))?;
    let installer_path = temp_dir.join(format!("Apogee_{version}_x64-setup.exe"));
    std::fs::write(&installer_path, bytes)
        .map_err(|e| format!("couldn't write the update installer to disk: {e}"))?;

    log::info!("launching update installer at {installer_path:?}");

    match std::process::Command::new(&installer_path)
        .args(["/P", "/R", "/UPDATE"])
        .creation_flags(CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Ok(child) => {
            log::info!("update installer launched (pid {})", child.id());
            mpv::kill_blocking(&app_handle.state::<MpvState>());
            app_handle.cleanup_before_exit();
            app_handle.exit(0);
            Ok(())
        }
        Err(e) => {
            log::error!("failed to launch update installer: {e}");
            let _ = std::fs::remove_file(&installer_path);
            Err(format!(
        "Couldn't start the installer ({e}). Try downloading it manually from the GitHub releases page."
      ))
        }
    }
}
