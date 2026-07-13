use crate::config::Config;
use crate::store::Store;
use axum::extract::FromRef;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        Self {
            store,
            config: Arc::new(config),
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
