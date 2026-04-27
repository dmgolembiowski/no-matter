//! Message handlers: load (cursor pagination), post (idempotent), edit,
//! delete, mark_read.
//!
//! Cursor pagination uses `WHERE id < $cursor ORDER BY id DESC` per
//! backend.md §4. ULID ids make this work on the PK alone — no extra
//! `(created_at, id)` tiebreaker.
//!
//! Soft-deletes: `delete_message` sets `deleted_at` and clears `body`
//! rather than hard-deleting. `load_messages` filters out tombstones so
//! the user-visible behavior is identical, but a re-paginating client
//! that already has the message cached can still match by id when the
//! `MessageDeleted` event arrives.

use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::auth::{now_ms, CurrentUser};
use crate::channels::{channel_member_ids, require_membership};
use crate::entities::prelude::*;
use crate::entities::{channel_read, message, message_file};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::ws::bus::RoutedEvent;
use crate::ws::events::ServerEvent;

// ─────────────────────────────────────────────────────────────────────
// Wire types
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDto {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at: i64,
    pub file_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MessagePage {
    pub messages: Vec<MessageDto>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoadMessagesRequest {
    pub channel_id: String,
    pub before: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
pub struct PostMessageRequest {
    pub channel_id: String,
    pub body: String,
    pub file_ids: Vec<String>,
    pub client_msg_id: String,
}

#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    pub channel_id: String,
    pub message_id: String,
    pub new_body: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteMessageRequest {
    pub channel_id: String,
    pub message_id: String,
}

#[derive(Debug, Deserialize)]
pub struct MarkReadRequest {
    pub channel_id: String,
    pub last_message_id: String,
}

// ─────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────

pub async fn load_messages(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<LoadMessagesRequest>,
) -> AppResult<Json<MessagePage>> {
    require_membership(&state, &user.id, &req.channel_id).await?;

    let limit = req.limit.clamp(1, 100) as u64;

    let mut query = Messages::find()
        .filter(message::Column::ChannelId.eq(&req.channel_id))
        .filter(message::Column::DeletedAt.is_null())
        .order_by_desc(message::Column::Id)
        .limit(limit);

    if let Some(cursor) = req.before {
        query = query.filter(message::Column::Id.lt(cursor));
    }

    let rows = query.all(&state.db).await?;

    // If we got a full page back, there might be more.
    let next_cursor = if rows.len() as u64 == limit {
        rows.last().map(|m| m.id.clone())
    } else {
        None
    };

    // Look up file attachments for the page in one query, then fold
    // them onto messages by message_id. Avoids N+1 in busy channels.
    let msg_ids: Vec<String> = rows.iter().map(|m| m.id.clone()).collect();
    let file_links = if msg_ids.is_empty() {
        Vec::new()
    } else {
        MessageFiles::find()
            .filter(message_file::Column::MessageId.is_in(msg_ids))
            .all(&state.db)
            .await?
    };

    let messages = rows
        .into_iter()
        .map(|m| {
            let mut file_ids: Vec<(i32, String)> = file_links
                .iter()
                .filter(|l| l.message_id == m.id)
                .map(|l| (l.position, l.file_id.clone()))
                .collect();
            file_ids.sort_by_key(|(pos, _)| *pos);
            MessageDto {
                id: m.id,
                channel_id: m.channel_id,
                author_id: m.author_id,
                body: m.body,
                created_at: m.created_at,
                file_ids: file_ids.into_iter().map(|(_, id)| id).collect(),
            }
        })
        .collect();

    Ok(Json(MessagePage {
        messages,
        next_cursor,
    }))
}

pub async fn post_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<PostMessageRequest>,
) -> AppResult<Json<MessageDto>> {
    require_membership(&state, &user.id, &req.channel_id).await?;

    let body = req.body.trim().to_string();
    if body.is_empty() {
        return Err(AppError::BadRequest("body is empty".into()));
    }

    // Idempotency check: same (channel_id, client_msg_id) pair returns
    // the prior canonical row instead of inserting a duplicate. Saves
    // the client from double-posting on a flaky network.
    if let Some(existing) = Messages::find()
        .filter(message::Column::ChannelId.eq(&req.channel_id))
        .filter(message::Column::ClientMsgId.eq(&req.client_msg_id))
        .one(&state.db)
        .await?
    {
        return Ok(Json(MessageDto {
            id: existing.id,
            channel_id: existing.channel_id,
            author_id: existing.author_id,
            body: existing.body,
            created_at: existing.created_at,
            file_ids: Vec::new(),
        }));
    }

    let id = ulid::Ulid::new().to_string();
    let now = now_ms();

    // Inserting the message + its file links has to be atomic — if we
    // wrote the message but failed to link files, recipients would see
    // a message claiming to have attachments that aren't there.
    let txn = state.db.begin().await?;

    message::ActiveModel {
        id: Set(id.clone()),
        channel_id: Set(req.channel_id.clone()),
        author_id: Set(user.id.clone()),
        body: Set(body.clone()),
        created_at: Set(now),
        edited_at: Set(None),
        deleted_at: Set(None),
        client_msg_id: Set(Some(req.client_msg_id)),
    }
    .insert(&txn)
    .await?;

    for (pos, file_id) in req.file_ids.iter().enumerate() {
        message_file::ActiveModel {
            message_id: Set(id.clone()),
            file_id: Set(file_id.clone()),
            position: Set(pos as i32),
        }
        .insert(&txn)
        .await?;
    }

    txn.commit().await?;

    let file_ids = req.file_ids;

    let dto = MessageDto {
        id: id.clone(),
        channel_id: req.channel_id.clone(),
        author_id: user.id.clone(),
        body: body.clone(),
        created_at: now,
        file_ids: file_ids.clone(),
    };

    // Broadcast the new message to all channel members (including
    // sender — frontend dedupes by id against any local optimistic
    // insert).
    let recipients = channel_member_ids(&state, &req.channel_id).await?;
    let preview = preview_of(&body);
    let _ = state.bus.send(RoutedEvent {
        recipients: recipients.into_iter().collect(),
        event: ServerEvent::MessagePosted {
            channel_id: req.channel_id,
            message_id: id,
            author_id: user.id,
            preview,
            mentions_me: false,
            body: Some(body),
            created_at: now,
            file_ids,
        },
    });

    Ok(Json(dto))
}

pub async fn edit_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<EditMessageRequest>,
) -> AppResult<StatusCode> {
    let row = Messages::find_by_id(&req.message_id)
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    if row.channel_id != req.channel_id {
        return Err(AppError::NotFound);
    }
    if row.deleted_at.is_some() {
        return Err(AppError::NotFound);
    }
    if row.author_id != user.id {
        return Err(AppError::Forbidden);
    }

    let new_body = req.new_body.trim().to_string();
    if new_body.is_empty() {
        return Err(AppError::BadRequest("body is empty".into()));
    }

    let now = now_ms();
    let mut active: message::ActiveModel = row.into();
    active.body = Set(new_body.clone());
    active.edited_at = Set(Some(now));
    active.update(&state.db).await?;

    let recipients = channel_member_ids(&state, &req.channel_id).await?;
    let _ = state.bus.send(RoutedEvent {
        recipients: recipients.into_iter().collect(),
        event: ServerEvent::MessageEdited {
            channel_id: req.channel_id,
            message_id: req.message_id,
            body: new_body,
            edited_at: now,
        },
    });

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<DeleteMessageRequest>,
) -> AppResult<StatusCode> {
    let row = Messages::find_by_id(&req.message_id)
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    if row.channel_id != req.channel_id {
        return Err(AppError::NotFound);
    }
    if row.deleted_at.is_some() {
        // Already gone — return success so a retry doesn't 404.
        return Ok(StatusCode::NO_CONTENT);
    }
    if row.author_id != user.id {
        return Err(AppError::Forbidden);
    }

    let now = now_ms();
    let mut active: message::ActiveModel = row.into();
    active.body = Set(String::new());
    active.deleted_at = Set(Some(now));
    active.update(&state.db).await?;

    let recipients = channel_member_ids(&state, &req.channel_id).await?;
    let _ = state.bus.send(RoutedEvent {
        recipients: recipients.into_iter().collect(),
        event: ServerEvent::MessageDeleted {
            channel_id: req.channel_id,
            message_id: req.message_id,
        },
    });

    Ok(StatusCode::NO_CONTENT)
}

pub async fn mark_channel_read(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<MarkReadRequest>,
) -> AppResult<StatusCode> {
    require_membership(&state, &user.id, &req.channel_id).await?;

    let now = now_ms();

    // Upsert: if a read record exists, update; else insert. SQLite
    // doesn't have a clean SeaORM upsert helper — explicit lookup is
    // cheap on the (user_id, channel_id) PK.
    let existing = ChannelReads::find_by_id((user.id.clone(), req.channel_id.clone()))
        .one(&state.db)
        .await?;

    match existing {
        Some(row) => {
            let mut active: channel_read::ActiveModel = row.into();
            active.last_read_message_id = Set(req.last_message_id.clone());
            active.read_at = Set(now);
            active.update(&state.db).await?;
        }
        None => {
            channel_read::ActiveModel {
                user_id: Set(user.id.clone()),
                channel_id: Set(req.channel_id.clone()),
                last_read_message_id: Set(req.last_message_id),
                read_at: Set(now),
            }
            .insert(&state.db)
            .await?;
        }
    }

    // Fan out to *just this user's* connections — that's the whole
    // point of the event: tell the user's other devices to clear their
    // badges. Everyone else doesn't need to know.
    let _ = state.bus.send(RoutedEvent {
        recipients: [user.id.clone()].into_iter().collect(),
        event: ServerEvent::ChannelRead {
            channel_id: req.channel_id,
            user_id: user.id,
            last_read_at: now,
        },
    });

    Ok(StatusCode::NO_CONTENT)
}

// 200 chars is enough for a sidebar preview and avoids paying to ship
// long bodies in the WS frame when the client already has the body via
// MessagePosted's full `body` field.
fn preview_of(body: &str) -> String {
    let limit = 200;
    if body.chars().count() <= limit {
        body.to_string()
    } else {
        let truncated: String = body.chars().take(limit).collect();
        format!("{truncated}…")
    }
}
