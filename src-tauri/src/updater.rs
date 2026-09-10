use serde::Serialize;
use serde_json::Value;
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
/// that installs by calling `ShellExecuteW` while ignoring its return value
/// and then unconditionally calls `std::process::exit(0)`, so a launch
/// failure makes the app silently vanish with no error and no update. See
/// `install_windows` below, which reports that failure instead.
///
/// This path originally also existed to escape Apogee's Job Object, which was
/// created to kill MPV on exit. That job object is gone with MPV, so the
/// breakaway flag has been removed; the launch-failure reporting is why the
/// custom path remains.
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

/// Writes the downloaded installer to a temp file and launches it directly,
/// instead of via the plugin's `ShellExecuteW` call.
///
/// No longer passes `CREATE_BREAKAWAY_FROM_JOB`: that existed solely to escape
/// the Job Object Apogee used to create so MPV died with the app. With MPV and
/// the job object both removed, the installer has no job to break away from and
/// the flag would be a no-op.
///
/// `/P /R` reproduces the plugin's documented default `Passive` install mode
/// (Apogee doesn't override `plugins.updater.windows.installMode`); `/UPDATE`
/// is what the plugin adds unconditionally too. Apogee exits only *after*
/// confirming the installer process actually started - unlike the plugin's own
/// path, a launch failure here is returned as a normal error instead of the app
/// silently vanishing.
#[cfg(windows)]
fn install_windows<R: Runtime>(
    app_handle: &tauri::AppHandle<R>,
    version: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let temp_dir = std::env::temp_dir().join(format!("apogee-updater-{version}"));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("couldn't create temp dir for the update installer: {e}"))?;
    let installer_path = temp_dir.join(format!("Apogee_{version}_x64-setup.exe"));
    std::fs::write(&installer_path, bytes)
        .map_err(|e| format!("couldn't write the update installer to disk: {e}"))?;

    log::info!("launching update installer at {installer_path:?}");

    match std::process::Command::new(&installer_path)
        .args(["/P", "/R", "/UPDATE"])
        .spawn()
    {
        Ok(child) => {
            log::info!("update installer launched (pid {})", child.id());
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

/// Fetches the repository's release list.
///
/// Moved out of the frontend so it goes through the shared `NetworkService` like every
/// other request. GitHub has no "latest release including prereleases" alias, so the
/// channel is resolved by walking the list; that filtering stays in the frontend, which
/// already has the version-comparison logic and its tests.
#[tauri::command]
pub async fn github_releases(
    network: tauri::State<'_, crate::network::NetworkService>,
    repo: String,
) -> Result<Value, String> {
    // The repo comes from a frontend constant, but validate anyway: it is interpolated
    // into a path, and "owner/name" is the only shape that can be correct.
    let mut parts = repo.split('/');
    let valid = matches!((parts.next(), parts.next(), parts.next()), (Some(o), Some(n), None)
        if !o.is_empty()
            && !n.is_empty()
            && repo
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')));
    if !valid {
        return Err("invalid repository identifier".to_string());
    }

    let url = format!("https://api.github.com/repos/{repo}/releases");
    let cancel = tokio_util::sync::CancellationToken::new();
    let body = network
        .fetch_json_with_headers(&url, &[("Accept", "application/vnd.github+json")], &cancel)
        .await
        .map_err(|error| match error {
            crate::network::NetworkError::Status { status, .. } => {
                format!("GitHub API request failed: {}", status.as_u16())
            }
            other => format!(
                "GitHub API request failed: {}",
                crate::network::redact_text(&other.to_string())
            ),
        })?;
    serde_json::from_slice(&body.bytes).map_err(|_| "GitHub API returned invalid JSON".to_string())
}

#[cfg(test)]
mod repo_tests {
    /// Mirrors the validation in `github_releases`; kept in step by construction.
    fn valid(repo: &str) -> bool {
        let mut parts = repo.split('/');
        matches!((parts.next(), parts.next(), parts.next()), (Some(o), Some(n), None)
            if !o.is_empty()
                && !n.is_empty()
                && repo
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
    }

    #[test]
    fn accepts_an_ordinary_owner_and_name() {
        assert!(valid("slamanna212/Apogee"));
        assert!(valid("some-org/some_repo.js"));
    }

    #[test]
    fn rejects_anything_that_could_escape_the_path() {
        for bad in [
            "owner",            // no name
            "owner/name/extra", // too many segments
            "owner/",           // empty name
            "/name",            // empty owner
            "owner/name?x=1",   // query injection
            "owner/name#frag",  // fragment
            "owner/na me",      // space
            "../../etc",        // traversal
        ] {
            assert!(!valid(bad), "should have been rejected: {bad}");
        }
    }
}
