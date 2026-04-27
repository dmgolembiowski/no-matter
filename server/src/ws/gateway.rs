//! WebSocket upgrade handler + per-connection task.
//!
//! Auth flow:
//!   1. Client connects to `/ws?token=…`.
//!   2. Server resolves the token to a user (same lookup as bearer
//!      auth, see `auth::lookup_token`).
//!   3. Server upgrades the connection.
//!   4. Per-connection task subscribes to the broadcast bus and
//!      filters by recipient set.
//!
//! Inbound text frames are accepted but currently ignored; a richer
//! impl would parse client-side actions (`typing`, `subscribe`) here.

use std::collections::HashMap;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

use crate::auth::lookup_token;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct WsParams {
    token: String,
}

pub async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
) -> impl IntoResponse {
    // Authenticate before upgrading. A bad token gets a 401, not a
    // dangling socket the client has to time out on.
    let user = match lookup_token(&state.db, &params.token).await {
        Ok(u) => u,
        Err(_) => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
    };

    let user_id = user.id.clone();
    ws.on_upgrade(move |socket| handle(socket, state, user_id))
}

async fn handle(socket: WebSocket, state: AppState, user_id: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut bus_rx = state.bus.subscribe();

    // Reader: drain inbound frames so the client can heartbeat / send
    // typing actions later. For now we only care about close detection.
    let reader = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => continue,
            }
        }
    });

    // Writer: broadcast → JSON → frame, filtered by recipients.
    let user_id_for_writer = user_id.clone();
    let writer = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(routed) => {
                    if !routed.recipients.contains(&user_id_for_writer) {
                        continue;
                    }
                    let json = match serde_json::to_string(&routed.event) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!("ws: serialize event: {e}");
                            continue;
                        }
                    };
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                // Lagged: a slow client that fell behind. Disconnect;
                // the client reconnects and re-hydrates fresh state.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => break,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Whichever task ends first, kill the other.
    tokio::select! {
        _ = reader => {},
        _ = writer => {},
    }

    let _ = HashMap::<String, String>::new();
    tracing::debug!("ws: closed for user {user_id}");
}
