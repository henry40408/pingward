use crate::config::Config;
use crate::store::Store;
use axum::extract::FromRef;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Capacity of the live-tail event bus. A lagging subscriber just gets a
/// coalesced "changed" signal, so this only needs to absorb a burst between
/// scan-loop ticks.
const EVENTS_CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
    /// Signal bus for the check-detail live tail: publishes a `check_id`
    /// whenever that check changes. Carries no payload — subscribers re-fetch
    /// the existing HTML fragment instead.
    pub events: broadcast::Sender<i64>,
    /// Login-attempt limiter keyed by client address, in-memory per-process.
    /// The `Arc` makes every `AppState::clone()` share one set of counters; a
    /// bare `RateLimiter` would give each clone its own and silently disable
    /// the control.
    pub login_limiter: Arc<crate::ratelimit::RateLimiter<std::net::IpAddr>>,
    /// Login-attempt limiter keyed by the submitted username. A per-address
    /// counter cannot see a distributed attack: N addresses simply buy N times
    /// the budget against one account. See `ratelimit::ACCOUNT_MAX_ATTEMPTS`.
    pub account_limiter: Arc<crate::ratelimit::RateLimiter<String>>,
    /// Which browser sessions have re-asserted their password recently, and so
    /// may perform the `/admin` actions that grant access. In-memory and
    /// per-process — see `crate::elevate`.
    pub elevations: Arc<crate::elevate::Elevations>,
}

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        let (events, _rx) = broadcast::channel(EVENTS_CHANNEL_CAPACITY);
        Self {
            store,
            config: Arc::new(config),
            events,
            login_limiter: Arc::new(crate::ratelimit::RateLimiter::new(
                crate::ratelimit::MAX_ATTEMPTS,
                crate::ratelimit::WINDOW_SECS,
            )),
            account_limiter: Arc::new(crate::ratelimit::RateLimiter::new(
                crate::ratelimit::ACCOUNT_MAX_ATTEMPTS,
                crate::ratelimit::ACCOUNT_WINDOW_SECS,
            )),
            elevations: Arc::new(crate::elevate::Elevations::new(
                crate::elevate::ELEVATION_TTL_SECS,
            )),
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
