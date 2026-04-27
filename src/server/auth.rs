//! Auth server fns.
//!
//! Each fn translates the public request type into the wire shape the
//! HTTP server expects, then maps the response back into the
//! `AuthStore`-friendly `Session`. The `server_url` is a frontend-only
//! concern (the server doesn't know its own URL); we tack it back on
//! before returning so the caller can pass the result straight into
//! `AuthStore::set`.

use serde::{Deserialize, Serialize};

use crate::server::client;
use crate::server::messages::ServerError;
use crate::stores::auth::Session;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub server_url: String,
}

#[derive(Serialize)]
struct LoginWire<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Deserialize)]
struct AuthResponse {
    user_id: String,
    username: String,
    token: String,
}

pub async fn login(req: LoginRequest) -> Result<Session, ServerError> {
    let server_url = req.server_url.trim_end_matches('/').to_string();
    let resp: AuthResponse = client::post_unauth(
        &server_url,
        "/api/login",
        &LoginWire {
            username: req.username.trim(),
            password: &req.password,
        },
    )
    .await?;
    Ok(Session {
        user_id: resp.user_id,
        username: resp.username,
        server_url,
        token: resp.token,
    })
}

pub async fn signup(req: LoginRequest) -> Result<Session, ServerError> {
    let server_url = req.server_url.trim_end_matches('/').to_string();
    let resp: AuthResponse = client::post_unauth(
        &server_url,
        "/api/signup",
        &LoginWire {
            username: req.username.trim(),
            password: &req.password,
        },
    )
    .await?;
    Ok(Session {
        user_id: resp.user_id,
        username: resp.username,
        server_url,
        token: resp.token,
    })
}

pub async fn logout() -> Result<(), ServerError> {
    client::post_no_resp("/api/logout", &serde_json::json!({})).await
}

#[derive(Deserialize)]
struct MeResponse {
    user_id: String,
    username: String,
}

/// Used after reconnect to confirm the token is still good. Caller
/// supplies `server_url` + `token` since `/me` doesn't echo them.
pub async fn me(server_url: String, token: String) -> Result<Session, ServerError> {
    let resp: MeResponse = client::get("/api/me").await?;
    Ok(Session {
        user_id: resp.user_id,
        username: resp.username,
        server_url,
        token,
    })
}
