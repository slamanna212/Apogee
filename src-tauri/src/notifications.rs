/// `tauri-plugin-notification`'s Linux backend never sets the D-Bus `desktop-entry`
/// hint, which GNOME (and other shells) rely on to resolve which app is sending a
/// notification - without it, notifications can silently fail to display. This
/// command exists purely to plug that gap by talking to `notify-rust` directly with
/// the hint set. It's a no-op on other platforms, where the plugin's own path works.
#[tauri::command]
pub fn send_os_notification(
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let identifier = app.config().identifier.clone();
        if let Err(e) = notify_rust::Notification::new()
            .summary(&title)
            .body(&body)
            .appname("Apogee")
            .auto_icon()
            .hint(notify_rust::Hint::DesktopEntry(identifier))
            .show()
        {
            log::warn!("failed to show OS notification: {e}");
            return Err(e.to_string());
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, title, body);
    }
    Ok(())
}
