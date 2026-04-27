//! Typing indicators.
//!
//! Each `Typing { channel_id, user_id }` event extends that user's
//! "typing until" timestamp by the dwell window (5s). A single timer
//! per channel sweeps expired entries and triggers a re-render. We don't
//! sweep eagerly on every event because that would mean a ton of timer
//! churn in a busy channel; instead, the sweep is debounced to once per
//! second per channel that has any active typers.
//!
//! Why a per-channel sweep instead of a single global one: typing events
//! are sparse, and per-channel timers only run when that channel
//! actually has typers. A global 1Hz tick would re-render every
//! component subscribed to the typing store every second, even idle ones.

use std::collections::HashMap;

use leptos::prelude::*;
use leptos::task::spawn_local;

const DWELL_MS: f64 = 5_000.0;
const SWEEP_INTERVAL_MS: i32 = 1_000;

#[derive(Debug, Clone, Default)]
pub struct ChannelTyping {
    /// user_id -> expiry timestamp (ms since epoch / performance.now()).
    pub typers: HashMap<String, f64>,
}

impl ChannelTyping {
    /// Returns the list of currently-typing users, filtered to non-expired.
    pub fn active(&self, now: f64) -> Vec<String> {
        let mut active: Vec<String> = self
            .typers
            .iter()
            .filter(|(_, expiry)| **expiry > now)
            .map(|(uid, _)| uid.clone())
            .collect();
        active.sort();
        active
    }
}

#[derive(Copy, Clone)]
pub struct TypingStore(pub RwSignal<HashMap<String, ChannelTyping>>);

impl TypingStore {
    pub fn new() -> Self {
        Self(RwSignal::new(HashMap::new()))
    }

    /// Read view: the list of users currently typing in a channel,
    /// excluding the current user (passed in to filter out self-echo).
    pub fn channel(&self, channel_id: String, exclude_user_id: String) -> Memo<Vec<String>> {
        let store = self.0;
        Memo::new(move |_| {
            let now = now();
            store.with(|m| {
                m.get(&channel_id)
                    .map(|c| {
                        let mut active = c.active(now);
                        active.retain(|u| u != &exclude_user_id);
                        active
                    })
                    .unwrap_or_default()
            })
        })
    }

    /// Record a typing event. Idempotent — repeated events for the same
    /// user just bump the expiry forward.
    pub fn record(&self, channel_id: String, user_id: String) {
        let expiry = now() + DWELL_MS;
        let was_empty = self.0.with_untracked(|m| {
            m.get(&channel_id)
                .map(|c| c.typers.is_empty())
                .unwrap_or(true)
        });

        self.0.update(|m| {
            let entry = m.entry(channel_id.clone()).or_default();
            entry.typers.insert(user_id, expiry);
        });

        // Start a sweep loop for this channel if it just became active.
        if was_empty {
            self.start_sweep(channel_id);
        }
    }

    fn start_sweep(&self, channel_id: String) {
        let store = self.0;
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(SWEEP_INTERVAL_MS as u32).await;

                let still_active = store.try_update(|m| {
                    let now = now();
                    if let Some(entry) = m.get_mut(&channel_id) {
                        entry.typers.retain(|_, expiry| *expiry > now);
                        if entry.typers.is_empty() {
                            m.remove(&channel_id);
                            false
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                });

                // `try_update` returns None if the signal was disposed
                // (app teardown) — exit cleanly in that case.
                match still_active {
                    Some(true) => continue,
                    _ => break,
                }
            }
        });
    }
}

impl Default for TypingStore {
    fn default() -> Self {
        Self::new()
    }
}

fn now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}
