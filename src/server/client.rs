//! HTTP client plumbing for the server-fn modules.
//!
//! Two layers:
//!
//!   - **Ambient credentials** (`Credentials { base_url, token }`) live
//!     in a process-global `OnceLock<RwLock<…>>`. `AuthStore::set`
//!     populates it; `AuthStore::clear` empties it. Every authenticated
//!     server fn pulls from it without threading state through every
//!     call site.
//!
//!   - **Request helpers** (`post`, `get`, `post_no_resp`,
//!     `post_unauth`) handle JSON encode/decode and error mapping. They
//!     return `Result<_, ServerError>` so the existing call-site
//!     signatures don't change.
//!
//! Why an ambient global is OK here: this is a single-tenant desktop
//! app, only one user logged in at a time, the `AuthStore` is the sole
//! writer. Same justification as `panic!` on missing context — if the
//! ambient state isn't installed, nothing should be calling auth'd
//! server fns.

use std::sync::{OnceLock, RwLock};

use gloo_net::http::{Request, Response};
use serde::{de::DeserializeOwned, Serialize};

use crate::server::messages::ServerError;

#[derive(Clone, Debug)]
pub struct Credentials {
    pub base_url: String,
    pub token: String,
}

static STATE: OnceLock<RwLock<Option<Credentials>>> = OnceLock::new();

fn cell() -> &'static RwLock<Option<Credentials>> {
    STATE.get_or_init(|| RwLock::new(None))
}

pub fn install(creds: Credentials) {
    *cell().write().unwrap() = Some(creds);
}

pub fn clear() {
    *cell().write().unwrap() = None;
}

fn current() -> Result<Credentials, ServerError> {
    cell()
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| ServerError("not signed in".into()))
}

// ─────────────────────────────────────────────────────────────────────
// Authenticated requests — read base_url + token from the ambient cell.
// ─────────────────────────────────────────────────────────────────────

pub async fn post<Req: Serialize, Resp: DeserializeOwned>(
    path: &str,
    body: &Req,
) -> Result<Resp, ServerError> {
    let creds = current()?;
    let resp = build_post(&creds.base_url, Some(&creds.token), path, body)?
        .send()
        .await
        .map_err(map_err)?;
    parse_json(check_ok(resp).await?).await
}

pub async fn post_no_resp<Req: Serialize>(path: &str, body: &Req) -> Result<(), ServerError> {
    let creds = current()?;
    let resp = build_post(&creds.base_url, Some(&creds.token), path, body)?
        .send()
        .await
        .map_err(map_err)?;
    check_ok(resp).await?;
    Ok(())
}

pub async fn get<Resp: DeserializeOwned>(path: &str) -> Result<Resp, ServerError> {
    let creds = current()?;
    let url = format!("{}{}", creds.base_url.trim_end_matches('/'), path);
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", creds.token))
        .send()
        .await
        .map_err(map_err)?;
    parse_json(check_ok(resp).await?).await
}

// ─────────────────────────────────────────────────────────────────────
// Unauthenticated — used by login/signup, where we don't yet have a
// token but need to talk to a specific server URL.
// ─────────────────────────────────────────────────────────────────────

pub async fn post_unauth<Req: Serialize, Resp: DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &Req,
) -> Result<Resp, ServerError> {
    let resp = build_post(base_url, None, path, body)?
        .send()
        .await
        .map_err(map_err)?;
    parse_json(check_ok(resp).await?).await
}

/// Multipart upload of a `web_sys::FormData`. Returns the parsed JSON
/// response. Used by the composer's attach flow — gloo-net's body type
/// accepts `JsValue`, so we hand it the FormData directly.
pub async fn post_multipart<Resp: DeserializeOwned>(
    path: &str,
    form: &web_sys::FormData,
) -> Result<Resp, ServerError> {
    let creds = current()?;
    let url = format!("{}{}", creds.base_url.trim_end_matches('/'), path);
    // Pass the FormData through wasm_bindgen's JsValue so gloo-net's
    // `body()` accepts it (web_sys::FormData implements
    // `AsRef<JsValue>` but the inference needs a hint).
    let body: wasm_bindgen::JsValue = form.clone().into();
    let req = Request::post(&url)
        .header("Authorization", &format!("Bearer {}", creds.token))
        .body(body)
        .map_err(map_err)?;
    let resp = req.send().await.map_err(map_err)?;
    parse_json(check_ok(resp).await?).await
}

/// Build an absolute URL for an asset by combining the ambient base
/// URL with a server-relative path (typically returned in `FileMeta`).
/// Falls back to the path itself if no credentials are installed,
/// which means the WebView's relative resolution will (incorrectly)
/// kick in — caller should treat that case as an error path, but
/// returning *something* renderable beats panicking.
pub fn absolute_url(path: &str) -> String {
    match current() {
        Ok(creds) => format!("{}{}", creds.base_url.trim_end_matches('/'), path),
        Err(_) => path.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Internals
// ─────────────────────────────────────────────────────────────────────

fn build_post<Req: Serialize>(
    base_url: &str,
    token: Option<&str>,
    path: &str,
    body: &Req,
) -> Result<Request, ServerError> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let mut builder = Request::post(&url);
    if let Some(t) = token {
        builder = builder.header("Authorization", &format!("Bearer {t}"));
    }
    builder.json(body).map_err(map_err)
}

async fn check_ok(resp: Response) -> Result<Response, ServerError> {
    if resp.ok() {
        return Ok(resp);
    }
    // Try to surface the server's structured error code as part of the
    // message — modals match on substrings like `name_taken` so the
    // code needs to be visible.
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if let Ok(parsed) = serde_json::from_str::<ErrorBody>(&text) {
        return Err(ServerError(format!("{} [{}]", parsed.error, parsed.code)));
    }
    Err(ServerError(format!("{status}: {text}")))
}

async fn parse_json<Resp: DeserializeOwned>(resp: Response) -> Result<Resp, ServerError> {
    resp.json::<Resp>().await.map_err(map_err)
}

fn map_err<E: std::fmt::Display>(e: E) -> ServerError {
    ServerError(e.to_string())
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error: String,
    code: String,
}
