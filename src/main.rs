mod app;
pub(crate) mod components;
pub(crate) mod config;
pub(crate) mod realtime;
pub(crate) mod server;
pub(crate) mod stores;
pub(crate) mod tauri_bridge;

use app::*;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| {
        view! {
            <App/>
        }
    })
}
