//! Isomorphic server-fn surface.
//!
//! Modules here hold the wire types and `#[server]` functions the
//! frontend calls. With only the `csr` feature on, the macros expand
//! to typed HTTP RPC stubs; the actual server impls live in a separate
//! Axum service (see `claude/backend.md`, §7.1).

pub mod auth;
pub mod channels;
pub mod client;
pub mod messages;
