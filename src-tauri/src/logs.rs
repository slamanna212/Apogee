use tauri::Manager;

#[tauri::command]
pub fn export_log_file(app: tauri::AppHandle, destination: String) -> Result<(), String> {
  let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
  let source = log_dir.join(format!("{}.log", app.package_info().name));
  if !source.exists() {
    return Err("No log file found yet".into());
  }
  std::fs::copy(&source, &destination).map_err(|e| e.to_string())?;
  Ok(())
}
