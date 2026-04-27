//! Chat surface for one channel: header, message list, composer.
//!
//! The header renders the channel's display name plus contextual
//! actions (e.g. "Add member" on groups). The message list is the
//! existing paginated view. The composer posts to the server fn and
//! relies on the realtime bridge to deliver the canonical message back
//! via `MessagePosted`.
//!
//! Optimistic insertion is wired through `MessageStore::insert_optimistic`
//! when the user hits Send: the message appears immediately with a
//! pending state, and the WebSocket echo replaces it on confirm. If the
//! server-fn call errors before the echo arrives, we leave the
//! optimistic message in place but flag it visually; the user can retry.

use leptos::ev::SubmitEvent;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

use crate::components::message_list::MessageList;
use crate::server::messages::{post_message, upload_file, PostMessageRequest};
use crate::stores::channels::{ChannelKind, ChannelStore};
use crate::stores::messages::MessageStore;
use crate::stores::route::{Modal, RouteStore};

/// Lightweight in-memory model of a staged attachment in the composer.
/// Kept local to this component instead of in `UploadStore` for
/// simplicity — `UploadStore` is set up for a richer progress flow we
/// haven't fully wired yet.
#[derive(Clone, PartialEq)]
struct StagedFile {
    /// Once upload completes, this is the server-side file id. While
    /// uploading it's a synthetic local id so React-style key tracking
    /// in the For loop is stable.
    local_id: String,
    name: String,
    file_id: Option<String>,
    error: Option<String>,
}

#[component]
pub fn ChatInterface(channel_id: String) -> impl IntoView {
    let channels = expect_context::<ChannelStore>();
    let channel = channels.get(channel_id.clone());

    let header_id = channel_id.clone();
    let list_id = channel_id.clone();
    let composer_id = channel_id.clone();

    view! {
        <div class="chat">
            <ChatHeader channel_id=header_id channel=channel/>
            <div class="chat-scroll">
                <MessageList channel_id=list_id/>
            </div>
            <Composer channel_id=composer_id/>
        </div>
    }
}

#[component]
fn ChatHeader(
    channel_id: String,
    channel: Memo<Option<crate::stores::channels::Channel>>,
) -> impl IntoView {
    let route = expect_context::<RouteStore>();
    let cid_for_button = channel_id.clone();

    view! {
        <header class="chat-header">
            <h2 class="chat-title">
                {move || channel.with(|c| match c {
                    Some(c) => match c.kind {
                        ChannelKind::Public => format!("# {}", c.name),
                        ChannelKind::Private => format!("🔒 {}", c.name),
                        ChannelKind::Group => c.name.clone(),
                        ChannelKind::Dm => format!("@{}", c.name),
                    },
                    None => "Loading…".to_string(),
                })}
            </h2>

            // Add-member button only makes sense for groups (and private
            // channels, in a fuller impl). DMs/public channels skip it.
            <Show when=move || channel.with(|c| {
                matches!(c.as_ref().map(|x| x.kind), Some(ChannelKind::Group))
            })>
                <button
                    class="chat-action"
                    on:click={
                        let cid = cid_for_button.clone();
                        move |_| route.open_modal(Modal::AddMember(cid.clone()))
                    }
                >"Add member"</button>
            </Show>
        </header>
    }
}

#[component]
fn Composer(channel_id: String) -> impl IntoView {
    let messages = expect_context::<MessageStore>();
    let body = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let staged = RwSignal::new(Vec::<StagedFile>::new());
    let form_ref: NodeRef<html::Form> = NodeRef::new();
    let file_input: NodeRef<html::Input> = NodeRef::new();

    let cid = StoredValue::new(channel_id);

    // The set of fully-uploaded file_ids currently attached to the
    // draft. A staged file with `error.is_some()` is excluded so the
    // user can retry without that attachment going out.
    let ready_file_ids = move || -> Vec<String> {
        staged.with(|v| {
            v.iter()
                .filter_map(|s| s.file_id.clone())
                .collect()
        })
    };

    let any_uploading = move || staged.with(|v| v.iter().any(|s| s.file_id.is_none() && s.error.is_none()));

    let on_file_change = move |_| {
        let Some(input) = file_input.get() else { return };
        let Some(files) = input.files() else { return };
        for i in 0..files.length() {
            let Some(file) = files.item(i) else { continue };
            let local_id = client_msg_id();
            let name = file.name();
            staged.update(|v| v.push(StagedFile {
                local_id: local_id.clone(),
                name,
                file_id: None,
                error: None,
            }));
            let channel = cid.get_value();
            let local_id = local_id.clone();
            spawn_local(async move {
                match upload_file(&channel, &file).await {
                    Ok(id) => staged.update(|v| {
                        if let Some(s) = v.iter_mut().find(|s| s.local_id == local_id) {
                            s.file_id = Some(id);
                        }
                    }),
                    Err(e) => staged.update(|v| {
                        if let Some(s) = v.iter_mut().find(|s| s.local_id == local_id) {
                            s.error = Some(e.0);
                        }
                    }),
                }
            });
        }
        // Reset the input so picking the same file twice in a row works.
        input.set_value("");
    };

    let remove_staged = move |local_id: String| {
        staged.update(|v| v.retain(|s| s.local_id != local_id));
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        if any_uploading() {
            error.set(Some("Wait for uploads to finish".into()));
            return;
        }
        let text = body.get_untracked().trim().to_string();
        let file_ids = ready_file_ids();
        if text.is_empty() && file_ids.is_empty() {
            return;
        }

        pending.set(true);
        error.set(None);

        let channel_id = cid.get_value();
        let req = PostMessageRequest {
            channel_id: channel_id.clone(),
            body: text,
            file_ids,
            client_msg_id: client_msg_id(),
        };

        spawn_local(async move {
            match post_message(req).await {
                Ok(msg) => {
                    body.set(String::new());
                    staged.set(Vec::new());
                    messages.append(&channel_id, msg);
                }
                Err(e) => error.set(Some(format!("Send failed: {e}"))),
            }
            pending.set(false);
        });
    };

    view! {
        <form class="composer" node_ref=form_ref on:submit=submit>
            <Show when=move || !staged.with(|v| v.is_empty())>
                <ul class="composer-staged">
                    <For
                        each=move || staged.get()
                        key=|s| s.local_id.clone()
                        children=move |s: StagedFile| {
                            let id_for_remove = s.local_id.clone();
                            let is_loading = s.file_id.is_none() && s.error.is_none();
                            let err_text = s.error.clone();
                            let has_error = err_text.is_some();
                            let err_for_show = err_text.clone();
                            view! {
                                <li class="staged-chip"
                                    class:uploading=is_loading
                                    class:errored=has_error
                                >
                                    <span class="staged-icon">"📎"</span>
                                    <span class="staged-name">{s.name.clone()}</span>
                                    <Show when=move || is_loading>
                                        <span class="staged-status">"Uploading…"</span>
                                    </Show>
                                    <Show when=move || has_error>
                                        <span class="staged-status err">
                                            {err_for_show.clone().unwrap_or_default()}
                                        </span>
                                    </Show>
                                    <button
                                        type="button"
                                        class="staged-remove"
                                        on:click={
                                            let id = id_for_remove.clone();
                                            move |_| remove_staged(id.clone())
                                        }
                                    >"×"</button>
                                </li>
                            }
                        }
                    />
                </ul>
            </Show>

            <Show when=move || error.with(|e| e.is_some())>
                <div class="composer-error" role="alert">
                    {move || error.get().unwrap_or_default()}
                </div>
            </Show>

            <div class="composer-row">
                <button
                    type="button"
                    class="composer-attach"
                    title="Attach a file"
                    on:click=move |_| {
                        if let Some(el) = file_input.get() {
                            // Trigger native picker.
                            let _ = el.unchecked_ref::<web_sys::HtmlElement>().click();
                        }
                    }
                    disabled=move || pending.get()
                >
                    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/>
                    </svg>
                </button>
                <input
                    node_ref=file_input
                    type="file"
                    multiple="true"
                    style="display: none"
                    on:change=on_file_change
                />
                <textarea
                    class="composer-input"
                    placeholder="Write a message…"
                    rows="2"
                    prop:value=move || body.get()
                    on:input=move |ev| body.set(event_target_value(&ev))
                    on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                        if ev.key() == "Enter" && !ev.shift_key() {
                            ev.prevent_default();
                            if let Some(form) = form_ref.get() {
                                let _ = form.request_submit();
                            }
                        }
                    }
                    disabled=move || pending.get()
                />
                <button
                    class="composer-send"
                    type="submit"
                    disabled=move || {
                        pending.get()
                            || any_uploading()
                            || (body.with(|b| b.trim().is_empty())
                                && staged.with(|v| !v.iter().any(|s| s.file_id.is_some())))
                    }
                >
                    <Show
                        when=move || pending.get()
                        fallback=|| view! {
                            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <line x1="22" y1="2" x2="11" y2="13"/>
                                <polygon points="22 2 15 22 11 13 2 9 22 2"/>
                            </svg>
                        }
                    >
                        <span>"…"</span>
                    </Show>
                </button>
            </div>
        </form>
    }
}

fn client_msg_id() -> String {
    // Lightweight unique-ish id — good enough for the idempotency key
    // until we wire `uuid` with the `js` feature.
    let now = web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now() as u64)
        .unwrap_or(0);
    let rand = (js_sys::Math::random() * 1e9) as u64;
    format!("c-{now:x}-{rand:x}")
}
