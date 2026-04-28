// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod system;
mod upload;
mod ws;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(ws::WsState::default())
        .invoke_handler(tauri::generate_handler![
            ws::connect_realtime,
            upload::upload_file,
            system::set_dock_badge,
            system::notify,
            audio::play_chime,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
