//! Native-side WebSocket manager.
//!
//! Lives in the Tauri process, owns the connection lifecycle (auth,
//! reconnect, ping/pong), and forwards typed events to the WebView via
//! `AppHandle::emit`. The frontend never speaks WebSocket directly.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

/// Server -> client events. Mirrored byte-for-byte on the Leptos side
/// (see `src/realtime.rs`) so a single `serde_json::from_str` round-trips.
///
/// Variants and field names must stay aligned with the frontend
/// `ServerEvent` (snake_case, internally tagged on `type`). Adding or
/// renaming requires touching both files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    MessagePosted {
        channel_id: String,
        message_id: String,
        author_id: String,
        preview: String,
        mentions_me: bool,
        /// Full message body, included so the client doesn't need a
        /// follow-up fetch for the common case. May be omitted by the
        /// gateway for very long bodies.
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
    /// A channel/group/DM became visible to the current user. Carries
    /// the full channel record so the client can render it without a
    /// follow-up fetch.
    ChannelCreated {
        channel: ChannelSummary,
    },
    MemberAdded {
        channel_id: String,
        user_id: String,
    },
    Reconnected,
    Disconnected,
}

/// Mirrors the frontend's `Channel` struct. Kept here as a copy rather
/// than imported from a shared crate to keep the Tauri side standalone;
/// the wire format is the contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Public,
    Private,
    Dm,
    Group,
}

/// Outbound channel handle, kept in Tauri-managed state so other commands
/// (e.g. "send typing indicator") can push frames without owning the socket.
#[derive(Default)]
pub struct WsState {
    pub outbound: Mutex<Option<mpsc::UnboundedSender<WsMessage>>>,
}

/// Spawns the connection task. Idempotent at the call site; calling twice
/// just creates two connections, so guard from the frontend.
pub fn spawn(app: AppHandle, url: String, token: String) {
    tauri::async_runtime::spawn(async move {
        let mut backoff = Duration::from_millis(500);

        // Server authenticates from the ?token= query at upgrade time
        // (see server/src/ws/gateway.rs). Tokens are URL-safe base64
        // (auth.rs::issue_token), so no percent-encoding required.
        let connect_url = if url.contains('?') {
            format!("{url}&token={token}")
        } else {
            format!("{url}?token={token}")
        };

        loop {
            match connect_async(&connect_url).await {
                Ok((stream, _)) => {
                    backoff = Duration::from_millis(500);
                    let _ = app.emit("ws://event", ServerEvent::Reconnected);

                    let (mut write, mut read) = stream.split();

                    // Wire up the outbound channel so other commands can send.
                    let (tx, mut rx) = mpsc::unbounded_channel::<WsMessage>();
                    if let Some(state) = app.try_state::<WsState>() {
                        *state.outbound.lock().await = Some(tx);
                    }

                    let writer = tokio::spawn(async move {
                        while let Some(msg) = rx.recv().await {
                            if write.send(msg).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Reader: parse and forward. Unknown frames are dropped
                    // intentionally so an older client tolerates a newer server.
                    while let Some(Ok(msg)) = read.next().await {
                        if let WsMessage::Text(txt) = msg {
                            match serde_json::from_str::<ServerEvent>(&txt) {
                                Ok(evt) => {
                                    let _ = app.emit("ws://event", evt);
                                }
                                Err(e) => {
                                    eprintln!("ws: unparseable frame: {e}");
                                }
                            }
                        }
                    }

                    writer.abort();
                    if let Some(state) = app.try_state::<WsState>() {
                        *state.outbound.lock().await = None;
                    }
                    let _ = app.emit("ws://event", ServerEvent::Disconnected);
                }
                Err(e) => eprintln!("ws: connect failed: {e}"),
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    });
}

#[tauri::command]
pub fn connect_realtime(app: AppHandle, url: String, token: String) {
    spawn(app, url, token);
}
