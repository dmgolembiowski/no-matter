//! Unread badge. Pure derivation — no fetching, no effects.
//!
//! This is the entire reason for the WebSocket-driven design: when
//! `UnreadStore` updates, this component re-renders. When other
//! channels' state changes, this component does *not* re-render
//! because the `Memo` only reads its own channel's slot.

use leptos::prelude::*;

use crate::stores::unread::UnreadStore;

#[component]
pub fn UnreadBadge(channel_id: String) -> impl IntoView {
    let store = expect_context::<UnreadStore>();
    let state = store.channel(channel_id);

    view! {
        <Show when=move || state.with(|s| s.count > 0)>
            <span
                class="badge"
                class:mention=move || state.with(|s| s.mention_count > 0)
            >
                {move || state.with(|s| {
                    if s.count > 99 { "99+".to_string() } else { s.count.to_string() }
                })}
            </span>
        </Show>
    }
}
