use pingward::{config::Config, db, scheduler, state::AppState, store::Store};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env();
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

    // Per-check channel binding replaces Plan 1's single global webhook: the scan
    // loop now resolves each check's bound channels via notify::deliver_event.
    tokio::spawn(scheduler::run_scan_loop(
        store.clone(),
        scan_interval_secs,
        smtp,
    ));
    tokio::spawn(pingward::prune::run_prune_loop(
        store.clone(),
        prune_interval_secs,
    ));

    let state = AppState::new(store, config);
    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
