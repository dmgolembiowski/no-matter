//! Modal overlays. Mounted once at shell level; `ModalHost` switches on
//! `RouteStore::modal` and renders the appropriate one (or nothing).
//!
//! All modals share `<ModalShell>` for the backdrop/escape behavior so
//! the dismiss semantics are consistent. Modal bodies are independent
//! components that own their own form state and call out to server fns.

use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::server::channels::{
    add_member, check_channel_name, create_channel, create_group, open_dm, CreateChannelRequest,
    CreateGroupRequest,
};
use crate::server::messages::delete_message;
use crate::stores::channels::{ChannelKind, ChannelStore, UserSummary};
use crate::stores::messages::MessageStore;
use crate::stores::route::{Modal, RouteStore};

#[component]
pub fn ModalHost() -> impl IntoView {
    let route = expect_context::<RouteStore>();

    view! {
        {move || {
            let modal = route.modal.get();
            match modal {
                Modal::None => ().into_any(),
                Modal::CreateChannel => view! {
                    <ModalShell title="New channel">
                        <CreateChannelForm/>
                    </ModalShell>
                }.into_any(),
                Modal::CreateGroup => view! {
                    <ModalShell title="New group">
                        <CreateGroupForm/>
                    </ModalShell>
                }.into_any(),
                Modal::StartDm => view! {
                    <ModalShell title="New direct message">
                        <StartDmForm/>
                    </ModalShell>
                }.into_any(),
                Modal::AddMember(channel_id) => view! {
                    <ModalShell title="Add member">
                        <AddMemberForm channel_id=channel_id/>
                    </ModalShell>
                }.into_any(),
                Modal::ConfirmDeleteMessage { channel_id, message_id } => view! {
                    <ModalShell title="Delete message?">
                        <ConfirmDelete channel_id=channel_id message_id=message_id/>
                    </ModalShell>
                }.into_any(),
            }
        }}
    }
}

#[component]
fn ModalShell(title: &'static str, children: Children) -> impl IntoView {
    let route = expect_context::<RouteStore>();

    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Escape" {
            route.close_modal();
        }
    };

    view! {
        <div
            class="modal-backdrop"
            on:click=move |_| route.close_modal()
            on:keydown=on_keydown
            tabindex="0"
        >
            <div
                class="modal-card"
                on:click=|ev| ev.stop_propagation()
                role="dialog"
                aria-modal="true"
            >
                <header class="modal-header">
                    <h3 class="modal-title">{title}</h3>
                    <button
                        class="modal-close"
                        title="Close"
                        on:click=move |_| route.close_modal()
                    >"×"</button>
                </header>
                <div class="modal-body">
                    {children()}
                </div>
            </div>
        </div>
    }
}

// ─────────────────────────────────────────────────────────────────────
// Create channel
// ─────────────────────────────────────────────────────────────────────

#[component]
fn CreateChannelForm() -> impl IntoView {
    let route = expect_context::<RouteStore>();
    let channels = expect_context::<ChannelStore>();

    let name = RwSignal::new(String::new());
    let private = RwSignal::new(false);
    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let availability = RwSignal::new(Availability::Unknown);

    // Debounced server-side name check. Local check is the first gate;
    // the server check runs only when local says it's free.
    Effect::new(move |_| {
        let n = name.get();
        let trimmed = n.trim().to_string();
        if trimmed.is_empty() {
            availability.set(Availability::Unknown);
            return;
        }
        if channels.name_taken(&trimmed) {
            availability.set(Availability::Taken);
            return;
        }
        availability.set(Availability::Checking);
        spawn_local(async move {
            // Cheap debounce via a single sleep; per-keystroke spam is
            // fine because each keystroke spawns a fresh task and the
            // `availability` write is idempotent.
            gloo_timers::future::TimeoutFuture::new(300).await;
            // If the input changed while we were sleeping, drop this result.
            if name.get_untracked().trim() != trimmed {
                return;
            }
            match check_channel_name(trimmed.clone()).await {
                Ok(true) => availability.set(Availability::Available),
                Ok(false) => availability.set(Availability::Taken),
                // The check fn isn't implemented yet — surface as Unknown
                // so the user can still attempt to create. The server
                // remains the authority via its unique index.
                Err(_) => availability.set(Availability::Unknown),
            }
        });
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let trimmed = name.get_untracked().trim().to_string();
        if trimmed.is_empty() {
            error.set(Some("Name required".into()));
            return;
        }
        if matches!(availability.get_untracked(), Availability::Taken) {
            error.set(Some("That name is taken".into()));
            return;
        }
        pending.set(true);
        error.set(None);

        let kind = if private.get_untracked() {
            ChannelKind::Private
        } else {
            ChannelKind::Public
        };

        spawn_local(async move {
            match create_channel(CreateChannelRequest { name: trimmed, kind }).await {
                Ok(channel) => {
                    channels.upsert(channel);
                    route.close_modal();
                }
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("name_taken") {
                        error.set(Some("That name is taken".into()));
                        availability.set(Availability::Taken);
                    } else {
                        error.set(Some(format!("Create failed: {e}")));
                    }
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <form on:submit=submit class="modal-form">
            <label class="modal-field">
                <span class="modal-label">"Channel name"</span>
                <input
                    class="modal-input"
                    type="text"
                    placeholder="general"
                    autofocus="true"
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                    disabled=move || pending.get()
                />
                <AvailabilityHint state=availability/>
            </label>

            <label class="modal-checkbox">
                <input
                    type="checkbox"
                    prop:checked=move || private.get()
                    on:change=move |ev| private.set(event_target_checked(&ev))
                    disabled=move || pending.get()
                />
                <span>"Make this channel private"</span>
            </label>

            <Show when=move || error.with(|e| e.is_some())>
                <div class="modal-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>

            <footer class="modal-footer">
                <button
                    type="button"
                    class="modal-btn"
                    on:click=move |_| route.close_modal()
                    disabled=move || pending.get()
                >"Cancel"</button>
                <button
                    type="submit"
                    class="modal-btn primary"
                    disabled=move || pending.get()
                        || name.with(|n| n.trim().is_empty())
                        || matches!(availability.get(), Availability::Taken)
                >
                    {move || if pending.get() { "Creating…" } else { "Create" }}
                </button>
            </footer>
        </form>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Availability {
    Unknown,
    Checking,
    Available,
    Taken,
}

#[component]
fn AvailabilityHint(state: RwSignal<Availability>) -> impl IntoView {
    view! {
        <span class="modal-hint">
            {move || match state.get() {
                Availability::Unknown => view! { <span/> }.into_any(),
                Availability::Checking => view! {
                    <span class="hint-checking">"Checking…"</span>
                }.into_any(),
                Availability::Available => view! {
                    <span class="hint-ok">"Available"</span>
                }.into_any(),
                Availability::Taken => view! {
                    <span class="hint-bad">"Already taken"</span>
                }.into_any(),
            }}
        </span>
    }
}

// ─────────────────────────────────────────────────────────────────────
// Create group
// ─────────────────────────────────────────────────────────────────────

#[component]
fn CreateGroupForm() -> impl IntoView {
    let route = expect_context::<RouteStore>();
    let channels = expect_context::<ChannelStore>();
    let users = channels.users_list();

    let name = RwSignal::new(String::new());
    let selected_ids = RwSignal::new(Vec::<String>::new());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let toggle = move |id: String| {
        selected_ids.update(|v| {
            if let Some(pos) = v.iter().position(|x| x == &id) {
                v.remove(pos);
            } else {
                v.push(id);
            }
        });
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let trimmed = name.get_untracked().trim().to_string();
        if trimmed.is_empty() {
            error.set(Some("Group name required".into()));
            return;
        }
        if channels.name_taken(&trimmed) {
            error.set(Some("That name is taken".into()));
            return;
        }
        let members = selected_ids.get_untracked();
        if members.is_empty() {
            error.set(Some("Pick at least one member".into()));
            return;
        }
        pending.set(true);
        error.set(None);

        spawn_local(async move {
            match create_group(CreateGroupRequest {
                name: trimmed,
                member_ids: members,
            })
            .await
            {
                Ok(group) => {
                    channels.upsert(group);
                    route.close_modal();
                }
                Err(e) => {
                    error.set(Some(format!("Create failed: {e}")));
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <form on:submit=submit class="modal-form">
            <label class="modal-field">
                <span class="modal-label">"Group name"</span>
                <input
                    class="modal-input"
                    type="text"
                    placeholder="weekend-plans"
                    autofocus="true"
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                    disabled=move || pending.get()
                />
            </label>

            <div class="modal-field">
                <span class="modal-label">"Members"</span>
                <Show
                    when=move || !users.with(|v| v.is_empty())
                    fallback=|| view! {
                        <p class="modal-empty">"No other users to add yet."</p>
                    }
                >
                    <ul class="modal-userlist">
                        <For
                            each=move || users.get()
                            key=|u| u.id.clone()
                            children=move |u: UserSummary| {
                                let id_for_toggle = u.id.clone();
                                let id_for_check = u.id.clone();
                                let checked = Memo::new(move |_| {
                                    selected_ids.with(|v| v.contains(&id_for_check))
                                });
                                view! {
                                    <li>
                                        <label class="modal-userrow">
                                            <input
                                                type="checkbox"
                                                prop:checked=move || checked.get()
                                                on:change={
                                                    let id = id_for_toggle.clone();
                                                    move |_| toggle(id.clone())
                                                }
                                            />
                                            <span>{u.username.clone()}</span>
                                        </label>
                                    </li>
                                }
                            }
                        />
                    </ul>
                </Show>
            </div>

            <Show when=move || error.with(|e| e.is_some())>
                <div class="modal-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>

            <footer class="modal-footer">
                <button
                    type="button"
                    class="modal-btn"
                    on:click=move |_| route.close_modal()
                    disabled=move || pending.get()
                >"Cancel"</button>
                <button
                    type="submit"
                    class="modal-btn primary"
                    disabled=move || pending.get()
                >
                    {move || if pending.get() { "Creating…" } else { "Create" }}
                </button>
            </footer>
        </form>
    }
}

// ─────────────────────────────────────────────────────────────────────
// Add member
// ─────────────────────────────────────────────────────────────────────

#[component]
fn AddMemberForm(channel_id: String) -> impl IntoView {
    let route = expect_context::<RouteStore>();
    let channels = expect_context::<ChannelStore>();
    let users = channels.users_list();
    let channel = channels.get(channel_id.clone());

    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let cid = StoredValue::new(channel_id);

    let add = move |user_id: String| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(None);
        let cid = cid.get_value();
        let user_id_for_local = user_id.clone();
        spawn_local(async move {
            match add_member(cid.clone(), user_id).await {
                Ok(()) => {
                    channels.add_member(&cid, user_id_for_local);
                    pending.set(false);
                }
                Err(e) => {
                    error.set(Some(format!("Add failed: {e}")));
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <div class="modal-form">
            <Show
                when=move || !users.with(|v| v.is_empty())
                fallback=|| view! { <p class="modal-empty">"No users to add."</p> }
            >
                <ul class="modal-userlist">
                    <For
                        each=move || users.get()
                        key=|u| u.id.clone()
                        children=move |u: UserSummary| {
                            let id_for_btn = u.id.clone();
                            let id_for_check = u.id.clone();
                            let already = Memo::new(move |_| {
                                channel.with(|c| {
                                    c.as_ref().map(|c| c.members.contains(&id_for_check)).unwrap_or(false)
                                })
                            });
                            view! {
                                <li class="modal-userrow">
                                    <span>{u.username.clone()}</span>
                                    <Show
                                        when=move || !already.get()
                                        fallback=|| view! {
                                            <span class="modal-pill">"In group"</span>
                                        }
                                    >
                                        <button
                                            class="modal-btn"
                                            on:click={
                                                let id = id_for_btn.clone();
                                                move |_| add(id.clone())
                                            }
                                            disabled=move || pending.get()
                                        >"Add"</button>
                                    </Show>
                                </li>
                            }
                        }
                    />
                </ul>
            </Show>

            <Show when=move || error.with(|e| e.is_some())>
                <div class="modal-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>

            <footer class="modal-footer">
                <button
                    type="button"
                    class="modal-btn primary"
                    on:click=move |_| route.close_modal()
                >"Done"</button>
            </footer>
        </div>
    }
}

// ─────────────────────────────────────────────────────────────────────
// Confirm delete message
// ─────────────────────────────────────────────────────────────────────

#[component]
fn ConfirmDelete(channel_id: String, message_id: String) -> impl IntoView {
    let route = expect_context::<RouteStore>();
    let messages = expect_context::<MessageStore>();

    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let cid = StoredValue::new(channel_id);
    let mid = StoredValue::new(message_id);

    let confirm = move |_| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(None);
        let cid = cid.get_value();
        let mid = mid.get_value();
        spawn_local(async move {
            match delete_message(cid.clone(), mid.clone()).await {
                Ok(()) => {
                    messages.delete(&cid, &mid);
                    route.close_modal();
                }
                Err(e) => {
                    error.set(Some(format!("Delete failed: {e}")));
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <div class="modal-form">
            <p>"This message will be removed for everyone."</p>

            <Show when=move || error.with(|e| e.is_some())>
                <div class="modal-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>

            <footer class="modal-footer">
                <button
                    type="button"
                    class="modal-btn"
                    on:click=move |_| route.close_modal()
                    disabled=move || pending.get()
                >"Cancel"</button>
                <button
                    type="button"
                    class="modal-btn danger"
                    on:click=confirm
                    disabled=move || pending.get()
                >
                    {move || if pending.get() { "Deleting…" } else { "Delete" }}
                </button>
            </footer>
        </div>
    }
}

// ─────────────────────────────────────────────────────────────────────
// Start DM
// ─────────────────────────────────────────────────────────────────────

#[component]
fn StartDmForm() -> impl IntoView {
    let route = expect_context::<RouteStore>();
    let channels = expect_context::<ChannelStore>();
    let auth = expect_context::<crate::stores::auth::AuthStore>();
    let users_memo = channels.users_list();

    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    // Filter out the current user — you can't DM yourself.
    let users = Memo::new(move |_| {
        let me = auth.session().with(|s| s.as_ref().map(|s| s.user_id.clone()));
        users_memo.with(|all| {
            all.iter()
                .filter(|u| me.as_ref() != Some(&u.id))
                .cloned()
                .collect::<Vec<_>>()
        })
    });

    let pick = move |user_id: String| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(None);
        spawn_local(async move {
            match open_dm(user_id).await {
                Ok(channel) => {
                    let id = channel.id.clone();
                    channels.upsert(channel);
                    route.select(crate::stores::route::Selected::Dm(id));
                    route.close_modal();
                }
                Err(e) => {
                    error.set(Some(format!("Couldn't open DM: {e}")));
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <div class="modal-form">
            <Show
                when=move || !users.with(|v| v.is_empty())
                fallback=|| view! {
                    <p class="modal-empty">"No one else has joined yet."</p>
                }
            >
                <ul class="modal-userlist">
                    <For
                        each=move || users.get()
                        key=|u| u.id.clone()
                        children=move |u: UserSummary| {
                            let id_for_btn = u.id.clone();
                            view! {
                                <li class="modal-userrow">
                                    <span>{u.username.clone()}</span>
                                    <button
                                        class="modal-btn primary"
                                        on:click={
                                            let id = id_for_btn.clone();
                                            move |_| pick(id.clone())
                                        }
                                        disabled=move || pending.get()
                                    >"Message"</button>
                                </li>
                            }
                        }
                    />
                </ul>
            </Show>

            <Show when=move || error.with(|e| e.is_some())>
                <div class="modal-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>
        </div>
    }
}
