//! Authenticated app shell: sidebar on the left, body on the right.
//!
//! The body switches on `RouteStore::selected`:
//!   - `Selected::None` → welcome card
//!   - any of the channel variants → `ChatInterface` for that id
//!
//! Modals (CreateChannel, CreateGroup, …) are mounted here at shell
//! level so they overlay everything regardless of which body view is
//! active.

use leptos::prelude::*;

use crate::components::chat::ChatInterface;
use crate::components::modals::ModalHost;
use crate::components::sidebar::Sidebar;
use crate::stores::route::{RouteStore, Selected};

#[component]
pub fn Shell() -> impl IntoView {
    let route = expect_context::<RouteStore>();

    view! {
        <div class="shell">
            <Sidebar/>
            <main class="shell-body">
                {move || match route.selected.get() {
                    Selected::None => view! { <Welcome/> }.into_any(),
                    Selected::Channel(id) | Selected::Dm(id) | Selected::Group(id) => {
                        view! { <ChatInterface channel_id=id/> }.into_any()
                    }
                }}
            </main>
            <ModalHost/>
        </div>
    }
}

#[component]
fn Welcome() -> impl IntoView {
    view! {
        <div class="welcome">
            <h2 class="welcome-title">"Pick a conversation"</h2>
            <p class="welcome-subtitle">
                "Choose a channel, direct message, or group from the sidebar to start chatting."
            </p>
        </div>
    }
}
