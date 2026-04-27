//! Channel/group/DM handlers.
//!
//! All endpoints are `POST` (or `GET` for read-only) under `/api/...`
//! and require a `CurrentUser`. Authorization is "are you a member of
//! the channel?" — checked via a join on `channel_members`.
//!
//! Channel-name uniqueness is enforced two ways: the partial unique
//! index in the migration, and a pre-check endpoint for instant UI
//! feedback. Both are case-insensitive (`lower(name)`).

use std::collections::HashSet;

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use sea_orm::{
    sea_query::{Expr, Func},
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::auth::{now_ms, CurrentUser};
use crate::entities::channel::ChannelKind as DbChannelKind;
use crate::entities::prelude::*;
use crate::entities::{channel, channel_member, user};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::ws::bus::RoutedEvent;
use crate::ws::events::{ChannelSummary, ServerEvent};

// ─────────────────────────────────────────────────────────────────────
// Wire types — mirror the frontend's `Channel` / `UserSummary`.
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelDto {
    pub id: String,
    pub name: String,
    /// Stringly-typed on the wire to keep the JSON identical to the
    /// frontend's `ChannelKind` (which uses `serde(rename_all = "snake_case")`).
    pub kind: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSummaryDto {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct InitialState {
    pub channels: Vec<ChannelDto>,
    pub users: Vec<UserSummaryDto>,
    pub current_user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CheckNameRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CheckNameResponse {
    pub available: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    /// "public" | "private" — group is its own endpoint, DMs use open_dm.
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddMemberRequest {
    pub channel_id: String,
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenDmRequest {
    pub other_user_id: String,
}

// ─────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────

pub async fn initial_state(
    State(state): State<AppState>,
    user: CurrentUser,
) -> AppResult<Json<InitialState>> {
    // 1. The user's channel ids.
    let memberships = ChannelMembers::find()
        .filter(channel_member::Column::UserId.eq(&user.id))
        .all(&state.db)
        .await?;
    let channel_ids: Vec<String> = memberships.iter().map(|m| m.channel_id.clone()).collect();

    // 2. The channels themselves + every member of each (one batch query).
    let channels_rows = if channel_ids.is_empty() {
        Vec::new()
    } else {
        Channels::find()
            .filter(channel::Column::Id.is_in(channel_ids.clone()))
            .all(&state.db)
            .await?
    };

    let all_member_rows = if channel_ids.is_empty() {
        Vec::new()
    } else {
        ChannelMembers::find()
            .filter(channel_member::Column::ChannelId.is_in(channel_ids))
            .all(&state.db)
            .await?
    };

    // Fold members into per-channel vectors.
    let mut channels: Vec<ChannelDto> = channels_rows
        .into_iter()
        .map(|c| ChannelDto {
            id: c.id,
            name: c.name,
            kind: kind_to_str(c.kind).to_string(),
            members: Vec::new(),
        })
        .collect();
    for m in all_member_rows {
        if let Some(c) = channels.iter_mut().find(|c| c.id == m.channel_id) {
            c.members.push(m.user_id);
        }
    }

    // 3. Every user the current user shares a channel with — drives
    // member pickers and DM display names. Plus the current user, so
    // the UI can resolve their own messages without a special case.
    let mut user_ids: HashSet<String> = channels.iter().flat_map(|c| c.members.clone()).collect();
    user_ids.insert(user.id.clone());

    let users_rows = if user_ids.is_empty() {
        Vec::new()
    } else {
        let ids: Vec<String> = user_ids.into_iter().collect();
        Users::find()
            .filter(user::Column::Id.is_in(ids))
            .all(&state.db)
            .await?
    };

    let users = users_rows
        .into_iter()
        .map(|u| UserSummaryDto {
            id: u.id,
            username: u.username,
        })
        .collect();

    Ok(Json(InitialState {
        channels,
        users,
        current_user_id: user.id,
    }))
}

pub async fn check_name(
    State(state): State<AppState>,
    _user: CurrentUser,
    Json(req): Json<CheckNameRequest>,
) -> AppResult<Json<CheckNameResponse>> {
    let trimmed = req.name.trim();
    if trimmed.is_empty() {
        return Ok(Json(CheckNameResponse { available: false }));
    }
    let lower = trimmed.to_lowercase();
    let existing = Channels::find()
        .filter(Expr::expr(Func::lower(Expr::col(channel::Column::Name))).eq(lower))
        .filter(channel::Column::Kind.ne(DbChannelKind::Dm))
        .one(&state.db)
        .await?;
    Ok(Json(CheckNameResponse {
        available: existing.is_none(),
    }))
}

pub async fn create_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateChannelRequest>,
) -> AppResult<Json<ChannelDto>> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name required".into()));
    }
    let kind = match req.kind.as_str() {
        "public" => DbChannelKind::Public,
        "private" => DbChannelKind::Private,
        // Groups have their own endpoint that requires member_ids.
        // DMs are created via open_dm. Reject anything else.
        other => {
            return Err(AppError::BadRequest(format!(
                "invalid kind {other:?}; use create_group or open_dm"
            )))
        }
    };

    let dto = create_with_members(&state, &user, name, kind, vec![user.id.clone()]).await?;
    Ok(Json(dto))
}

pub async fn create_group(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateGroupRequest>,
) -> AppResult<Json<ChannelDto>> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name required".into()));
    }
    if req.member_ids.is_empty() {
        return Err(AppError::BadRequest("at least one member required".into()));
    }

    // Ensure all the proposed members actually exist before we hit the
    // unique-name check; saves a useless conflict on garbage input.
    let count = Users::find()
        .filter(user::Column::Id.is_in(req.member_ids.clone()))
        .count(&state.db)
        .await? as usize;
    if count != req.member_ids.len() {
        return Err(AppError::BadRequest("unknown member(s)".into()));
    }

    // Auto-accept: caller is added too, plus the requested member ids.
    // De-duplicate to handle the case where the caller listed themselves.
    let mut members: Vec<String> = req.member_ids;
    if !members.contains(&user.id) {
        members.push(user.id.clone());
    }

    let dto =
        create_with_members(&state, &user, name, DbChannelKind::Group, members).await?;
    Ok(Json(dto))
}

pub async fn open_dm(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<OpenDmRequest>,
) -> AppResult<Json<ChannelDto>> {
    if req.other_user_id == user.id {
        return Err(AppError::BadRequest("can't DM yourself".into()));
    }

    // Confirm the other user exists.
    let other = Users::find_by_id(&req.other_user_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("unknown user".into()))?;

    // Find an existing DM channel with exactly these two members.
    // Two-membership join is cheap on small data; if this scales, add a
    // canonical (user_a, user_b) DM index.
    let candidate_ids: Vec<String> = ChannelMembers::find()
        .filter(channel_member::Column::UserId.eq(&user.id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.channel_id)
        .collect();

    let dms = if candidate_ids.is_empty() {
        Vec::new()
    } else {
        Channels::find()
            .filter(channel::Column::Id.is_in(candidate_ids))
            .filter(channel::Column::Kind.eq(DbChannelKind::Dm))
            .all(&state.db)
            .await?
    };

    for c in &dms {
        let members = channel_member_ids(&state, &c.id).await?;
        if members.len() == 2 && members.contains(&req.other_user_id) {
            return Ok(Json(ChannelDto {
                id: c.id.clone(),
                name: c.name.clone(),
                kind: kind_to_str(c.kind).to_string(),
                members,
            }));
        }
    }

    // No existing DM — create one. Name is the *other* user's username
    // from the caller's perspective; the frontend renders DMs by
    // looking up the non-self member id in `users`, so this name is
    // mostly for the audit trail.
    let dto = create_with_members(
        &state,
        &user,
        other.username.clone(),
        DbChannelKind::Dm,
        vec![user.id.clone(), req.other_user_id],
    )
    .await?;
    Ok(Json(dto))
}

pub async fn add_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<AddMemberRequest>,
) -> AppResult<StatusCode> {
    // Caller must be a member to add anyone.
    require_membership(&state, &user.id, &req.channel_id).await?;

    // No-op if the target is already a member.
    let exists = ChannelMembers::find()
        .filter(channel_member::Column::ChannelId.eq(&req.channel_id))
        .filter(channel_member::Column::UserId.eq(&req.user_id))
        .one(&state.db)
        .await?;
    if exists.is_some() {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Verify the new user actually exists.
    Users::find_by_id(&req.user_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("unknown user".into()))?;

    let now = now_ms();
    channel_member::ActiveModel {
        channel_id: Set(req.channel_id.clone()),
        user_id: Set(req.user_id.clone()),
        joined_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    // Tell the new member: a channel they didn't have just appeared.
    // Tell existing members: someone joined.
    let summary = load_summary(&state, &req.channel_id).await?;
    let recipients: HashSet<String> = summary.members.iter().cloned().collect();

    // The new member needs the full `channel_created` so their sidebar
    // gets a row to render; everyone else needs `member_added` so their
    // existing row updates its member list.
    let _ = state.bus.send(RoutedEvent {
        recipients: [req.user_id.clone()].into_iter().collect(),
        event: ServerEvent::ChannelCreated {
            channel: summary.clone(),
        },
    });
    let _ = state.bus.send(RoutedEvent {
        recipients: recipients
            .into_iter()
            .filter(|u| u != &req.user_id)
            .collect(),
        event: ServerEvent::MemberAdded {
            channel_id: req.channel_id,
            user_id: req.user_id,
        },
    });

    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────
// Helpers — public to other modules so message handlers can reuse them.
// ─────────────────────────────────────────────────────────────────────

pub fn kind_to_str(k: DbChannelKind) -> &'static str {
    match k {
        DbChannelKind::Public => "public",
        DbChannelKind::Private => "private",
        DbChannelKind::Dm => "dm",
        DbChannelKind::Group => "group",
    }
}

pub async fn require_membership(
    state: &AppState,
    user_id: &str,
    channel_id: &str,
) -> AppResult<()> {
    let exists = ChannelMembers::find()
        .filter(channel_member::Column::ChannelId.eq(channel_id))
        .filter(channel_member::Column::UserId.eq(user_id))
        .one(&state.db)
        .await?;
    if exists.is_none() {
        // 404 rather than 403 to avoid disclosing channel existence to
        // non-members.
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn channel_member_ids(
    state: &AppState,
    channel_id: &str,
) -> AppResult<Vec<String>> {
    Ok(ChannelMembers::find()
        .filter(channel_member::Column::ChannelId.eq(channel_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.user_id)
        .collect())
}

async fn load_summary(state: &AppState, channel_id: &str) -> AppResult<ChannelSummary> {
    let row = Channels::find_by_id(channel_id)
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let members = channel_member_ids(state, channel_id).await?;
    Ok(ChannelSummary {
        id: row.id,
        name: row.name,
        kind: kind_to_str(row.kind).to_string(),
        members,
    })
}

/// Atomic create-channel-with-members. Wrapped in a transaction because
/// a half-created channel (channel row but no member rows) would be
/// invisible to the creator and violate "members non-empty" implicitly.
async fn create_with_members(
    state: &AppState,
    creator: &CurrentUser,
    name: String,
    kind: DbChannelKind,
    member_ids: Vec<String>,
) -> AppResult<ChannelDto> {
    let txn = state.db.begin().await?;

    // Pre-check name (the unique index is the source of truth, but this
    // gives a clean conflict error rather than relying on string-match
    // on the DB error).
    if matches!(kind, DbChannelKind::Public | DbChannelKind::Private | DbChannelKind::Group) {
        let lower = name.to_lowercase();
        let existing = Channels::find()
            .filter(Expr::expr(Func::lower(Expr::col(channel::Column::Name))).eq(lower))
            .filter(channel::Column::Kind.ne(DbChannelKind::Dm))
            .one(&txn)
            .await?;
        if existing.is_some() {
            return Err(AppError::Conflict("name_taken"));
        }
    }

    let id = ulid::Ulid::new().to_string();
    let now = now_ms();
    channel::ActiveModel {
        id: Set(id.clone()),
        name: Set(name.clone()),
        kind: Set(kind),
        created_at: Set(now),
    }
    .insert(&txn)
    .await?;

    let mut deduped: Vec<String> = Vec::new();
    for m in member_ids {
        if !deduped.contains(&m) {
            deduped.push(m);
        }
    }
    for member_id in &deduped {
        channel_member::ActiveModel {
            channel_id: Set(id.clone()),
            user_id: Set(member_id.clone()),
            joined_at: Set(now),
        }
        .insert(&txn)
        .await?;
    }

    txn.commit().await?;

    let dto = ChannelDto {
        id: id.clone(),
        name,
        kind: kind_to_str(kind).to_string(),
        members: deduped.clone(),
    };

    // Broadcast: every member should see their sidebar grow.
    let summary = ChannelSummary {
        id: dto.id.clone(),
        name: dto.name.clone(),
        kind: dto.kind.clone(),
        members: dto.members.clone(),
    };
    let _ = state.bus.send(RoutedEvent {
        recipients: deduped.into_iter().collect(),
        event: ServerEvent::ChannelCreated { channel: summary },
    });

    let _ = creator; // silence unused-warning; useful for future auditing
    Ok(dto)
}
