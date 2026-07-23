/// mimalloc as the global allocator. pingward is a long-lived, multi-threaded
/// tokio server (HTTP handlers plus two background loops), the allocation
/// pattern the system allocator handles worst; mimalloc typically lowers RSS
/// and tail latency for that shape. Installed on the binary only (not
/// `src/lib.rs`), so the test/bench harness keeps the system allocator unless a
/// target opts in.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use pingward::{
    config::{Config, LogFormat},
    db, scheduler,
    secret::SecretSource,
    shutdown,
    state::AppState,
    store::Store,
};
use std::time::Duration;

/// How long the drain waits for the pool's connections to come back before
/// giving up. Fire-and-forget notification deliveries (`tokio::spawn` in
/// `ping::apply` / `scheduler::run_scan_loop`) can still be retrying against a
/// slow endpoint and holding a connection; bounding the wait keeps a stuck
/// delivery from turning a graceful stop into a hang that Docker resolves with
/// SIGKILL anyway. Well inside Docker's default 10s stop grace period.
const POOL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Warn once at startup when the session/CSRF secret is not configured, since
/// the consequence — every browser session ending on restart — is otherwise
/// only visible as an unexplained logout. Emitted after `init_tracing` so it
/// honours `PINGWARD_LOG_FORMAT`.
fn warn_on_ephemeral_secret(source: SecretSource) {
    let cause = match source {
        SecretSource::Env => return,
        SecretSource::Generated => "PINGWARD_SECRET is not set",
        SecretSource::Rejected => {
            "PINGWARD_SECRET is shorter than the 16-byte minimum and was ignored"
        }
    };
    tracing::warn!(
        "{cause}; using a secret generated for this process only. Every signed-in \
         browser session will end on restart. Set PINGWARD_SECRET (e.g. `openssl rand -hex 32`) \
         to keep sessions across restarts. API keys are unaffected."
    );
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    init_tracing(config.log_format);
    warn_on_ephemeral_secret(config.secret_source);

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

    // One shutdown flag drives all three long-lived tasks (server + both
    // loops). Raised by the first SIGTERM/SIGINT; see `shutdown::os_signal` for
    // why the handler is mandatory rather than a nicety under Docker.
    let (shutdown_tx, shutdown) = shutdown::channel();
    tokio::spawn(async move {
        shutdown::os_signal().await;
        tracing::info!("shutdown requested; draining");
        shutdown_tx.trigger();
    });

    // Per-check channel binding replaces Plan 1's single global webhook: the scan
    // loop now resolves each check's bound channels via notify::deliver_event.
    let scan = tokio::spawn(scheduler::run_scan_loop(
        store.clone(),
        scan_interval_secs,
        smtp,
        state.events.clone(),
        shutdown.clone(),
    ));
    let prune = tokio::spawn(pingward::prune::run_prune_loop(
        store.clone(),
        prune_interval_secs,
        shutdown.clone(),
    ));

    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    let served = axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    // Stops accepting new connections and lets in-flight requests finish. Note
    // that an open SSE stream (`web::sse_for_check`) only ends when the client
    // disconnects, so `POOL_CLOSE_TIMEOUT` — not this — bounds the drain.
    .with_graceful_shutdown(async move { shutdown.wait().await })
    .await;
    if let Err(e) = served {
        // Logged, not `unwrap`ed: the database still has to be closed cleanly.
        tracing::error!("http server error: {e}");
    }

    // Both loops hold pool connections, so join them before closing the pool —
    // otherwise a scan/prune query races the shutdown and fails with
    // `PoolClosed`. `join!` cannot deadlock here: each loop returns on the same
    // flag that already ended the server.
    let (scan, prune) = tokio::join!(scan, prune);
    if let Err(e) = scan {
        tracing::error!("scan loop panicked: {e}");
    }
    if let Err(e) = prune {
        tracing::error!("prune loop panicked: {e}");
    }

    // Closing the pool is the point of the whole drain for SQLite: a clean
    // close of the *last* connection checkpoints the WAL into the main database
    // and removes the `-wal`/`-shm` sidecars. Under SIGKILL that never happens,
    // so every start had to replay the WAL instead.
    if tokio::time::timeout(POOL_CLOSE_TIMEOUT, store.pool.close())
        .await
        .is_ok()
    {
        tracing::info!("database pool closed");
    } else {
        tracing::warn!(
            "database pool did not close within {}s; exiting anyway",
            POOL_CLOSE_TIMEOUT.as_secs()
        );
    }
}
