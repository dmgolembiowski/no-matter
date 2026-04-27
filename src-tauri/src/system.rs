//! OS integrations: dock/taskbar badge + native notifications.
//!
//! Both commands return `Result<(), String>` so the frontend's
//! `invoke::<()>(...)` path can propagate any platform error as a
//! string. Failures are non-fatal — the calling Effect just logs and
//! moves on. (You can still chat fine without a dock badge.)

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// Set the OS dock/taskbar badge to `count`. `0` clears the badge.
///
/// Tauri 2 exposes the badge API per-window. We grab the main webview
/// window and forward; the per-platform behavior is:
///   - macOS: full integer support on the Dock icon.
///   - Linux: Unity-launcher protocol (works in GNOME/KDE that honor
///     it; silently no-op elsewhere).
///   - Windows: shows a small overlay icon on the taskbar entry.
///
/// `None` clears the badge.
#[tauri::command]
pub fn set_dock_badge(app: AppHandle, count: u32) -> Result<(), String> {
    let value = if count == 0 { None } else { Some(count as i64) };
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    win.set_badge_count(value).map_err(|e| e.to_string())
}

/// Show a native notification banner. The plugin handles the OS
/// permission prompt on first call (macOS at least; on Linux it tends
/// to be granted by default).
#[tauri::command]
pub fn notify(app: AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}
