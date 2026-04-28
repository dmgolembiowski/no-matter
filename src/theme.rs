//! Theme (dark/light) state, persistence, and DOM application.
//!
//! The toggle is a single signal in context. An `Effect` mounted in
//! `App` applies the current value to `<html data-theme="…">` and
//! mirrors it into `localStorage`, so a reload picks up the user's
//! choice. CSS is the source of truth for what each theme looks like
//! (see `styles.css`); this module only chooses which one is active.
//!
//! On first run, with nothing in `localStorage`, we honor the OS-level
//! `prefers-color-scheme: light` query. Anything else, including the
//! user explicitly toggling, sticks.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "no-matter:theme";

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

#[derive(Copy, Clone)]
pub struct ThemeStore(pub RwSignal<Theme>);

impl ThemeStore {
    pub fn new() -> Self {
        Self(RwSignal::new(initial_theme()))
    }

    pub fn current(&self) -> Memo<Theme> {
        let s = self.0;
        Memo::new(move |_| s.get())
    }

    pub fn toggle(&self) {
        self.0.update(|t| *t = t.toggle());
    }
}

impl Default for ThemeStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Push the chosen theme into the DOM and `localStorage`. Called from a
/// reactive `Effect` so it fires on every change without callers having
/// to remember to plumb both sinks.
pub fn apply(theme: Theme) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(el) = doc.document_element() {
            let _ = el.set_attribute("data-theme", theme.as_str());
        }
    }
    if let Some(win) = web_sys::window() {
        if let Ok(Some(storage)) = win.local_storage() {
            let _ = storage.set_item(STORAGE_KEY, theme.as_str());
        }
    }
}

fn initial_theme() -> Theme {
    if let Some(stored) = read_storage().and_then(|s| Theme::parse(&s)) {
        return stored;
    }
    if prefers_light() {
        Theme::Light
    } else {
        Theme::Dark
    }
}

fn read_storage() -> Option<String> {
    let win = web_sys::window()?;
    let storage = win.local_storage().ok()??;
    storage.get_item(STORAGE_KEY).ok()?
}

fn prefers_light() -> bool {
    let Some(win) = web_sys::window() else {
        return false;
    };
    match win.match_media("(prefers-color-scheme: light)") {
        Ok(Some(m)) => m.matches(),
        _ => false,
    }
}
