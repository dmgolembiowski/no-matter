//! In-memory navigation state.
//!
//! Tracks two pieces of UI state that the rest of the tree reads from:
//!
//!   - `selected`: which sidebar item is active. The body region renders
//!     a `ChatInterface` over it, or a welcome card if `None`.
//!   - `modal`: which modal (if any) is mounted on top. We don't need a
//!     stack — only one modal can be open at a time in this UI.
//!
//! Why not `leptos_router`: a desktop chat client doesn't expose URL
//! navigation to users, and routing-on-URL would force us to model
//! transient modal state as either query params or out-of-band, both of
//! which are messier than a tiny in-memory store.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Selected {
    #[default]
    None,
    Channel(String),
    Dm(String),
    Group(String),
}

impl Selected {
    /// All four variants resolve back to a channel id (DMs are channels
    /// with `kind = Dm` keyed by the DM's channel id, not the other
    /// user's id — the sidebar handles user-id-to-DM-channel resolution).
    pub fn channel_id(&self) -> Option<&str> {
        match self {
            Selected::Channel(id) | Selected::Dm(id) | Selected::Group(id) => Some(id.as_str()),
            Selected::None => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Modal {
    #[default]
    None,
    CreateChannel,
    CreateGroup,
    StartDm,
    AddMember(String),
    ConfirmDeleteMessage {
        channel_id: String,
        message_id: String,
    },
}

#[derive(Copy, Clone)]
pub struct RouteStore {
    pub selected: RwSignal<Selected>,
    pub modal: RwSignal<Modal>,
}

impl RouteStore {
    pub fn new() -> Self {
        Self {
            selected: RwSignal::new(Selected::None),
            modal: RwSignal::new(Modal::None),
        }
    }

    pub fn select(&self, sel: Selected) {
        self.selected.set(sel);
    }

    pub fn open_modal(&self, modal: Modal) {
        self.modal.set(modal);
    }

    pub fn close_modal(&self) {
        self.modal.set(Modal::None);
    }
}

impl Default for RouteStore {
    fn default() -> Self {
        Self::new()
    }
}
