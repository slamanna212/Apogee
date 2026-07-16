use tauri::Manager;

/// `tauri_plugin_log`'s dispatch filter level is fixed at plugin build time
/// (see lib.rs, which builds it permissively at Debug) - the actual runtime
/// verbosity is controlled by this global gate instead, which the `log`
/// crate checks before a `log::debug!` call even constructs a Record. This
/// lets a Settings toggle turn on the mpv IPC/command tracing added in
/// mpv.rs without needing to rebuild/reattach the logger.
#[tauri::command]
pub fn set_log_level(verbose: bool) {
    log::set_max_level(if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
}

/// Bundles the active log file plus any rotated backups (see the
/// `KeepSome(5)` rotation strategy in lib.rs - a fresh dated file like
/// `apogee_<date>.log` is created each time the active file rotates) into
/// one export, so downloading the log doesn't miss the window an earlier
/// rotation pushed out of the active file.
#[tauri::command]
pub fn export_log_file(app: tauri::AppHandle, destination: String) -> Result<(), String> {
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let name = app.package_info().name.clone();

    let mut log_files: Vec<_> = std::fs::read_dir(&log_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.starts_with(&name) && f.ends_with(".log"))
        })
        .collect();

    if log_files.is_empty() {
        return Err("No log file found yet".into());
    }

    // Rotated file names are date-suffixed, not sequence-numbered, so sort by
    // modification time (oldest first) rather than relying on name ordering.
    log_files.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());

    let mut combined = String::new();
    for path in &log_files {
        let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("?");
        let contents = std::fs::read_to_string(path)
            .unwrap_or_else(|e| format!("<failed to read {file_name}: {e}>\n"));
        combined.push_str(&format!("=== {file_name} ===\n"));
        combined.push_str(&contents);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }

    std::fs::write(&destination, combined).map_err(|e| e.to_string())?;
    Ok(())
}
