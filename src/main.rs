use pingward::{
    config::{Config, LogFormat},
    db, scheduler,
    state::AppState,
    store::Store,
};

/// Install the global tracing subscriber. `RUST_LOG` (via `EnvFilter`) controls
/// verbosity; `format` selects the human-readable text renderer or line-delimited
/// JSON for a log aggregator.
fn init_tracing(format: LogFormat) {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Json => builder.json().init(),
        LogFormat::Text => builder.init(),
    }
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    init_tracing(config.log_format);

    let bind = config.bind.clone();
    let scan_interval_secs = config.scan_interval_secs;
    let prune_interval_secs = config.prune_interval_secs;
    let smtp = config.smtp.clone();

    let pool = db::connect(&config.database_url)
        .await
        .expect("failed to connect to database");
    db::migrate(&pool, &config.database_url)
        .await
        .expect("failed to run migrations");
    let store = Store::new(pool);

    // Built before the background loops so both the scan loop and the HTTP
    // server share the same live-tail event bus (state.events).
    let state = AppState::new(store.clone(), config);

    // Per-check channel binding replaces Plan 1's single global webhook: the scan
    // loop now resolves each check's bound channels via notify::deliver_event.
    tokio::spawn(scheduler::run_scan_loop(
        store.clone(),
        scan_interval_secs,
        smtp,
        state.events.clone(),
    ));
    tokio::spawn(pingward::prune::run_prune_loop(
        store.clone(),
        prune_interval_secs,
    ));

    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
