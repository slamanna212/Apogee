use tauri::WebviewWindow;

#[cfg(not(any(windows, target_os = "macos")))]
use tauri::{PhysicalPosition, PhysicalSize};

#[cfg(windows)]
fn set_platform_window_bounds(
    window: &WebviewWindow,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    };

    let tauri_hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let hwnd = HWND(tauri_hwnd.0);
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width.try_into().map_err(|_| "window width is too large")?,
            height
                .try_into()
                .map_err(|_| "window height is too large")?,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER,
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "macos")]
fn set_platform_window_bounds(
    window: &WebviewWindow,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    use objc2_app_kit::NSWindow;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let current_position = window.outer_position().map_err(|error| error.to_string())?;
    let ns_window = window.ns_window().map_err(|error| error.to_string())? as *const NSWindow;
    let ns_window = unsafe { ns_window.as_ref() }.ok_or("native NSWindow handle was null")?;
    let current_frame = unsafe { ns_window.frame() };

    // Tauri exposes physical, top-left-origin desktop coordinates. AppKit uses
    // logical points with a bottom-left origin, so apply the requested movement
    // as a delta from the known current frame. This also stays correct across
    // macOS's multi-monitor coordinate space.
    let delta_x = (x - current_position.x) as f64 / scale;
    let delta_top = (y - current_position.y) as f64 / scale;
    let new_width = width as f64 / scale;
    let new_height = height as f64 / scale;
    let current_top = current_frame.origin.y + current_frame.size.height;
    let frame = CGRect::new(
        CGPoint::new(
            current_frame.origin.x + delta_x,
            current_top - delta_top - new_height,
        ),
        CGSize::new(new_width, new_height),
    );

    unsafe { ns_window.setFrame_display_animate(frame, true, false) };
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set_platform_window_bounds(
    window: &WebviewWindow,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    // Wayland intentionally provides no global top-level positioning API. Keep
    // the existing behavior isolated here until the Wayland-specific transition
    // can be implemented without pretending it has atomic frame semantics.
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())?;
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| error.to_string())
}

/// Changes the complete native window frame atomically on platforms that
/// expose that primitive, preventing a compositor paint between move/resize.
#[tauri::command]
pub fn set_window_bounds(
    window: WebviewWindow,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    set_platform_window_bounds(&window, x, y, width, height)
}
