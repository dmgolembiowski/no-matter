//! Bridge from Tauri events into Leptos reactive state.
//!
//! Mounted once at app startup. Every `ws://event` and `upload://*`
//! payload is decoded and dispatched to the appropriate store. New
//! event types can be added by extending `ServerEvent` here and on the
//! Tauri side — the `serde(tag = "type")` keeps the wire format stable.
//!
//! All stores are passed in by value (they're `Copy`) rather than pulled
//! from context, so this function can be called before the component
//! tree is mounted if needed.

use futures_util::StreamExt;
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

/// Subscribes to Tauri events and routes payloads into stores.
/// Call exactly once, after stores are constructed.
pub fn install_realtime_bridge(stores: Stores) {
    install_ws_listener(stores);
    install_upload_listeners(stores);
}

fn install_ws_listener(stores: Stores) {
    spawn_local(async move {
        let mut events = match tauri_sys::event::listen::<ServerEvent>("ws://event").await {
            Ok(e) => e,
            Err(e) => {
                leptos::logging::error!("realtime: ws listen failed: {e:?}");
                return;
            }
        };

        while let Some(evt) = events.next().await {
            handle_server_event(stores, evt.payload);
        }
    });
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

/// Asks Tauri to open the WebSocket. Done once at boot, after auth.
pub async fn connect(url: String, token: String) -> Result<(), String> {
    use tauri_sys::tauri::invoke;

    #[derive(Serialize)]
    struct Args {
        url: String,
        token: String,
    }

    let _: () = invoke("connect_realtime", &Args { url, token })
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}
