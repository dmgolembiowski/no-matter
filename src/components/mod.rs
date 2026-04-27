//! View components.
//!
//! Components are pure derivations from stores + server fns. They never
//! own state that another component needs to read; that always lives in
//! a store under `crate::stores`.

pub mod chat;
pub mod login;
pub mod media;
pub mod message_list;
pub mod modals;
pub mod shell;
pub mod sidebar;
pub mod unread_badge;
