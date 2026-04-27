//! Message + file server fns. Each delegates to the HTTP server via
//! the ambient `client::Credentials` set by `AuthStore`.
//!
//! Wire shapes match the server's exactly (see `server/messages.rs` and
//! `server/files.rs` over there). When that contract changes, both
//! sides have to move together — they don't share a crate.

use serde::{Deserialize, Serialize};

use crate::server::client;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at: i64,
    pub file_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    /// Pass back as `before` to fetch the next older page. `None` means
    /// the channel's start has been reached.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMeta {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub thumb_url: Option<String>,
    pub url: String,
}

/// Error type the rest of the app branches on. Display impl preserves
/// the server's structured `[code]` suffix so modals can match codes
/// like `name_taken` via substring (see `client::check_ok`).
#[derive(Debug, Clone)]
pub struct ServerError(pub String);

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Serialize)]
struct LoadMessagesWire<'a> {
    channel_id: &'a str,
    before: Option<&'a str>,
    limit: u32,
}

pub async fn load_messages(
    channel_id: String,
    before: Option<String>,
    limit: u32,
) -> Result<MessagePage, ServerError> {
    client::post(
        "/api/load_messages",
        &LoadMessagesWire {
            channel_id: &channel_id,
            before: before.as_deref(),
            limit,
        },
    )
    .await
}

#[derive(Serialize)]
struct GetFileMetaWire<'a> {
    file_id: &'a str,
}

pub async fn get_file_meta(file_id: String) -> Result<FileMeta, ServerError> {
    client::post("/api/get_file_meta", &GetFileMetaWire { file_id: &file_id }).await
}

#[derive(Serialize)]
struct MarkReadWire<'a> {
    channel_id: &'a str,
    last_message_id: &'a str,
}

pub async fn mark_channel_read(
    channel_id: String,
    last_message_id: String,
) -> Result<(), ServerError> {
    client::post_no_resp(
        "/api/mark_channel_read",
        &MarkReadWire {
            channel_id: &channel_id,
            last_message_id: &last_message_id,
        },
    )
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostMessageRequest {
    pub channel_id: String,
    pub body: String,
    pub file_ids: Vec<String>,
    /// Idempotency key. The server's `messages.client_msg_id` column
    /// has a unique index — retries from a flaky network won't
    /// double-post.
    pub client_msg_id: String,
}

pub async fn post_message(req: PostMessageRequest) -> Result<Message, ServerError> {
    client::post("/api/post_message", &req).await
}

#[derive(Serialize)]
struct EditMessageWire<'a> {
    channel_id: &'a str,
    message_id: &'a str,
    new_body: &'a str,
}

pub async fn edit_message(
    channel_id: String,
    message_id: String,
    new_body: String,
) -> Result<(), ServerError> {
    client::post_no_resp(
        "/api/edit_message",
        &EditMessageWire {
            channel_id: &channel_id,
            message_id: &message_id,
            new_body: &new_body,
        },
    )
    .await
}

#[derive(Serialize)]
struct DeleteMessageWire<'a> {
    channel_id: &'a str,
    message_id: &'a str,
}

pub async fn delete_message(
    channel_id: String,
    message_id: String,
) -> Result<(), ServerError> {
    client::post_no_resp(
        "/api/delete_message",
        &DeleteMessageWire {
            channel_id: &channel_id,
            message_id: &message_id,
        },
    )
    .await
}

#[derive(Deserialize)]
struct UploadResp {
    file_id: String,
}

/// Upload a file as multipart. Returns the new `file_id` which the
/// caller attaches to a draft message via `post_message::file_ids`.
///
/// Uses a `web_sys::FormData` so we don't have to manually compose the
/// multipart body; the browser/WebView handles boundaries for us.
pub async fn upload_file(
    channel_id: &str,
    file: &web_sys::File,
) -> Result<String, ServerError> {
    let form = web_sys::FormData::new()
        .map_err(|e| ServerError(format!("FormData::new: {e:?}")))?;
    form.append_with_str("channel_id", channel_id)
        .map_err(|e| ServerError(format!("append channel_id: {e:?}")))?;
    form.append_with_blob_and_filename("file", file.as_ref(), &file.name())
        .map_err(|e| ServerError(format!("append file: {e:?}")))?;

    let resp: UploadResp = client::post_multipart("/files", &form).await?;
    Ok(resp.file_id)
}
