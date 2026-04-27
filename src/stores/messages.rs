//! Per-channel message cache.
//!
//! The previous version kept `messages` as a local `RwSignal` inside
//! `MessageList`. That works for pagination but breaks down the moment
//! something *outside* the component needs to mutate the list — most
//! notably the realtime bridge appending a `MessagePosted` event, or an
//! edit/delete arriving while the user is scrolled elsewhere.
//!
//! Lifting the cache here means:
//!   - `MessageList` becomes a pure view over `store.channel(id)`.
//!   - The bridge calls `store.append(...)` without knowing about components.
//!   - Switching channels is free — caches persist, so going back to a
//!     channel doesn't refetch unless the cache was evicted.
//!
//! Eviction is intentionally simple: an LRU over channel ids, capped at
//! `MAX_CACHED_CHANNELS`. Per-channel message lists are not capped here
//! because old messages are bounded by what the user actually paginated
//! to; if that becomes a memory issue, add a per-channel ring buffer.

use std::collections::HashMap;

use leptos::prelude::*;

use crate::server::messages::Message;

const MAX_CACHED_CHANNELS: usize = 32;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelMessages {
    pub messages: Vec<Message>,
    /// Cursor for the next *older* page. `None` after a fresh load means
    /// "haven't paginated yet"; `None` after pagination means "exhausted".
    pub older_cursor: Option<String>,
    pub exhausted: bool,
    /// Tracks LRU order for eviction.
    pub last_accessed: f64,
}

#[derive(Copy, Clone)]
pub struct MessageStore(pub RwSignal<HashMap<String, ChannelMessages>>);

impl MessageStore {
    pub fn new() -> Self {
        Self(RwSignal::new(HashMap::new()))
    }

    /// Read-only view of one channel. Components subscribe via this Memo,
    /// so an append to channel A doesn't re-render a list viewing channel B.
    pub fn channel(&self, channel_id: String) -> Memo<ChannelMessages> {
        let store = self.0;
        Memo::new(move |_| {
            store.with(|m| m.get(&channel_id).cloned().unwrap_or_default())
        })
    }

    /// Read-only view of a single message. Components subscribe via this
    /// Memo so edits flow through to row-level renders without forcing
    /// the whole For loop to remount keyed-by-id rows.
    pub fn message(&self, channel_id: String, message_id: String) -> Memo<Option<Message>> {
        let store = self.0;
        Memo::new(move |_| {
            store.with(|m| {
                m.get(&channel_id)?
                    .messages
                    .iter()
                    .find(|msg| msg.id == message_id)
                    .cloned()
            })
        })
    }

    /// Replace a page after a paginated fetch. `older` indicates whether
    /// these messages are older than what's already cached (prepend) or
    /// the initial load (replace).
    pub fn ingest_page(
        &self,
        channel_id: String,
        mut page: Vec<Message>,
        next_cursor: Option<String>,
        older: bool,
    ) {
        // Server returns DESC; we store ASC.
        page.reverse();

        self.0.update(|m| {
            self.evict_if_needed(m);
            let entry = m.entry(channel_id).or_default();
            entry.last_accessed = now();

            if older {
                // Prepend, but skip duplicates in case the cursor overlapped.
                let existing_ids: std::collections::HashSet<_> =
                    entry.messages.iter().map(|x| x.id.clone()).collect();
                page.retain(|m| !existing_ids.contains(&m.id));
                page.append(&mut entry.messages);
                entry.messages = page;
            } else {
                entry.messages = page;
            }

            entry.exhausted = next_cursor.is_none();
            entry.older_cursor = next_cursor;
        });
    }

    /// Append a single new message — used by the realtime bridge when a
    /// `MessagePosted` event arrives. No-op if the channel hasn't been
    /// loaded yet (the user will see the message when they open it).
    pub fn append(&self, channel_id: &str, msg: Message) {
        self.0.update(|m| {
            if let Some(entry) = m.get_mut(channel_id) {
                // Dedupe: optimistic local echoes may have inserted this
                // message already with the same id.
                if !entry.messages.iter().any(|x| x.id == msg.id) {
                    entry.messages.push(msg);
                }
                entry.last_accessed = now();
            }
        });
    }

    /// Optimistic insert for the local sender — shows the message
    /// instantly with a `pending` flag in the UI (not modeled in the
    /// `Message` struct here, but you'd add a `status` field).
    pub fn insert_optimistic(&self, channel_id: &str, msg: Message) {
        self.0.update(|m| {
            let entry = m.entry(channel_id.to_string()).or_default();
            entry.messages.push(msg);
            entry.last_accessed = now();
        });
    }

    /// Replace an optimistic message once the server confirms it. The
    /// optimistic id is replaced with the canonical server id.
    pub fn confirm_optimistic(
        &self,
        channel_id: &str,
        client_msg_id: &str,
        confirmed: Message,
    ) {
        self.0.update(|m| {
            if let Some(entry) = m.get_mut(channel_id) {
                if let Some(slot) = entry.messages.iter_mut().find(|x| x.id == client_msg_id) {
                    *slot = confirmed;
                }
            }
        });
    }

    pub fn edit(&self, channel_id: &str, message_id: &str, new_body: String) {
        self.0.update(|m| {
            if let Some(entry) = m.get_mut(channel_id) {
                if let Some(msg) = entry.messages.iter_mut().find(|x| x.id == message_id) {
                    msg.body = new_body;
                }
            }
        });
    }

    pub fn delete(&self, channel_id: &str, message_id: &str) {
        self.0.update(|m| {
            if let Some(entry) = m.get_mut(channel_id) {
                entry.messages.retain(|x| x.id != message_id);
            }
        });
    }

    fn evict_if_needed(&self, map: &mut HashMap<String, ChannelMessages>) {
        if map.len() < MAX_CACHED_CHANNELS {
            return;
        }
        // Find the LRU entry and drop it. A real LRU crate would be
        // tidier, but this is O(n) over a tiny n.
        if let Some((evict_key, _)) = map
            .iter()
            .min_by(|a, b| {
                a.1.last_accessed
                    .partial_cmp(&b.1.last_accessed)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(k, v)| (k.clone(), v.last_accessed))
        {
            map.remove(&evict_key);
        }
    }
}

impl Default for MessageStore {
    fn default() -> Self {
        Self::new()
    }
}

fn now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(0.0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }
}
