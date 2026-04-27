//! Authentication: signup, login, logout, /me, and the `CurrentUser`
//! extractor that guards every other handler.
//!
//! Password hashing uses Argon2id with library defaults; secrets never
//! cross the wire in plaintext aside from the initial POST body over
//! TLS. Session tokens are 32 random bytes, base64url-encoded for the
//! client. The DB stores only `sha256(token)` so a backup leak doesn't
//! hand out live sessions.
//!
//! The same `Authorization: Bearer …` header authenticates HTTP and
//! the WS gateway re-uses the same lookup via a `?token=` query param —
//! see `ws.rs::authenticate`.

use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2, PasswordHash, PasswordVerifier,
};
use axum::{
    extract::{FromRef, FromRequestParts, State},
    http::{header, request::Parts, StatusCode},
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::entities::prelude::*;
use crate::entities::{session, user};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ─────────────────────────────────────────────────────────────────────
// Wire types
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub user_id: String,
    pub username: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user_id: String,
    pub username: String,
}

// ─────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────

pub async fn signup(
    State(state): State<AppState>,
    Json(req): Json<SignupRequest>,
) -> AppResult<Json<AuthResponse>> {
    let username = req.username.trim().to_string();
    if username.len() < 2 {
        return Err(AppError::BadRequest("username too short".into()));
    }
    if req.password.len() < 6 {
        return Err(AppError::BadRequest("password too short".into()));
    }

    let existing = Users::find()
        .filter(user::Column::Username.eq(&username))
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(AppError::Conflict("username_taken"));
    }

    let id = ulid::Ulid::new().to_string();
    let now = now_ms();
    let password_hash = hash_password(&req.password)?;

    let model = user::ActiveModel {
        id: Set(id.clone()),
        username: Set(username.clone()),
        password_hash: Set(password_hash),
        created_at: Set(now),
    };
    model.insert(&state.db).await?;

    let token = issue_token(&state, &id).await?;
    Ok(Json(AuthResponse {
        user_id: id,
        username,
        token,
    }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    let username = req.username.trim().to_string();
    let user = Users::find()
        .filter(user::Column::Username.eq(&username))
        .one(&state.db)
        .await?
        .ok_or(AppError::Unauthenticated)?;

    if !verify_password(&req.password, &user.password_hash)? {
        return Err(AppError::Unauthenticated);
    }

    let token = issue_token(&state, &user.id).await?;
    Ok(Json(AuthResponse {
        user_id: user.id,
        username: user.username,
        token,
    }))
}

pub async fn logout(
    State(state): State<AppState>,
    user: CurrentUser,
) -> AppResult<StatusCode> {
    // Revoke every session this user has — simple and good enough for
    // a desktop client. Multi-device users can re-auth.
    Sessions::delete_many()
        .filter(session::Column::UserId.eq(&user.id))
        .exec(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(user: CurrentUser) -> Json<MeResponse> {
    Json(MeResponse {
        user_id: user.id,
        username: user.username,
    })
}

// ─────────────────────────────────────────────────────────────────────
// Token issuance + verification
// ─────────────────────────────────────────────────────────────────────

const SESSION_LIFETIME_MS: i64 = 30 * 24 * 60 * 60 * 1000; // 30 days

async fn issue_token(state: &AppState, user_id: &str) -> AppResult<String> {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let token_hash = sha256_hex(&token);

    let now = now_ms();
    let row = session::ActiveModel {
        id: Set(ulid::Ulid::new().to_string()),
        user_id: Set(user_id.to_string()),
        token_hash: Set(token_hash),
        expires_at: Set(now + SESSION_LIFETIME_MS),
        created_at: Set(now),
    };
    row.insert(&state.db).await?;
    Ok(token)
}

pub async fn lookup_token(
    db: &sea_orm::DatabaseConnection,
    raw_token: &str,
) -> AppResult<user::Model> {
    let token_hash = sha256_hex(raw_token);
    let now = now_ms();

    let row = Sessions::find()
        .filter(session::Column::TokenHash.eq(token_hash))
        .filter(session::Column::ExpiresAt.gt(now))
        .one(db)
        .await?
        .ok_or(AppError::Unauthenticated)?;

    Users::find_by_id(row.user_id)
        .one(db)
        .await?
        .ok_or(AppError::Unauthenticated)
}

// ─────────────────────────────────────────────────────────────────────
// Extractor: every authenticated handler takes `CurrentUser` as a param
// ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
    AppState: axum::extract::FromRef<S>,
{
    type Rejection = axum::response::Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::Unauthenticated.into_response())?;

        let user = lookup_token(&app_state.db, token)
            .await
            .map_err(IntoResponse::into_response)?;

        Ok(CurrentUser {
            id: user.id,
            username: user.username,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("argon2 hash: {e}")))
}

fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("argon2 parse: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
