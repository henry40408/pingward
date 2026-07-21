use crate::config::Config;
use crate::store::Store;
use axum::extract::FromRef;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Capacity of the live-tail event bus. A lagging subscriber just gets a
/// coalesced "changed" signal (see `web::sse_for_check`), so this only needs
/// to be big enough to absorb a burst between scan-loop ticks.
const EVENTS_CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
    /// Signal bus for the check-detail live tail: publishes a `check_id`
    /// whenever that check changes (a ping arrives, or the scan loop
    /// transitions it). Carries no payload data — subscribers re-fetch the
    /// existing HTML fragment instead.
    pub events: broadcast::Sender<i64>,
}

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        let (events, _rx) = broadcast::channel(EVENTS_CHANNEL_CAPACITY);
        Self {
            store,
            config: Arc::new(config),
            events,
        }
    }
}

impl FromRef<AppState> for Store {
    fn from_ref(state: &AppState) -> Store {
        state.store.clone()
    }
}

impl FromRef<AppState> for Arc<Config> {
    fn from_ref(state: &AppState) -> Arc<Config> {
        state.config.clone()
    }
}

impl FromRef<AppState> for broadcast::Sender<i64> {
    fn from_ref(state: &AppState) -> broadcast::Sender<i64> {
        state.events.clone()
    }
}
