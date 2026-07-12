use pingward::{
    config::Config,
    db,
    notify::{Notifier, WebhookNotifier},
    scheduler,
    store::Store,
};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env();
    let pool = db::connect(&config.database_url)
        .await
        .expect("failed to connect to database");
    db::migrate(&pool).await.expect("failed to run migrations");
    let store = Store::new(pool);

    // Plan 1 bound: a single global webhook from env; per-check channels come in Plan 2.
    let mut notifiers: Vec<Box<dyn Notifier>> = Vec::new();
    if let Ok(url) = std::env::var("PINGWARD_WEBHOOK_URL") {
        tracing::warn!(
            "Plan 1: using single global PINGWARD_WEBHOOK_URL; per-check channels come in Plan 2"
        );
        notifiers.push(Box::new(WebhookNotifier::new(url)));
    }
    let notifiers = Arc::new(notifiers);

    let bind = config.bind.clone();
    let scan_interval_secs = config.scan_interval_secs;

    tokio::spawn(scheduler::run_scan_loop(
        store.clone(),
        scan_interval_secs,
        notifiers,
    ));

    let state = pingward::state::AppState::new(store, config);

    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
