//! Splash login + signup screen.
//!
//! Three fields: server URL, username, password — plus a "Create
//! account" toggle that swaps the submit handler to call `signup`
//! instead of `login`. Same form shape, since the server's request
//! payload is identical for both.
//!
//! Validation is intentionally light. Server URL is parsed for an
//! obvious typo (must start with `http://` or `https://` and have a
//! host); username/password are checked for non-emptiness, and signup
//! requires a length floor that matches the server's. The server is
//! authoritative — bad credentials surface as a `ServerError` shown
//! inline. We don't hash or salt client-side; that's the server's job
//! over TLS.

use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::server::auth::{login, signup, LoginRequest};
use crate::stores::auth::AuthStore;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    SignIn,
    CreateAccount,
}

#[component]
pub fn LoginPage() -> impl IntoView {
    let auth = expect_context::<AuthStore>();

    let mode = RwSignal::new(Mode::SignIn);
    let server_url = RwSignal::new(String::from("http://localhost:8080"));
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());

    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let validate = move || -> Option<String> {
        let url = server_url.get();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Some("Server URL must start with http:// or https://".into());
        }
        let after_scheme = url.split("://").nth(1).unwrap_or("");
        if after_scheme.trim().is_empty() {
            return Some("Server URL needs a host".into());
        }
        if username.with(|u| u.trim().is_empty()) {
            return Some("Username is required".into());
        }
        if password.with(|p| p.is_empty()) {
            return Some("Password is required".into());
        }
        // Match the server's floors so the user gets the message
        // before round-tripping a request.
        if mode.get() == Mode::CreateAccount {
            if username.with(|u| u.trim().len() < 2) {
                return Some("Username must be at least 2 characters".into());
            }
            if password.with(|p| p.len() < 6) {
                return Some("Password must be at least 6 characters".into());
            }
        }
        None
    };

    let on_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }

        if let Some(msg) = validate() {
            error.set(Some(msg));
            return;
        }
        error.set(None);
        pending.set(true);

        let req = LoginRequest {
            username: username.get_untracked().trim().to_string(),
            password: password.get_untracked(),
            server_url: server_url.get_untracked().trim_end_matches('/').to_string(),
        };
        let current_mode = mode.get_untracked();

        spawn_local(async move {
            let result = match current_mode {
                Mode::SignIn => login(req).await,
                Mode::CreateAccount => signup(req).await,
            };
            match result {
                Ok(session) => {
                    auth.set(session);
                }
                Err(e) => {
                    let prefix = match current_mode {
                        Mode::SignIn => "Sign in failed",
                        Mode::CreateAccount => "Sign up failed",
                    };
                    error.set(Some(format!("{prefix}: {e}")));
                    pending.set(false);
                }
            }
        });
    };

    let toggle_mode = move |_| {
        if pending.get_untracked() {
            return;
        }
        error.set(None);
        mode.update(|m| {
            *m = match *m {
                Mode::SignIn => Mode::CreateAccount,
                Mode::CreateAccount => Mode::SignIn,
            };
        });
    };

    view! {
        <div class="login-screen">
            <div class="login-card">
                <h1 class="login-title">"no-matter"</h1>
                <p class="login-subtitle">
                    {move || match mode.get() {
                        Mode::SignIn => "Sign in to your workspace",
                        Mode::CreateAccount => "Create an account on your workspace",
                    }}
                </p>

                <form class="login-form" on:submit=on_submit>
                    <label class="login-field">
                        <span class="login-label">"Server URL"</span>
                        <input
                            class="login-input"
                            type="url"
                            autocomplete="url"
                            placeholder="https://chat.example.com"
                            prop:value=move || server_url.get()
                            on:input=move |ev| server_url.set(event_target_value(&ev))
                            disabled=move || pending.get()
                        />
                    </label>

                    <label class="login-field">
                        <span class="login-label">"Username"</span>
                        <input
                            class="login-input"
                            type="text"
                            autocomplete=move || match mode.get() {
                                Mode::SignIn => "username",
                                Mode::CreateAccount => "username",
                            }
                            prop:value=move || username.get()
                            on:input=move |ev| username.set(event_target_value(&ev))
                            disabled=move || pending.get()
                        />
                    </label>

                    <label class="login-field">
                        <span class="login-label">"Password"</span>
                        <input
                            class="login-input"
                            type="password"
                            autocomplete=move || match mode.get() {
                                Mode::SignIn => "current-password",
                                Mode::CreateAccount => "new-password",
                            }
                            prop:value=move || password.get()
                            on:input=move |ev| password.set(event_target_value(&ev))
                            disabled=move || pending.get()
                        />
                    </label>

                    <Show when=move || error.with(|e| e.is_some())>
                        <div class="login-error" role="alert">
                            {move || error.get().unwrap_or_default()}
                        </div>
                    </Show>

                    <button
                        class="login-submit"
                        type="submit"
                        disabled=move || pending.get()
                    >
                        {move || match (mode.get(), pending.get()) {
                            (Mode::SignIn, true)        => "Signing in…",
                            (Mode::SignIn, false)       => "Sign in",
                            (Mode::CreateAccount, true) => "Creating…",
                            (Mode::CreateAccount, false)=> "Create account",
                        }}
                    </button>

                    <button
                        type="button"
                        class="login-toggle"
                        on:click=toggle_mode
                        disabled=move || pending.get()
                    >
                        {move || match mode.get() {
                            Mode::SignIn => "Need an account? Create one",
                            Mode::CreateAccount => "Already have an account? Sign in",
                        }}
                    </button>
                </form>
            </div>
        </div>
    }
}
