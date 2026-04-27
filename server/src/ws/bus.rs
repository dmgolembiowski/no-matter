//! In-process event bus.
//!
//! Every mutating handler computes the recipient set (usually "the
//! current channel's members") and publishes a `RoutedEvent` onto the
//! broadcast channel. WS connections each hold their own receiver and
//! filter inbound `RoutedEvent`s by checking whether the connection's
//! user is in `recipients`.
//!
//! `recipients` is a `HashSet` because membership tests are O(1) and
//! the set is rebuilt per event rather than mutated, so the hashing
//! cost is paid once at publish time.

use std::collections::HashSet;

use super::events::ServerEvent;

#[derive(Debug, Clone)]
pub struct RoutedEvent {
    pub recipients: HashSet<String>,
    pub event: ServerEvent,
}
