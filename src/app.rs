//! Top-level app component.
//!
//! Boot order:
//!   1. Construct all stores and provide them via context.
//!   2. Install the realtime bridge (subscribes to Tauri events) once.
//!   3. Gate render on `AuthStore`:
//!         - `None`  → `LoginPage`.
//!         - `Some` → `Shell`. On the first transition we hydrate the
//!                    channel catalog and ask Tauri to open the
//!                    WebSocket using the session's URL + token.
//!
//! Sign-out clears the session, the gate flips back to LoginPage, and
//! the existing WebSocket stays open until the next reconnect cycle.
//! A fuller impl would invoke a Tauri "disconnect_realtime" command on
//! sign-out — flagged as remaining work.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::components::login::LoginPage;
use crate::components::shell::Shell;
use crate::realtime::{connect, install_realtime_bridge, Stores};
use crate::server::channels::list_initial_state;
use crate::stores::{
    auth::AuthStore,
    channels::ChannelStore,
    messages::MessageStore,
    route::RouteStore,
    typing::TypingStore,
    unread::UnreadStore,
    upload::UploadStore,
};

#[component]
pub fn App() -> impl IntoView {
    // Construct + provide every store. They're cheap signal handles.
    let auth = AuthStore::new();
    let unread = UnreadStore::new();
    let messages = MessageStore::new();
    let typing = TypingStore::new();
    let uploads = UploadStore::new();
    let channels = ChannelStore::new();
    let route = RouteStore::new();

    provide_context(auth);
    provide_context(unread);
    provide_context(messages);
    provide_context(typing);
    provide_context(uploads);
    provide_context(channels);
    provide_context(route);

    let stores = Stores {
        unread,
        messages,
        typing,
        uploads,
        channels,
        route,
        auth,
    };

    // Mount the realtime bridge once, regardless of auth state. It's a
    // no-op until events start flowing.
    Effect::new(move |prev: Option<()>| {
        if prev.is_some() {
            return;
        }
        install_realtime_bridge(stores);
    });

    // Push the total-mention count out to the OS dock/taskbar badge
    // whenever it changes. The Memo recomputes only when an unread
    // entry actually shifts, so this Effect is quiet during typing /
    // typical idle.
    let total_mentions = unread.total_mentions();
    Effect::new(move |prev: Option<u32>| {
        let now = total_mentions.get();
        if Some(now) == prev {
            return now;
        }
        spawn_local(async move {
            if let Err(e) = crate::tauri_bridge::set_dock_badge(now).await {
                leptos::logging::warn!("set_dock_badge failed: {e}");
            }
        });
        now
    });

    // Re-fire whenever auth flips from None → Some: hydrate channel
    // catalog, open the WebSocket. The `prev` guard makes sure we only
    // run on transition (not on every signal read).
    let authed = auth.is_authed();
    Effect::new(move |prev: Option<bool>| {
        let now = authed.get();
        let was = prev.unwrap_or(false);
        if !was && now {
            // Just signed in.
            spawn_local(async move {
                match list_initial_state().await {
                    Ok(state) => channels.hydrate(state.channels, state.users),
                    Err(e) => leptos::logging::error!("initial_state: {e}"),
                }
            });

            if let Some(session) = auth.snapshot() {
                let url = ws_url_for(&session.server_url);
                let token = session.token.clone();
                spawn_local(async move {
                    if let Err(e) = connect(url, token).await {
                        leptos::logging::error!("realtime connect: {e}");
                    }
                });
            }
        }
        now
    });

    view! {
        {move || if auth.is_authed().get() {
            view! { <Shell/> }.into_any()
        } else {
            view! { <LoginPage/> }.into_any()
        }}
    }
}

/// Derive the WebSocket URL from the HTTP server URL the user typed.
/// `http://` → `ws://`, `https://` → `wss://`, append `/ws`.
fn ws_url_for(server_url: &str) -> String {
    let trimmed = server_url.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}/ws")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}/ws")
    } else {
        // Fallback: the LoginPage's validation should have caught this,
        // but be defensive.
        format!("{trimmed}/ws")
    }
}
