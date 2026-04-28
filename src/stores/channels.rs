//! Channel/DM/group catalog.
//!
//! The three sidebar sections (Channels, DMs, Groups) are all backed by
//! the same `Channel` struct. They differ only in `kind` and how they're
//! displayed — DMs render as "@other_user" using the `users` map, groups
//! and channels render as their `name`.
//!
//! Why one store instead of three: cross-cutting events (`MessagePosted`,
//! `MemberAdded`) need to find a channel by id without caring what kind
//! it is. One `HashMap<id, Channel>` keeps that lookup O(1) and avoids
//! "is it here? oh, must be over there" branching in the bridge.

use std::collections::HashMap;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// Public channel — discoverable, anyone in the workspace can join.
    Public,
    /// Private channel — invite-only, members listed.
    Private,
    /// Direct message — exactly two members, displayed by the *other*
    /// member's name. Created lazily when first DM is sent.
    Dm,
    /// Group DM — 3+ members, displayed by name. Auto-accept: members
    /// added are joined immediately, no invitation flow.
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
}

#[derive(Copy, Clone)]
pub struct ChannelStore {
    pub channels: RwSignal<HashMap<String, Channel>>,
    pub users: RwSignal<HashMap<String, UserSummary>>,
}

impl ChannelStore {
    pub fn new() -> Self {
        Self {
            channels: RwSignal::new(HashMap::new()),
            users: RwSignal::new(HashMap::new()),
        }
    }

    pub fn hydrate(&self, channels: Vec<Channel>, users: Vec<UserSummary>) {
        self.channels.update(|m| {
            m.clear();
            for c in channels {
                m.insert(c.id.clone(), c);
            }
        });
        self.users.update(|m| {
            m.clear();
            for u in users {
                m.insert(u.id.clone(), u);
            }
        });
    }

    /// All channels of a given kind, sorted by name. Drives a sidebar
    /// section. The Memo only re-runs when the underlying map changes.
    pub fn by_kind(&self, kind: ChannelKind) -> Memo<Vec<Channel>> {
        let store = self.channels;
        Memo::new(move |_| {
            store.with(|m| {
                let mut v: Vec<Channel> = m.values().filter(|c| c.kind == kind).cloned().collect();
                v.sort_by(|a, b| a.name.cmp(&b.name));
                v
            })
        })
    }

    pub fn get(&self, channel_id: String) -> Memo<Option<Channel>> {
        let store = self.channels;
        Memo::new(move |_| store.with(|m| m.get(&channel_id).cloned()))
    }

    pub fn user(&self, user_id: String) -> Memo<Option<UserSummary>> {
        let store = self.users;
        Memo::new(move |_| store.with(|m| m.get(&user_id).cloned()))
    }

    pub fn upsert(&self, channel: Channel) {
        self.channels.update(|m| {
            m.insert(channel.id.clone(), channel);
        });
    }

    /// Upsert a fresh user directory snapshot without blowing away
    /// existing entries. Modals call this after fetching `/api/list_users`
    /// so newly-signed-up accounts become discoverable in member pickers.
    pub fn merge_users(&self, users: Vec<UserSummary>) {
        self.users.update(|m| {
            for u in users {
                m.insert(u.id.clone(), u);
            }
        });
    }

    pub fn add_member(&self, channel_id: &str, user_id: String) {
        self.channels.update(|m| {
            if let Some(c) = m.get_mut(channel_id) {
                if !c.members.contains(&user_id) {
                    c.members.push(user_id);
                }
            }
        });
    }

    /// Local pre-check before hitting the server. Channel and group names
    /// must be unique across kinds (we don't want a DM named "general"
    /// colliding with a channel named "general"). Case-insensitive — the
    /// server enforces the same rule via a unique index on lower(name).
    pub fn name_taken(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        self.channels
            .with_untracked(|m| m.values().any(|c| c.name.to_lowercase() == lower))
    }

    /// Snapshot of all known users — drives the member picker in the
    /// create-group / add-member modals.
    pub fn users_list(&self) -> Memo<Vec<UserSummary>> {
        let store = self.users;
        Memo::new(move |_| {
            let mut v: Vec<UserSummary> = store.with(|m| m.values().cloned().collect());
            v.sort_by(|a, b| a.username.cmp(&b.username));
            v
        })
    }
}

impl Default for ChannelStore {
    fn default() -> Self {
        Self::new()
    }
}
