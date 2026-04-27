//! Authentication state.
//!
//! Holds the currently logged-in user's session. The whole UI gates on
//! `session.is_some()`: the splash login screen renders when None,
//! the app shell when Some.
//!
//! Token persistence is intentionally in-memory only. Wiring this to the
//! OS keychain (via `tauri-plugin-stronghold` or similar) is the right
//! "polished" answer but is its own piece of work — see backend.md §7.1
//! and the followup notes.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub user_id: String,
    pub username: String,
    pub server_url: String,
    pub token: String,
}

#[derive(Copy, Clone)]
pub struct AuthStore(pub RwSignal<Option<Session>>);

impl AuthStore {
    pub fn new() -> Self {
        Self(RwSignal::new(None))
    }

    /// Memo on whether the user is authed. Components gate render on this.
    pub fn is_authed(&self) -> Memo<bool> {
        let store = self.0;
        Memo::new(move |_| store.with(|s| s.is_some()))
    }

    /// Read-only view of the session. Returns `None` for the unauthed
    /// state. Most components should prefer `expect_session()` once they
    /// know they're past the gate.
    pub fn session(&self) -> Memo<Option<Session>> {
        let store = self.0;
        Memo::new(move |_| store.get())
    }

    /// Convenience for components that are only mounted post-auth — panics
    /// in the unauthed state, which would be a programming bug.
    pub fn expect_session(&self) -> Memo<Session> {
        let store = self.0;
        Memo::new(move |_| store.get().expect("AuthStore: session not set"))
    }

    pub fn set(&self, session: Session) {
        // Mirror into the ambient HTTP client so server-fn callers
        // automatically get the bearer token + base URL.
        crate::server::client::install(crate::server::client::Credentials {
            base_url: session.server_url.clone(),
            token: session.token.clone(),
        });
        self.0.set(Some(session));
    }

    pub fn clear(&self) {
        crate::server::client::clear();
        self.0.set(None);
    }

    /// Snapshot read for non-reactive callers (e.g. the WS connect step
    /// which only needs to look once).
    pub fn snapshot(&self) -> Option<Session> {
        self.0.get_untracked()
    }
}

impl Default for AuthStore {
    fn default() -> Self {
        Self::new()
    }
}
