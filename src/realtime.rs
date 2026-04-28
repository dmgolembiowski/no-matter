//! Bridge from server WebSocket events into Leptos reactive state.
//!
//! The Leptos WASM frontend opens the WebSocket directly via
//! `gloo-net` so the realtime path works identically in a browser tab
//! and inside Tauri's WebView. (The previous design routed frames
//! through a Tauri command + `app.emit("ws://event", …)`, which left
//! pure-browser sessions with no event source at all and was the reason
//! cross-session messages weren't propagating.)
//!
//! Upload progress events still come from Tauri (`upload://*`) since
//! the upload itself is owned by the Tauri side.

use futures_util::StreamExt;
use gloo_net::websocket::{futures::WebSocket, Message as WsMessage};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use crate::server::messages::Message;
use crate::stores::{
    auth::AuthStore,
    channels::{Channel, ChannelStore},
    messages::MessageStore,
    route::{RouteStore, Selected},
    typing::TypingStore,
    unread::UnreadStore,
    upload::UploadStore,
};
use crate::tauri_bridge;

/// Mirror of `src-tauri/src/ws.rs::ServerEvent`. Keep these in sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    MessagePosted {
        channel_id: String,
        message_id: String,
        author_id: String,
        preview: String,
        mentions_me: bool,
        /// Full message body, included so we don't need a follow-up fetch
        /// for the common case. For very long bodies the server may omit
        /// this and the client falls back to fetching by id.
        body: Option<String>,
        created_at: i64,
        file_ids: Vec<String>,
    },
    MessageEdited {
        channel_id: String,
        message_id: String,
        body: String,
        edited_at: i64,
    },
    MessageDeleted {
        channel_id: String,
        message_id: String,
    },
    ChannelRead {
        channel_id: String,
        user_id: String,
        last_read_at: i64,
    },
    Typing {
        channel_id: String,
        user_id: String,
    },
    FileShared {
        channel_id: String,
        file_id: String,
        message_id: String,
    },
    /// A channel/group/DM became visible to the current user — either it
    /// was just created and they're a member, or someone added them. The
    /// client upserts it into `ChannelStore` and the sidebar lights up.
    ChannelCreated {
        channel: Channel,
    },
    /// Membership change to a channel the user is already in. We re-add
    /// the user to the local member list rather than re-fetching the
    /// whole channel.
    MemberAdded {
        channel_id: String,
        user_id: String,
    },
    Reconnected,
    Disconnected,
}

#[derive(Debug, Clone, Deserialize)]
struct UploadProgress {
    upload_id: String,
    bytes_sent: u64,
    total: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct UploadComplete {
    upload_id: String,
    file_id: String,
}

#[derive(Copy, Clone)]
pub struct Stores {
    pub unread: UnreadStore,
    pub messages: MessageStore,
    pub typing: TypingStore,
    pub uploads: UploadStore,
    pub channels: ChannelStore,
    pub route: RouteStore,
    pub auth: AuthStore,
}

/// Mounts auxiliary listeners (currently just upload progress, which
/// still flows from Tauri). The WebSocket itself isn't opened here —
/// `connect()` is called once auth is in hand.
pub fn install_realtime_bridge(stores: Stores) {
    install_upload_listeners(stores);
}

fn install_upload_listeners(stores: Stores) {
    spawn_local(async move {
        let Ok(mut events) = tauri_sys::event::listen::<UploadProgress>("upload://progress").await
        else {
            return;
        };
        while let Some(evt) = events.next().await {
            stores.uploads.progress(
                &evt.payload.upload_id,
                evt.payload.bytes_sent,
                evt.payload.total,
            );
        }
    });

    spawn_local(async move {
        let Ok(mut events) = tauri_sys::event::listen::<UploadComplete>("upload://complete").await
        else {
            return;
        };
        while let Some(evt) = events.next().await {
            stores
                .uploads
                .complete(&evt.payload.upload_id, evt.payload.file_id.clone());
        }
    });
}

fn handle_server_event(stores: Stores, evt: ServerEvent) {
    match evt {
        ServerEvent::MessagePosted {
            channel_id,
            message_id,
            author_id,
            preview,
            mentions_me,
            body,
            created_at,
            file_ids,
        } => {
            stores
                .unread
                .bump(channel_id.clone(), message_id.clone(), mentions_me);

            // OS notification: skip if the user is already viewing this
            // channel (they don't need a banner for what's on screen) or
            // if they're the author (don't notify on your own send).
            let is_self = stores
                .auth
                .session()
                .with_untracked(|s| s.as_ref().map(|s| s.user_id == author_id).unwrap_or(false));
            let is_active_channel = stores
                .route
                .selected
                .with_untracked(|sel| matches!(sel,
                    Selected::Channel(id) | Selected::Dm(id) | Selected::Group(id)
                        if id == &channel_id
                ));
            if !is_self && !is_active_channel {
                let title = notification_title(stores, &channel_id, &author_id);
                let body_for_notif = preview.clone();
                spawn_local(async move {
                    if let Err(e) = tauri_bridge::notify(title, body_for_notif).await {
                        leptos::logging::warn!("notify failed: {e}");
                    }
                });

                // Audible chime, but only for personal-feel channels
                // (DMs, group DMs). Public/private channel chatter
                // would be too noisy if every post pinged.
                let is_personal = stores.channels.get(channel_id.clone()).with_untracked(|c| {
                    matches!(
                        c.as_ref().map(|c| c.kind),
                        Some(crate::stores::channels::ChannelKind::Dm)
                            | Some(crate::stores::channels::ChannelKind::Group)
                    )
                });
                leptos::logging::log!(
                    "chime: channel {} personal={}",
                    channel_id,
                    is_personal
                );
                if is_personal {
                    spawn_local(async move {
                        if let Err(e) = tauri_bridge::play_chime().await {
                            leptos::logging::warn!("chime invoke failed: {e}");
                        }
                    });
                }
            }

            // Append to the message cache if the channel is loaded.
            // If the body wasn't included in the event, the cache append
            // is skipped — the next time the user opens the channel,
            // pagination will fetch it. (A fuller implementation could
            // fetch the single message by id here.)
            if let Some(body) = body {
                stores.messages.append(
                    &channel_id,
                    Message {
                        id: message_id,
                        channel_id: channel_id.clone(),
                        author_id,
                        body,
                        created_at,
                        file_ids,
                    },
                );
            }
        }
        ServerEvent::MessageEdited {
            channel_id,
            message_id,
            body,
            ..
        } => {
            stores.messages.edit(&channel_id, &message_id, body);
        }
        ServerEvent::MessageDeleted {
            channel_id,
            message_id,
        } => {
            stores.messages.delete(&channel_id, &message_id);
        }
        ServerEvent::ChannelRead { channel_id, .. } => {
            stores.unread.mark_read(&channel_id);
        }
        ServerEvent::Typing {
            channel_id,
            user_id,
        } => {
            stores.typing.record(channel_id, user_id);
        }
        ServerEvent::FileShared { .. } => {
            // The MessagePosted event for the same message carries the
            // file_ids, so this is currently informational only. Hook
            // here if you want a "file added to channel" sidebar.
        }
        ServerEvent::ChannelCreated { channel } => {
            stores.channels.upsert(channel);
            // Pull a fresh user directory: a brand-new DM/group can
            // include people who weren't in our `users` map yet
            // (initial_state only seeds shared-channel members), and
            // the sidebar's DM label needs their username to render
            // correctly from the recipient's perspective.
            let channels_store = stores.channels;
            spawn_local(async move {
                if let Ok(users) = crate::server::channels::list_users().await {
                    channels_store.merge_users(users);
                }
            });
        }
        ServerEvent::MemberAdded {
            channel_id,
            user_id,
        } => {
            stores.channels.add_member(&channel_id, user_id);
        }
        ServerEvent::Reconnected => {
            // After reconnect the unread snapshot may be stale. The
            // cleanest fix is to re-fetch /api/initial_state and replace
            // the store contents. Stub for now.
        }
        ServerEvent::Disconnected => {
            // Surface a "reconnecting…" toast somewhere in the UI.
        }
    }
}

/// Build a "{author} in {channel}" string for OS notifications. Falls
/// back to ids when the lookup tables haven't been hydrated yet, which
/// can happen briefly between login and `list_initial_state` resolving.
fn notification_title(stores: Stores, channel_id: &str, author_id: &str) -> String {
    let author = stores
        .channels
        .user(author_id.to_string())
        .with_untracked(|u| u.as_ref().map(|u| u.username.clone()).unwrap_or_else(|| author_id.to_string()));
    let channel = stores
        .channels
        .get(channel_id.to_string())
        .with_untracked(|c| {
            c.as_ref().map(|c| match c.kind {
                crate::stores::channels::ChannelKind::Public => format!("#{}", c.name),
                crate::stores::channels::ChannelKind::Private => format!("🔒 {}", c.name),
                crate::stores::channels::ChannelKind::Group => c.name.clone(),
                crate::stores::channels::ChannelKind::Dm => format!("@{}", c.name),
            })
            .unwrap_or_else(|| channel_id.to_string())
        });
    format!("{author} in {channel}")
}

/// Open the WebSocket and spawn the read loop. Reconnects with
/// exponential backoff (capped at 30s) so a transient network blip or
/// server restart doesn't require a re-login.
///
/// Returns immediately after spawning — the connection lifecycle runs
/// in the background. Errors that prevent opening at all are logged and
/// the loop retries; the function never returns Err in normal operation.
pub fn connect(stores: Stores, url: String, token: String) {
    // Server authenticates from `?token=` at the upgrade handshake
    // (see server/src/ws/gateway.rs). Tokens are URL-safe base64 so
    // no percent-encoding is needed.
    let connect_url = if url.contains('?') {
        format!("{url}&token={token}")
    } else {
        format!("{url}?token={token}")
    };

    spawn_local(async move {
        let mut backoff_ms: u32 = 500;
        loop {
            let ws = match WebSocket::open(&connect_url) {
                Ok(ws) => ws,
                Err(e) => {
                    leptos::logging::warn!("realtime: ws open failed: {e}");
                    gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
                    continue;
                }
            };

            backoff_ms = 500;
            let (_write, mut read) = ws.split();

            while let Some(frame) = read.next().await {
                match frame {
                    Ok(WsMessage::Text(txt)) => {
                        match serde_json::from_str::<ServerEvent>(&txt) {
                            Ok(evt) => handle_server_event(stores, evt),
                            Err(e) => {
                                leptos::logging::warn!("realtime: parse: {e} — frame: {txt}");
                            }
                        }
                    }
                    Ok(WsMessage::Bytes(_)) => {
                        // Server only sends text frames; ignore any binary noise.
                    }
                    Err(e) => {
                        leptos::logging::warn!("realtime: ws error: {e}");
                        break;
                    }
                }
            }

            // Stream ended — server closed or network dropped. Reconnect.
            leptos::logging::log!("realtime: ws closed, reconnecting…");
            handle_server_event(stores, ServerEvent::Disconnected);
            gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
        }
    });
}
