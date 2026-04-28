//! Frontend wrappers around the Tauri custom commands defined in
//! `src-tauri/src/system.rs`. Each one is a thin `invoke(...)` call
//! that returns either `Ok(())` or a stringified error so the caller
//! can log without panicking.
//!
//! Failures are non-fatal — if the dock badge or notification API
//! aren't available on the current platform, the user still has a
//! perfectly functional chat client.

use serde::Serialize;
use tauri_sys::tauri::invoke;

#[derive(Serialize)]
struct BadgeArgs {
    count: u32,
}

pub async fn set_dock_badge(count: u32) -> Result<(), String> {
    let _: () = invoke("set_dock_badge", &BadgeArgs { count })
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

#[derive(Serialize)]
struct NotifyArgs {
    title: String,
    body: String,
}

pub async fn notify(title: String, body: String) -> Result<(), String> {
    let _: () = invoke("notify", &NotifyArgs { title, body })
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

#[derive(Serialize)]
struct NoArgs {}

/// Play the embedded notification chime. The MP3 is baked into the
/// Tauri binary; the command spawns a thread there and returns
/// immediately. Failure (no audio device, no Tauri host) is non-fatal.
pub async fn play_chime() -> Result<(), String> {
    let _: () = invoke("play_chime", &NoArgs {})
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

