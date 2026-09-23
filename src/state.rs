use crate::config::Config;
use crate::store::Store;
use axum::extract::FromRef;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Live-tail bus capacity. A lagging subscriber gets one coalesced "changed"
/// signal, so this only needs to absorb a burst.
const EVENTS_CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
    /// Live-tail signal: the `check_id` of a check that changed. Subscribers
    /// re-fetch the HTML fragment.
    pub events: broadcast::Sender<i64>,
    /// Login attempts per client address. The `Arc` is load-bearing: without it
    /// each `AppState` clone would get its own counters.
    pub login_limiter: Arc<crate::ratelimit::RateLimiter<std::net::IpAddr>>,
    /// Login attempts per submitted username, which a distributed attack cannot
    /// spread across addresses.
    pub account_limiter: Arc<crate::ratelimit::RateLimiter<String>>,
    /// Sessions recently re-authenticated for `/admin` grants; see `crate::elevate`.
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
