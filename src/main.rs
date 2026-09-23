/// Binary only (not `src/lib.rs`), so tests and benches keep the system allocator.
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

/// Bounds the HTTP drain: an open SSE stream (`web::sse_for_check`) never ends
/// on its own, so an unbounded graceful shutdown would wait on its client.
const HTTP_DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// Bounds the pool close: a fire-and-forget delivery may still hold a connection
/// while retrying. With the drain, still inside Docker's 10s stop grace period.
const POOL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Used when `RUST_LOG` is unset or unparseable.
const DEFAULT_FILTER: &str = "error,pingward=info";

fn init_tracing(format: LogFormat) {
    // `Targets` rather than `EnvFilter`: same directives without the regex
    // engine. A bare word parses as a target at TRACE, so a mistyped
    // `RUST_LOG` can silence the log rather than fall back to the default.
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
    // Per no-color.org: only a set, non-empty `NO_COLOR` counts.
    let use_ansi = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    // Not a `fmt::layer()` default; without it a failed write is silent.
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

/// Otherwise the only symptom of an unset secret is an unexplained logout on restart.
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

/// A warning rather than a refusal, so gateway config can be staged in two steps.
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

    // Before the loops, so the scan loop and HTTP server share `state.events`.
    let state = AppState::new(store.clone(), config);

    // One flag stops the server and both loops.
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
    let server = axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown({
        let shutdown = shutdown.clone();
        async move { shutdown.wait().await }
    });
    let served = tokio::select! {
        served = std::future::IntoFuture::into_future(server) => served,
        () = async {
            shutdown.wait().await;
            tokio::time::sleep(HTTP_DRAIN_TIMEOUT).await;
        } => {
            tracing::warn!(
                "http connections still open after {}s; closing them",
                HTTP_DRAIN_TIMEOUT.as_secs()
            );
            Ok(())
        }
    };
    if let Err(e) = served {
        // Not `unwrap`ed: the database still has to close cleanly.
        tracing::error!("http server error: {e}");
    }

    // Join before closing the pool, or a loop query fails with `PoolClosed`.
    let (scan, prune) = tokio::join!(scan, prune);
    if let Err(e) = scan {
        tracing::error!("scan loop panicked: {e}");
    }
    if let Err(e) = prune {
        tracing::error!("prune loop panicked: {e}");
    }

    // For SQLite, a clean close checkpoints the WAL and removes the `-wal`/`-shm`
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
