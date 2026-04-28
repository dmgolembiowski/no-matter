//! Shared application state passed to every Axum handler.
//!
//! Cloning `AppState` is cheap — it's a pair of clonable handles
//! (`DatabaseConnection` is `Arc`-backed; `broadcast::Sender` is too).
//! Handlers get it via `State<AppState>`.

use sea_orm::DatabaseConnection;
use tokio::sync::broadcast;

use crate::ws::bus::RoutedEvent;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub bus: broadcast::Sender<RoutedEvent>,
}

impl AppState {
    pub fn new(db: DatabaseConnection) -> Self {
        // Capacity 1024 — enough that a momentarily-stalled WS task won't
        // drop events for everyone else under normal traffic. Slow tasks
        // that fall behind get an `RecvError::Lagged` and disconnect, by
        // design (clients reconnect and re-hydrate).
        let (bus, _) = broadcast::channel(1024);
        Self { db, bus }
    }
}
