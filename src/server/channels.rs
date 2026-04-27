//! Channel/group server fns.
//!
//! `check_channel_name` is a fast pre-check — used while the user types
//! to give instant "name taken" feedback. The authoritative check is
//! still the `create_channel` server fn, which the server gates with a
//! unique index. Two clients racing the same name will both see the
//! pre-check pass; one will succeed at create, the other gets a
//! `ServerError` with code `name_taken` and the modal surfaces it.

use serde::{Deserialize, Serialize};

use crate::server::client;
use crate::server::messages::ServerError;
use crate::stores::channels::{Channel, ChannelKind, UserSummary};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitialState {
    pub channels: Vec<Channel>,
    pub users: Vec<UserSummary>,
    pub current_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub kind: ChannelKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub member_ids: Vec<String>,
}

#[derive(Serialize)]
struct CheckNameWire<'a> {
    name: &'a str,
}

#[derive(Deserialize)]
struct CheckNameResponse {
    available: bool,
}

#[derive(Serialize)]
struct AddMemberWire<'a> {
    channel_id: &'a str,
    user_id: &'a str,
}

pub async fn list_initial_state() -> Result<InitialState, ServerError> {
    client::get("/api/initial_state").await
}

pub async fn check_channel_name(name: String) -> Result<bool, ServerError> {
    let resp: CheckNameResponse =
        client::post("/api/check_channel_name", &CheckNameWire { name: &name }).await?;
    Ok(resp.available)
}

pub async fn create_channel(req: CreateChannelRequest) -> Result<Channel, ServerError> {
    client::post("/api/create_channel", &req).await
}

pub async fn create_group(req: CreateGroupRequest) -> Result<Channel, ServerError> {
    client::post("/api/create_group", &req).await
}

pub async fn add_member(channel_id: String, user_id: String) -> Result<(), ServerError> {
    client::post_no_resp(
        "/api/add_member",
        &AddMemberWire {
            channel_id: &channel_id,
            user_id: &user_id,
        },
    )
    .await
}

#[derive(Serialize)]
struct OpenDmWire<'a> {
    other_user_id: &'a str,
}

/// Open or get a 1:1 DM channel with another user. Server returns the
/// existing channel if one is already present (idempotent).
pub async fn open_dm(other_user_id: String) -> Result<Channel, ServerError> {
    client::post(
        "/api/open_dm",
        &OpenDmWire {
            other_user_id: &other_user_id,
        },
    )
    .await
}
