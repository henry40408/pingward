use pingward::{config::Config, db, store::Store};

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

    let listener = tokio::net::TcpListener::bind(&config.bind).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        pingward::app(store).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
