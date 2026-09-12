use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use tauri::{Emitter, Manager};

const TUNE_EVENT: &str = "notification-tune";
const TUNE_ACTION: &str = "tune";
const ARTWORK_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

fn artwork_extension(content_type: Option<&str>, path: &str) -> Option<&'static str> {
    let mime = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    match mime {
        Some("image/jpeg" | "image/jpg") => return Some("jpg"),
        Some("image/png") => return Some("png"),
        Some("image/gif") => return Some("gif"),
        Some(value) if value.starts_with("image/") => return None,
        _ => {}
    }

    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "gif" => Some("gif"),
        _ => None,
    }
}

fn cached_path(cache_dir: &Path, url: &str, extension: &str) -> PathBuf {
    cache_dir.join(format!("{:x}.{extension}", md5::compute(url.as_bytes())))
}

async fn prune_artwork_cache(cache_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(cache_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        let is_stale = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > ARTWORK_MAX_AGE);
        if is_stale {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

async fn cache_artwork(app: &tauri::AppHandle, artwork_url: Option<&str>) -> Option<PathBuf> {
    let artwork_url = artwork_url?.trim();
    if artwork_url.is_empty() {
        return None;
    }

    let url = match reqwest::Url::parse(artwork_url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url,
        _ => {
            log::warn!("ignoring invalid notification artwork URL");
            return None;
        }
    };
    let cache_dir = match app.path().app_cache_dir() {
        Ok(path) => path.join("notification-artwork"),
        Err(error) => {
            log::warn!("could not resolve notification artwork cache: {error}");
            return None;
        }
    };
    if let Err(error) = tokio::fs::create_dir_all(&cache_dir).await {
        log::warn!("could not create notification artwork cache: {error}");
        return None;
    }

    for extension in ["jpg", "png", "gif"] {
        let path = cached_path(&cache_dir, artwork_url, extension);
        if path.is_file() {
            return Some(path);
        }
    }

    // Shared NetworkService: its artwork profile already bounds both the deadline and the
    // body size, so the manual chunk loop that used to live here is gone.
    let cancel = tokio_util::sync::CancellationToken::new();
    let response = match crate::network::NetworkService::shared()
        .fetch_artwork(url.as_str(), &cancel)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            log::warn!(
                "could not download notification artwork: {}",
                crate::network::redact_text(&error.to_string())
            );
            return None;
        }
    };
    let extension =
        match artwork_extension(response.content_type.as_deref(), response.final_url.path()) {
            Some(extension) => extension,
            None => {
                log::warn!("notification artwork had an unsupported image type");
                return None;
            }
        };
    let bytes = response.bytes;
    if bytes.is_empty() {
        return None;
    }

    let destination = cached_path(&cache_dir, artwork_url, extension);
    let temporary = cache_dir.join(format!(
        "{:x}-{}.tmp",
        md5::compute(artwork_url.as_bytes()),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    if let Err(error) = tokio::fs::write(&temporary, bytes).await {
        log::warn!("could not cache notification artwork: {error}");
        return None;
    }
    if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        if !destination.is_file() {
            log::warn!("could not finalize notification artwork cache: {error}");
            return None;
        }
    }
    tokio::spawn({
        let cache_dir = cache_dir.clone();
        async move { prune_artwork_cache(&cache_dir).await }
    });
    Some(destination)
}

fn is_tune_action(action: Option<&str>) -> bool {
    action == Some(TUNE_ACTION)
}

fn emit_tune(app: &tauri::AppHandle, stream_id: u32) {
    if let Err(error) = app.emit(TUNE_EVENT, stream_id) {
        log::warn!("could not deliver notification Tune action: {error}");
    }
}

/// The Tauri notification plugin reports desktop permission as granted without
/// asking UNUserNotificationCenter on macOS. Use the native authorization API
/// there so enabling OS alerts prompts at the time the setting is changed.
#[tauri::command]
pub async fn ensure_os_notification_permission() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        return notify_rust::request_auth()
            .await
            .map_err(|error| error.to_string());
    }

    #[cfg(not(target_os = "macos"))]
    Ok(true)
}

/// Sends a rich alert notification through the platform-native desktop backend.
/// Artwork is cached locally because Windows desktop toasts and macOS notification
/// attachments cannot reliably consume the remote Stellar artwork URL directly.
#[tauri::command]
pub async fn send_os_notification(
    app: tauri::AppHandle,
    title: String,
    body: String,
    stream_id: u32,
    artwork_url: Option<String>,
) -> Result<(), String> {
    let artwork = cache_artwork(&app, artwork_url.as_deref()).await;

    #[cfg(target_os = "windows")]
    {
        use tauri_winrt_notification::{IconCrop, Toast};

        let app_id = if tauri::is_dev() {
            Toast::POWERSHELL_APP_ID
        } else {
            &app.config().identifier
        };
        let mut toast = Toast::new(app_id)
            .title(&title)
            .text1(&body)
            .add_button("Tune", TUNE_ACTION);
        if let Some(path) = artwork.as_deref() {
            toast = toast.icon(path, IconCrop::Square, "Album artwork");
        }
        let action_app = app.clone();
        toast
            .on_activated(move |action| {
                if is_tune_action(action.as_deref()) {
                    emit_tune(&action_app, stream_id);
                }
                Ok(())
            })
            .show()
            .map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        let identifier = app.config().identifier.clone();
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&title)
            .body(&body)
            .appname("Apogee")
            .auto_icon()
            .action(TUNE_ACTION, "Tune")
            .hint(notify_rust::Hint::DesktopEntry(identifier));
        if let Some(path) = artwork.as_deref().and_then(Path::to_str) {
            notification.image_path(path);
        }
        let handle = notification.show().map_err(|error| error.to_string())?;
        std::thread::spawn(move || {
            handle.wait_for_action(move |action| {
                if is_tune_action(Some(action)) {
                    emit_tune(&app, stream_id);
                }
            });
        });
    }

    #[cfg(target_os = "macos")]
    {
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&title)
            .body(&body)
            .action(TUNE_ACTION, "Tune");
        if let Some(path) = artwork.as_deref().and_then(Path::to_str) {
            notification.image_path(path);
        }
        let handle = notification.show().map_err(|error| error.to_string())?;
        std::thread::spawn(move || {
            handle.wait_for_action(move |action| {
                if is_tune_action(Some(action)) {
                    emit_tune(&app, stream_id);
                }
            });
        });
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    let _ = (app, title, body, stream_id, artwork);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_artwork_types() {
        assert_eq!(
            artwork_extension(Some("image/jpeg; charset=binary"), "cover"),
            Some("jpg")
        );
        assert_eq!(
            artwork_extension(Some("image/png"), "cover.bin"),
            Some("png")
        );
        assert_eq!(
            artwork_extension(Some("application/octet-stream"), "/covers/album.png"),
            Some("png")
        );
        assert_eq!(artwork_extension(None, "/covers/album.JPEG"), Some("jpg"));
        assert_eq!(artwork_extension(Some("image/webp"), "cover.webp"), None);
    }

    #[test]
    fn artwork_downloads_stay_capped_at_the_historical_limit() {
        // The incremental cap used to live in this file. It now belongs to the shared
        // NetworkService's artwork profile, so this guards against that move silently
        // loosening the ceiling.
        let config = crate::network::NetworkServiceConfig::default();
        assert_eq!(
            config.artwork_max_body_bytes,
            3 * 1024 * 1024,
            "notification artwork must stay capped at 3 MiB"
        );
    }

    #[test]
    fn only_explicit_tune_action_tunes() {
        assert!(is_tune_action(Some("tune")));
        assert!(!is_tune_action(Some("default")));
        assert!(!is_tune_action(None));
    }
}
