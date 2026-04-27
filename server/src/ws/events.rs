//! Server → client event shapes. Mirror of the frontend's
//! `crate::realtime::ServerEvent`.
//!
//! Adding or renaming variants here means touching three files:
//! - `app/src/realtime.rs` (frontend)
//! - `app/src-tauri/src/ws.rs` (Tauri-side mirror; the desktop process
//!   forwards frames opaquely via this enum so it stays decodable)
//! - this file
//!
//! The wire format (`{"type":"…", …}` snake_case) is the contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    MessagePosted {
        channel_id: String,
        message_id: String,
        author_id: String,
        preview: String,
        mentions_me: bool,
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
    ChannelCreated {
        channel: ChannelSummary,
    },
    MemberAdded {
        channel_id: String,
        user_id: String,
    },
    /// Synthesized client-side after a successful reconnect; the server
    /// never sends this. Kept here so the enum round-trips.
    Reconnected,
    /// Same — synthesized when the WebSocket drops.
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub members: Vec<String>,
}
