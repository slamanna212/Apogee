use serde::Serialize;
use tauri::{Manager, ResourceId, Runtime, Webview};
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
#[tauri::command]
pub async fn check_update_at_endpoint<R: Runtime>(
  webview: Webview<R>,
  url: String,
) -> Result<Option<Metadata>, String> {
  let endpoint = Url::parse(&url).map_err(|e| e.to_string())?;

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
      date
        .format(&time::format_description::well_known::Rfc3339)
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
