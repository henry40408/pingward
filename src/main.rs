/// mimalloc typically lowers RSS and tail latency for a long-lived,
/// multi-threaded tokio server. Installed on the binary only (not
/// `src/lib.rs`), so the test/bench harness keeps the system allocator.
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
use tracing_subscriber::{
    Layer as _, Registry, filter::Targets, fmt::format::FmtSpan, layer::Filter,
    layer::SubscriberExt, util::SubscriberInitExt,
};

/// How long the drain waits for the pool's connections. Fire-and-forget
/// notification deliveries can still be retrying against a slow endpoint while
/// holding one, and a stuck delivery must not turn a graceful stop into a hang
/// Docker ends with SIGKILL anyway. Well inside its 10s stop grace period.
const POOL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// What is logged when `RUST_LOG` says nothing: pingward's own events at INFO,
/// every dependency at ERROR.
const DEFAULT_FILTER: &str = "error,pingward=info";

/// `RUST_LOG` controls verbosity; `format` selects one of the three
/// human-readable renderers or line-delimited JSON.
fn init_tracing(format: LogFormat) {
    // `Targets` rather than `EnvFilter`: same `RUST_LOG` directives without the
    // regex engine, giving up only span/field filtering, which nothing here
    // writes. An unparseable *level* falls back to the default rather than
    // refusing to start; a mistyped *target* cannot be caught at all (a bare
    // word is a target name at TRACE), so `RUST_LOG=nonsense` parses into a
    // filter nothing matches and the log goes silent. `EnvFilter` behaves
    // identically on both inputs.
    let filter: Targets = std::env::var("RUST_LOG")
        .ok()
        .and_then(|directives| directives.parse().ok())
        .unwrap_or_else(|| DEFAULT_FILTER.parse().expect("the default filter parses"));
    let span_events =
        <Targets as Filter<Registry>>::max_level_hint(&filter).map_or(FmtSpan::CLOSE, |l| {
            if l >= tracing::Level::DEBUG {
                FmtSpan::CLOSE
            } else {
                FmtSpan::NONE
            }
        });
    // Per no-color.org, `NO_COLOR` disables colour only when set and non-empty.
    let use_ansi = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    // `log_internal_errors(true)` is not a default of `fmt::layer()` (it is of
    // the `fmt()` builder); without it a subscriber that fails to write is
    // silent about it.
    let layer = tracing_subscriber::fmt::layer()
        .with_span_events(span_events)
        .with_ansi(use_ansi)
        .log_internal_errors(true);
    let layer = match format {
        LogFormat::Full => layer.with_filter(filter).boxed(),
        LogFormat::Compact => layer.compact().with_filter(filter).boxed(),
        LogFormat::Pretty => layer.pretty().with_filter(filter).boxed(),
        LogFormat::Json => layer.json().with_filter(filter).boxed(),
    };
    tracing_subscriber::registry().with(layer).init();
}

/// Warn when the session/CSRF secret is not configured: the consequence, every
/// browser session ending on restart, is otherwise visible only as an
/// unexplained logout. Called after `init_tracing` to honour the log format.
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

/// Warn when a forward-auth logout URL is configured without forward auth
/// itself: logout still redirects there, but no request is authenticated by a
/// gateway. A warning rather than a refusal, so an operator staging gateway
/// config in two steps can still boot.
fn warn_on_orphan_logout_url(config: &Config) {
    if config.forward_auth_logout_url.is_some() && config.forward_auth_header.is_none() {
        tracing::warn!(
            "PINGWARD_FORWARD_AUTH_LOGOUT_URL is set but PINGWARD_FORWARD_AUTH_HEADER is not; \
             logging out will still redirect there, but no request is authenticated by a \
             gateway header."
        );
    }
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    init_tracing(config.log_format);
    warn_on_ephemeral_secret(config.secret_source);
    warn_on_orphan_logout_url(&config);

    let bind = config.bind.clone();
    let scan_interval_secs = config.scan_interval_secs;
    let prune_interval_secs = config.prune_interval_secs;
    let smtp = config.smtp.clone();
    let base_url = config.base_url.clone();

    let pool = db::connect(&config.database_url)
        .await
        .expect("failed to connect to database");
    db::migrate(&pool, &config.database_url)
        .await
        .expect("failed to run migrations");
    let store = Store::new(pool);

    // Built before the loops so the scan loop and the HTTP server share one
    // live-tail event bus (state.events).
    let state = AppState::new(store.clone(), config);

    // One flag drives all three long-lived tasks, raised by the first
    // SIGTERM/SIGINT. See `shutdown::os_signal` for why the handler is
    // mandatory under Docker.
    let (shutdown_tx, shutdown) = shutdown::channel();
    tokio::spawn(async move {
        shutdown::os_signal().await;
        tracing::info!("shutdown requested; draining");
        shutdown_tx.trigger();
    });

    let scan = tokio::spawn(scheduler::run_scan_loop(
        store.clone(),
        scan_interval_secs,
        smtp,
        base_url,
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
    // Stops accepting new connections and lets in-flight requests finish. An
    // open SSE stream (`web::sse_for_check`) only ends when the client
    // disconnects, so `POOL_CLOSE_TIMEOUT` bounds the drain, not this.
    .with_graceful_shutdown(async move { shutdown.wait().await })
    .await;
    if let Err(e) = served {
        // Logged, not `unwrap`ed: the database still has to close cleanly.
        tracing::error!("http server error: {e}");
    }

    // Both loops hold pool connections: join them before closing the pool, or a
    // scan/prune query races the shutdown and fails with `PoolClosed`. No
    // deadlock — each loop returns on the same flag that ended the server.
    let (scan, prune) = tokio::join!(scan, prune);
    if let Err(e) = scan {
        tracing::error!("scan loop panicked: {e}");
    }
    if let Err(e) = prune {
        tracing::error!("prune loop panicked: {e}");
    }

    // The point of the drain for SQLite: a clean close of the last connection
    // checkpoints the WAL into the main database and removes the `-wal`/`-shm`
    // sidecars, which SIGKILL never does.
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
