//! Lazy media preview.
//!
//! `FileShared` events arriving over the WebSocket only carry a `file_id`.
//! We don't fetch metadata until the message containing it actually
//! renders — and we don't fetch the *full* media until the user clicks.
//! `Resource` handles caching across re-renders for free.

use leptos::prelude::*;

use crate::server::client::absolute_url;
use crate::server::messages::{get_file_meta, FileMeta};

#[component]
pub fn MediaPreview(file_id: String) -> impl IntoView {
    // `LocalResource` (rather than `Resource::new`) because the
    // underlying `gloo-net` future is `!Send` — it holds JS interop
    // values. CSR is single-threaded, so this is fine; we just can't
    // use the Send-flavored Resource that would normally allow
    // SSR-isomorphic fetching.
    let id = file_id.clone();
    let meta = LocalResource::new(move || {
        let id = id.clone();
        async move { get_file_meta(id).await.map_err(|e| e.0) }
    });

    view! {
        <Suspense fallback=|| view! { <div class="media-skeleton"/> }>
            {move || meta.get().map(|wrapped| {
                // LocalResource hands back a `SendWrapper<T>`; take the
                // inner value so the match works against the real Result.
                match wrapped.take() {
                    Ok(FileMeta { mime, thumb_url, url, name, .. }) => {
                        // Server returns server-relative URLs; resolve
                        // against the ambient base so the WebView (or
                        // a `tauri://` origin) hits the right host.
                        let full_url = absolute_url(&url);
                        let thumb_full = thumb_url.as_deref().map(absolute_url);
                        let is_image = mime.starts_with("image/");
                        view! {
                            <a class="media" href=full_url.clone() target="_blank">
                                {if is_image {
                                    view! {
                                        <img
                                            src=thumb_full.unwrap_or_else(|| full_url.clone())
                                            alt=name.clone()
                                            loading="lazy"
                                        />
                                    }.into_any()
                                } else {
                                    view! {
                                        <span class="file-name">
                                            <span class="file-icon">"📄"</span>
                                            {name.clone()}
                                        </span>
                                    }.into_any()
                                }}
                            </a>
                        }.into_any()
                    }
                    Err(_) => view! { <span class="media-error">"unavailable"</span> }.into_any(),
                }
            })}
        </Suspense>
    }
}
