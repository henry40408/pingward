use crate::config::Config;
use crate::store::Store;
use axum::extract::FromRef;
use std::sync::{Arc, Mutex};
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
    /// `POST /setup` attempts per client address, charged before the token is checked
    /// and never refunded: a correct token ends setup anyway.
    pub setup_limiter: Arc<crate::ratelimit::RateLimiter<std::net::IpAddr>>,
    /// First-run token, memory only. `None` once an admin exists, and also when the
    /// process never armed one, which refuses every `/setup` (fail closed).
    pub setup_token: Arc<Mutex<Option<String>>>,
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
            setup_limiter: Arc::new(crate::ratelimit::RateLimiter::new(
                crate::ratelimit::SETUP_MAX_ATTEMPTS,
                crate::ratelimit::SETUP_WINDOW_SECS,
            )),
            setup_token: Arc::new(Mutex::new(None)),
        }
    }

    /// Makes `token` the one `/setup` accepts. `main` arms a generated token at boot
    /// when no user exists; tests call it to learn the token.
    pub fn arm_setup_token(&self, token: String) {
        *self.lock_setup_token() = Some(token);
    }

    /// Arms a fresh random token only while no user exists; returns it for the operator.
    pub async fn arm_setup_token_if_unclaimed(&self) -> Result<Option<String>, sqlx::Error> {
        if self.store.count_users().await? > 0 {
            return Ok(None);
        }
        let token = crate::secret::generate_setup_token();
        self.arm_setup_token(token.clone());
        Ok(Some(token))
    }

    /// The armed token, if any.
    pub fn setup_token(&self) -> Option<String> {
        self.lock_setup_token().clone()
    }

    /// Forgets the token once the first admin exists.
    pub fn clear_setup_token(&self) {
        *self.lock_setup_token() = None;
    }

    fn lock_setup_token(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        // A poisoned lock still holds a valid `Option`.
        self.setup_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
