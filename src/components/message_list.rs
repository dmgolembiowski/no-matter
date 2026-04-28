//! Channel message list with lazy upward pagination.
//!
//! The list is a *pure view* over `MessageStore::channel(id)`. The
//! store owns the per-channel `Vec<Message>`, cursor, and exhausted
//! flag — this component only triggers fetches and renders. Two
//! consequences:
//!
//!   - WS-driven appends from `MessagePosted` events arrive via the
//!     realtime bridge calling `MessageStore::append`, and re-render
//!     this list automatically.
//!
//!   - The user's own message, posted via the composer, is appended
//!     to the store after the server confirms — same path as any
//!     other arrival.
//!
//! An `IntersectionObserver` watches a sentinel `<div>` at the top of
//! the list. When it scrolls into view, we fetch the next older page
//! and ingest it via `MessageStore::ingest_page`. No timers, no scroll
//! handlers, no debouncing — the browser does the work.

use leptos::ev::SubmitEvent;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::IntersectionObserver;

use crate::markdown;
use crate::server::messages::{
    edit_message, load_messages, mark_channel_read, Message, MessagePage,
};
use crate::stores::auth::AuthStore;
use crate::stores::messages::MessageStore;
use crate::stores::route::{Modal, RouteStore};
use crate::stores::unread::UnreadStore;
use crate::theme::ThemeStore;

const PAGE_SIZE: u32 = 50;

#[component]
pub fn MessageList(channel_id: String) -> impl IntoView {
    let store = expect_context::<MessageStore>();
    let store_view = store.channel(channel_id.clone());

    let loading = RwSignal::new(false);
    // Whether the *initial* load for this channel has been issued.
    // Without this, switching back to a channel we've already loaded
    // would refetch from scratch and clobber any received realtime
    // messages.
    let initialized = RwSignal::new(false);
    let sentinel: NodeRef<html::Div> = NodeRef::new();

    let cid = StoredValue::new(channel_id.clone());

    // When the prop changes (parent re-renders with a different
    // channel_id), reset our local "initialized" guard so the new
    // channel triggers its own initial fetch. The store keeps both
    // channels' messages cached independently.
    Effect::new(move |_| {
        cid.set_value(channel_id.clone());
        initialized.set(false);
    });

    let load_more = move || {
        if loading.get_untracked() {
            return;
        }
        // Read pagination state *from the store*, not local signals.
        let snapshot = store_view.get_untracked();
        let already_loaded = initialized.get_untracked();
        if already_loaded && (snapshot.exhausted || snapshot.older_cursor.is_none() && !snapshot.messages.is_empty()) {
            // Already paginated; nothing more upward.
            if snapshot.exhausted {
                return;
            }
        }

        loading.set(true);
        let channel = cid.get_value();
        let before = if already_loaded {
            snapshot.older_cursor.clone()
        } else {
            None
        };
        let is_older = already_loaded;

        spawn_local(async move {
            match load_messages(channel.clone(), before, PAGE_SIZE).await {
                Ok(MessagePage {
                    messages: page,
                    next_cursor,
                }) => {
                    store.ingest_page(channel, page, next_cursor, is_older);
                    initialized.set(true);
                }
                Err(e) => leptos::logging::error!("load_messages: {e}"),
            }
            loading.set(false);
        });
    };

    // Initial load — runs on mount and whenever channel_id changes
    // (because `initialized` resets to false in the prop-change effect
    // above).
    Effect::new(move |_| {
        if !initialized.get() && !loading.get_untracked() {
            load_more();
        }
    });

    // IntersectionObserver lifecycle. Re-runs whenever the sentinel
    // node ref mounts; cleanup disconnects on unmount.
    Effect::new(move |_| {
        let Some(el) = sentinel.get() else { return };

        let cb = Closure::<dyn FnMut(Vec<web_sys::IntersectionObserverEntry>)>::new(
            move |entries: Vec<web_sys::IntersectionObserverEntry>| {
                if entries.iter().any(|e| e.is_intersecting()) {
                    load_more();
                }
            },
        );

        let observer = IntersectionObserver::new(cb.as_ref().unchecked_ref())
            .expect("IntersectionObserver unsupported");
        observer.observe(&el);

        // The closure must outlive this effect run — the observer holds
        // a JS-side reference to it.
        cb.forget();

        on_cleanup(move || observer.disconnect());
    });

    // Mark-read: a channel is in this component's view iff the user has
    // it selected (Shell only mounts MessageList for the active route).
    // So whenever this channel's UnreadStore slot says count > 0, the
    // user is engaging with it and the badge should be cleared. The
    // local clear is immediate; the server call persists the new
    // last_read_message_id and triggers a `ChannelRead` event so other
    // sessions of the same user (other tabs, other devices) match up.
    let unread = expect_context::<UnreadStore>();
    let unread_state = unread.channel(cid.get_value());
    Effect::new(move |_| {
        let snap = unread_state.get();
        if snap.count == 0 {
            return;
        }
        let Some(last_id) = snap.last_message_id.clone() else {
            return;
        };
        let channel = cid.get_value();
        unread.mark_read(&channel);
        spawn_local(async move {
            let _ = mark_channel_read(channel, last_id).await;
        });
    });

    view! {
        <div class="message-list">
            <div node_ref=sentinel class="sentinel">
                {move || {
                    if store_view.with(|c| c.exhausted) {
                        "Beginning of channel"
                    } else if loading.get() {
                        "Loading…"
                    } else {
                        ""
                    }
                }}
            </div>
            <For
                each=move || store_view.with(|c| c.messages.clone())
                key=|m| m.id.clone()
                children=|m: Message| {
                    // Pass ids only — MessageRow reads from the store
                    // reactively, so an `edit` after Save updates the
                    // visible body without remounting the row.
                    view! {
                        <MessageRow channel_id=m.channel_id message_id=m.id/>
                    }
                }
            />
        </div>
    }
}

#[component]
fn MessageRow(channel_id: String, message_id: String) -> impl IntoView {
    let auth = expect_context::<AuthStore>();
    let route = expect_context::<RouteStore>();
    let store = expect_context::<MessageStore>();
    let channels = expect_context::<crate::stores::channels::ChannelStore>();
    let theme = expect_context::<ThemeStore>().current();

    let msg = store.message(channel_id.clone(), message_id.clone());
    let editing = RwSignal::new(false);

    // Memos derived from `msg` — each only re-fires when its slice
    // changes, so the body update path doesn't redundantly re-evaluate
    // the author lookup or the is_mine comparison.
    let body = Memo::new(move |_| msg.with(|m| m.as_ref().map(|m| m.body.clone()).unwrap_or_default()));
    // Markdown HTML re-renders when either the body or the theme
    // changes — syntect's output is theme-baked, so a toggle has to
    // recompute the highlight spans for code blocks to repaint.
    let body_html = Memo::new(move |_| markdown::render(&body.get(), theme.get()));
    let author_id = Memo::new(move |_| msg.with(|m| m.as_ref().map(|m| m.author_id.clone()).unwrap_or_default()));
    let file_ids = Memo::new(move |_| msg.with(|m| m.as_ref().map(|m| m.file_ids.clone()).unwrap_or_default()));

    let is_mine = Memo::new(move |_| {
        let me = auth.session().with(|s| s.as_ref().map(|s| s.user_id.clone()));
        let them = author_id.get();
        me.as_ref().map_or(false, |id| id == &them)
    });

    // Friendly author label: prefer username from ChannelStore.users,
    // fall back to the raw id (ULID).
    let author_label = Memo::new(move |_| {
        let id = author_id.get();
        channels
            .user(id.clone())
            .with(|u| u.as_ref().map(|u| u.username.clone()).unwrap_or(id))
    });

    let cid = StoredValue::new(channel_id);
    let mid = StoredValue::new(message_id);

    let on_edit_click = move |_| editing.set(true);
    let on_delete_click = move |_| {
        route.open_modal(Modal::ConfirmDeleteMessage {
            channel_id: cid.get_value(),
            message_id: mid.get_value(),
        });
    };

    view! {
        <div class="message" class:own=move || is_mine.get()>
            <div class="message-side">
                <Avatar name=author_label/>
            </div>
            <div class="message-main">
                <div class="message-meta">
                    <span class="author">{move || author_label.get()}</span>
                    <Show when=move || is_mine.get() && !editing.get()>
                        <div class="message-actions">
                            <button class="message-action" title="Edit" on:click=on_edit_click>
                                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/>
                                </svg>
                            </button>
                            <button class="message-action danger" title="Delete" on:click=on_delete_click>
                                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/>
                                </svg>
                            </button>
                        </div>
                    </Show>
                </div>

                <Show
                    when=move || editing.get()
                    fallback=move || view! {
                        <div
                            class="body markdown"
                            inner_html=move || body_html.get()
                        ></div>
                    }
                >
                    <EditRow
                        initial=body.get_untracked()
                        channel_id=cid.get_value()
                        message_id=mid.get_value()
                        on_done=Callback::new(move |new_body: Option<String>| {
                            if let Some(b) = new_body {
                                store.edit(&cid.get_value(), &mid.get_value(), b);
                            }
                            editing.set(false);
                        })
                    />
                </Show>

                <Show when=move || !file_ids.with(|v| v.is_empty())>
                    <div class="attachments">
                        <For
                            each=move || file_ids.get()
                            key=|id| id.clone()
                            children=|id| view! {
                                <crate::components::media::MediaPreview file_id=id/>
                            }
                        />
                    </div>
                </Show>
            </div>
        </div>
    }
}

/// Inline avatar — colored initial derived from the display name. Same
/// algorithm everywhere so the same user always renders the same chip.
#[component]
fn Avatar(name: Memo<String>) -> impl IntoView {
    let initial = Memo::new(move |_| {
        name.with(|n| n.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_else(|| "?".into()))
    });
    let hue = Memo::new(move |_| {
        // Stable hash → hue. djb2 is ample for this.
        name.with(|n| {
            let mut h: u32 = 5381;
            for b in n.bytes() {
                h = h.wrapping_mul(33).wrapping_add(b as u32);
            }
            (h % 360) as i32
        })
    });
    view! {
        <span
            class="avatar"
            style=move || format!(
                "background: hsl({}, 65%, 42%); color: hsl({}, 80%, 92%);",
                hue.get(), hue.get(),
            )
        >
            {move || initial.get()}
        </span>
    }
}

#[component]
fn EditRow(
    initial: String,
    channel_id: String,
    message_id: String,
    on_done: Callback<Option<String>>,
) -> impl IntoView {
    let body = RwSignal::new(initial.clone());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let cid = StoredValue::new(channel_id);
    let mid = StoredValue::new(message_id);
    let initial_for_compare = StoredValue::new(initial);

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let trimmed = body.get_untracked().trim().to_string();
        if trimmed.is_empty() {
            error.set(Some("Message can't be empty".into()));
            return;
        }
        if initial_for_compare.with_value(|s| s == &trimmed) {
            on_done.run(None);
            return;
        }
        pending.set(true);
        error.set(None);

        let cid = cid.get_value();
        let mid = mid.get_value();
        let new_body = trimmed.clone();
        spawn_local(async move {
            match edit_message(cid, mid, new_body.clone()).await {
                Ok(()) => {
                    on_done.run(Some(new_body));
                }
                Err(e) => {
                    error.set(Some(format!("Edit failed: {e}")));
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <form class="message-edit" on:submit=submit>
            <textarea
                class="message-edit-input"
                rows="2"
                prop:value=move || body.get()
                on:input=move |ev| body.set(event_target_value(&ev))
                disabled=move || pending.get()
            />
            <Show when=move || error.with(|e| e.is_some())>
                <div class="composer-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>
            <div class="message-edit-actions">
                <button
                    type="button"
                    class="message-action"
                    on:click=move |_| on_done.run(None)
                    disabled=move || pending.get()
                >"Cancel"</button>
                <button
                    type="submit"
                    class="message-action primary"
                    disabled=move || pending.get()
                >
                    {move || if pending.get() { "Saving…" } else { "Save" }}
                </button>
            </div>
        </form>
    }
}
