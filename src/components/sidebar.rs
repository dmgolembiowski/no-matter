//! Sidebar navigation: three sections (channels, DMs, groups), each
//! with a "+" button and a list of items.
//!
//! Selecting an item updates `RouteStore::selected`; the body region
//! switches via a `Memo` over that value. The sidebar itself doesn't
//! know what the body looks like.

use leptos::prelude::*;

use crate::components::unread_badge::UnreadBadge;
use crate::stores::auth::AuthStore;
use crate::stores::channels::{Channel, ChannelKind, ChannelStore};
use crate::stores::route::{Modal, RouteStore, Selected};
use crate::theme::{Theme, ThemeStore};

#[component]
pub fn Sidebar() -> impl IntoView {
    let auth = expect_context::<AuthStore>();
    let theme_store = expect_context::<ThemeStore>();
    let theme = theme_store.current();
    let username = Memo::new(move |_| {
        auth.session()
            .with(|s| s.as_ref().map(|s| s.username.clone()).unwrap_or_default())
    });

    view! {
        <aside class="sidebar">
            <header class="sidebar-header">
                <span class="sidebar-username">{move || username.get()}</span>
                <button
                    class="sidebar-theme"
                    title=move || match theme.get() {
                        Theme::Dark => "Switch to light mode",
                        Theme::Light => "Switch to dark mode",
                    }
                    aria-label=move || match theme.get() {
                        Theme::Dark => "Switch to light mode",
                        Theme::Light => "Switch to dark mode",
                    }
                    on:click=move |_| theme_store.toggle()
                >
                    {move || match theme.get() {
                        // Show the icon of the *target* mode so the
                        // button telegraphs what clicking will do.
                        Theme::Dark => "☀",
                        Theme::Light => "☾",
                    }}
                </button>
                <button
                    class="sidebar-logout"
                    title="Sign out"
                    on:click=move |_| auth.clear()
                >
                    "Sign out"
                </button>
            </header>

            <Section
                title="Channels"
                kind=ChannelKind::Public
                modal=Modal::CreateChannel
                make_selected=|id| Selected::Channel(id)
            />
            <Section
                title="Private"
                kind=ChannelKind::Private
                modal=Modal::CreateChannel
                make_selected=|id| Selected::Channel(id)
            />
            <Section
                title="Direct Messages"
                kind=ChannelKind::Dm
                modal=Modal::StartDm
                make_selected=|id| Selected::Dm(id)
            />
            <Section
                title="Groups"
                kind=ChannelKind::Group
                modal=Modal::CreateGroup
                make_selected=|id| Selected::Group(id)
            />
        </aside>
    }
}

#[component]
fn Section(
    title: &'static str,
    kind: ChannelKind,
    modal: Modal,
    make_selected: fn(String) -> Selected,
) -> impl IntoView {
    let channels = expect_context::<ChannelStore>();
    let auth = expect_context::<AuthStore>();
    let route = expect_context::<RouteStore>();
    let items = channels.by_kind(kind);

    // Buttons aren't reactive to `modal`; they use it as a constant.
    let modal_for_button = modal.clone();

    view! {
        <section class="sidebar-section">
            <header class="sidebar-section-header">
                <span class="sidebar-section-title">{title}</span>
                <button
                    class="sidebar-add"
                    title=format!("New {}", title.to_lowercase())
                    on:click=move |_| route.open_modal(modal_for_button.clone())
                >"+"</button>
            </header>

            <Show
                when=move || !items.with(|v| v.is_empty())
                fallback=|| view! { <p class="sidebar-empty">"None yet"</p> }
            >
                <ul class="sidebar-list">
                    <For
                        each=move || items.get()
                        key=|c| c.id.clone()
                        children=move |c: Channel| {
                            let target = make_selected(c.id.clone());
                            let cid_for_badge = c.id.clone();
                            let is_active = {
                                let target = target.clone();
                                Memo::new(move |_| route.selected.with(|s| s == &target))
                            };
                            let label = display_label(c.clone(), auth, channels);
                            view! {
                                <li>
                                    <button
                                        class="sidebar-item"
                                        class:active=move || is_active.get()
                                        on:click={
                                            let target = target.clone();
                                            move |_| route.select(target.clone())
                                        }
                                    >
                                        <span class="sidebar-item-name">
                                            {move || label.get()}
                                        </span>
                                        <UnreadBadge channel_id=cid_for_badge/>
                                    </button>
                                </li>
                            }
                        }
                    />
                </ul>
            </Show>
        </section>
    }
}

/// Reactive sidebar label.
///
/// For DMs the stored `Channel.name` is whatever the *initiator* saw
/// when they opened the conversation (the other user's username from
/// their own perspective), so it's wrong from the recipient's side.
/// Compute the label from the membership instead: find the member that
/// isn't us, look up their username in `ChannelStore.users`, and prefix
/// with `@`. The username may not be cached yet at the moment a fresh
/// `ChannelCreated` event arrives — the realtime bridge kicks a user-
/// directory refresh in that case (see `src/realtime.rs`), and because
/// this is a `Memo` the label updates as soon as the lookup succeeds.
fn display_label(c: Channel, auth: AuthStore, channels: ChannelStore) -> Memo<String> {
    Memo::new(move |_| match c.kind {
        ChannelKind::Public => format!("# {}", c.name),
        ChannelKind::Private => format!("🔒 {}", c.name),
        ChannelKind::Group => c.name.clone(),
        ChannelKind::Dm => {
            let me = auth
                .session()
                .with(|s| s.as_ref().map(|s| s.user_id.clone()));
            let other_id = c
                .members
                .iter()
                .find(|m| me.as_ref() != Some(*m))
                .cloned();
            if let Some(other) = other_id {
                channels.user(other.clone()).with(|u| match u {
                    Some(u) => format!("@{}", u.username),
                    None => format!("@{other}"),
                })
            } else if !c.name.is_empty() {
                // Self-DM (single member): fall back to whatever the
                // server stored, then channel id as last resort.
                format!("@{}", c.name)
            } else {
                c.id.clone()
            }
        }
    })
}
