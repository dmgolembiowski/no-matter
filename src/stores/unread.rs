//! Global unread state.
//!
//! One `RwSignal<HashMap<ChannelId, UnreadState>>` lives at the app root
//! and is updated by the realtime bridge. Every badge, sidebar dot, and
//! mention counter derives from it via `Memo`, which means: when a
//! `MessagePosted` event arrives for channel X, only the components
//! reading channel X re-render. The other channels' badges don't even
//! re-evaluate their map lookup.

use std::collections::HashMap;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

pub type ChannelId = String;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnreadState {
    pub count: u32,
    pub mention_count: u32,
    pub last_message_id: Option<String>,
}

#[derive(Copy, Clone)]
pub struct UnreadStore(pub RwSignal<HashMap<ChannelId, UnreadState>>);

impl UnreadStore {
    pub fn new() -> Self {
        Self(RwSignal::new(HashMap::new()))
    }

    /// Hydrate from server-rendered initial state (e.g. user's last-read
    /// snapshot fetched at boot).
    pub fn hydrate(&self, initial: HashMap<ChannelId, UnreadState>) {
        self.0.set(initial);
    }

    /// A read-only view of one channel's unread state. Cheap to call from
    /// many components — Leptos memoizes per channel id.
    pub fn channel(&self, channel_id: ChannelId) -> Memo<UnreadState> {
        let store = self.0;
        Memo::new(move |_| store.with(|m| m.get(&channel_id).cloned().unwrap_or_default()))
    }

    pub fn bump(&self, channel_id: ChannelId, message_id: String, mention: bool) {
        self.0.update(|m| {
            let entry = m.entry(channel_id).or_default();
            entry.count = entry.count.saturating_add(1);
            if mention {
                entry.mention_count = entry.mention_count.saturating_add(1);
            }
            entry.last_message_id = Some(message_id);
        });
    }

    pub fn mark_read(&self, channel_id: &str) {
        self.0.update(|m| {
            if let Some(s) = m.get_mut(channel_id) {
                s.count = 0;
                s.mention_count = 0;
            }
        });
    }

    /// Sum of all mentions across channels — drives the dock-icon badge
    /// the OS shows on macOS / the notification count on Windows.
    pub fn total_mentions(&self) -> Memo<u32> {
        let store = self.0;
        Memo::new(move |_| store.with(|m| m.values().map(|s| s.mention_count).sum()))
    }
}

impl Default for UnreadStore {
    fn default() -> Self {
        Self::new()
    }
}
