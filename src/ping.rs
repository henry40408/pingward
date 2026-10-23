use crate::config::Config;
use crate::error::AppError;
use crate::models::{Check, CheckStatus, PingKind};
use crate::notify::{
    DeliveryOptions, DownCause, EventDetail, EventKind, NotificationEvent, RetryPolicy,
    deliver_event,
};
use crate::scheduler::due_time;
use crate::state::AppState;
use crate::store::{NewPing, PingMark, Store};
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, FromRef, FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    routing::get,
};
use chrono::Utc;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;

const MAX_BODY: usize = 10 * 1024;

fn truncate(bytes: &Bytes) -> String {
    let end = bytes.len().min(MAX_BODY);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The request's client address per `auth::client_ip` (the peer, or the client
/// behind a trusted proxy); shared with `web.rs` so pings and sessions use one
/// rule. `None` without `ConnectInfo`, e.g. under `axum-test`.
///
/// A wrapper because `ConnectInfo` does not implement `OptionalFromRequestParts`,
/// so `Option<ConnectInfo<_>>` is not an extractor (axum 0.8).
pub(crate) struct ClientIp(pub(crate) Option<String>);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
    Arc<Config>: FromRef<S>,
{
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let config = Arc::<Config>::from_ref(state);
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip());
        std::future::ready(Ok(Self(crate::auth::client_ip(
            &parts.headers,
            peer,
            &config,
        ))))
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ping/{uuid}", get(success).post(success))
        .route("/ping/{uuid}/fail", get(fail).post(fail))
        .route("/ping/{uuid}/start", get(start).post(start))
        .route("/ping/{uuid}/log", get(log).post(log))
        .route("/ping/{uuid}/{code}", get(exitcode).post(exitcode))
}

async fn resolve(store: &Store, uuid: &str) -> Result<crate::models::Check, AppError> {
    store
        .find_check_by_uuid(uuid)
        .await?
        .ok_or(AppError::NotFound)
}

async fn success(
    State(store): State<Store>,
    State(config): State<Arc<Config>>,
    State(events): State<broadcast::Sender<i64>>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let ping = PingRequest {
        uuid: &uuid,
        kind: PingKind::Success,
        exit_code: None,
        body: &body,
        conn,
    };
    apply(&store, &config, &events, ping).await
}
async fn fail(
    State(store): State<Store>,
    State(config): State<Arc<Config>>,
    State(events): State<broadcast::Sender<i64>>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let ping = PingRequest {
        uuid: &uuid,
        kind: PingKind::Fail,
        exit_code: None,
        body: &body,
        conn,
    };
    apply(&store, &config, &events, ping).await
}
async fn start(
    State(store): State<Store>,
    State(config): State<Arc<Config>>,
    State(events): State<broadcast::Sender<i64>>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let ping = PingRequest {
        uuid: &uuid,
        kind: PingKind::Start,
        exit_code: None,
        body: &body,
        conn,
    };
    apply(&store, &config, &events, ping).await
}
async fn log(
    State(store): State<Store>,
    State(config): State<Arc<Config>>,
    State(events): State<broadcast::Sender<i64>>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let ping = PingRequest {
        uuid: &uuid,
        kind: PingKind::Log,
        exit_code: None,
        body: &body,
        conn,
    };
    apply(&store, &config, &events, ping).await
}
async fn exitcode(
    State(store): State<Store>,
    State(config): State<Arc<Config>>,
    State(events): State<broadcast::Sender<i64>>,
    Path((uuid, code)): Path<(String, i64)>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let kind = if code == 0 {
        PingKind::Success
    } else {
        PingKind::Fail
    };
    let ping = PingRequest {
        uuid: &uuid,
        kind,
        exit_code: Some(code),
        body: &body,
        conn,
    };
    apply(&store, &config, &events, ping).await
}

struct PingRequest<'a> {
    uuid: &'a str,
    kind: PingKind,
    exit_code: Option<i64>,
    body: &'a Bytes,
    conn: ClientIp,
}

async fn apply(
    store: &Store,
    config: &Config,
    events: &broadcast::Sender<i64>,
    ping: PingRequest<'_>,
) -> Result<StatusCode, AppError> {
    let PingRequest {
        uuid,
        kind,
        exit_code,
        body,
        conn,
    } = ping;
    let check = resolve(store, uuid).await?;
    let now = Utc::now();
    let ip = conn.0;
    store
        .insert_ping(
            &NewPing {
                check_id: check.id,
                kind,
                exit_code,
                body: &truncate(body),
                source_ip: ip.as_deref(),
            },
            now,
        )
        .await?;

    // Live-tail signal; free when nobody is watching.
    if events.receiver_count() > 0 {
        let _ = events.send(check.id);
    }

    // A paused check's ping is recorded but must not move it to up/down.
    if check.status == CheckStatus::Paused {
        return Ok(StatusCode::OK);
    }

    let prev_status = check.status;
    match kind {
        PingKind::Success => {
            let mut updated = check.clone();
            updated.last_ping_at = Some(now);
            let next = due_time(&updated);
            store
                .mark_ping(
                    check.id,
                    &PingMark {
                        status: CheckStatus::Up,
                        last_ping_at: Some(now),
                        last_start_at: None,
                        next_due_at: next,
                    },
                )
                .await?;
            if prev_status == CheckStatus::Down {
                store.clear_nag(check.id).await?;
                spawn_delivery(
                    store.clone(),
                    config,
                    Delivery {
                        check: &check,
                        event: EventKind::Up,
                        now,
                        cause: None,
                    },
                );
            }
        }
        PingKind::Fail => {
            store
                .mark_ping(
                    check.id,
                    &PingMark {
                        status: CheckStatus::Down,
                        last_ping_at: Some(now),
                        last_start_at: None,
                        next_due_at: None,
                    },
                )
                .await?;
            if matches!(prev_status, CheckStatus::Up | CheckStatus::New) {
                store.begin_down_alert(check.id, now).await?;
                spawn_delivery(
                    store.clone(),
                    config,
                    Delivery {
                        check: &check,
                        event: EventKind::Down,
                        now,
                        cause: Some(DownCause::Failed { exit_code }),
                    },
                );
            }
        }
        PingKind::Start => {
            store
                .mark_ping(
                    check.id,
                    &PingMark {
                        status: check.status,
                        last_ping_at: None,
                        last_start_at: Some(now),
                        next_due_at: check.next_due_at,
                    },
                )
                .await?;
        }
        PingKind::Log => { /* recorded only */ }
        PingKind::Exitcode => unreachable!("the exitcode handler maps it to Success/Fail"),
    }
    Ok(StatusCode::OK)
}

/// Fire-and-forget, so notification I/O (and the project lookup) stays off the
/// ping response path. `check` must be the pre-ping snapshot: an `Up` message
/// reports the ping *before* the recovery.
struct Delivery<'a> {
    check: &'a Check,
    event: EventKind,
    now: chrono::DateTime<chrono::Utc>,
    cause: Option<DownCause>,
}

fn spawn_delivery(store: Store, config: &Config, delivery: Delivery<'_>) {
    let Delivery {
        check,
        event,
        now,
        cause,
    } = delivery;
    let snapshot = check.clone();
    let base_url = config.base_url.clone();
    let smtp = config.smtp.clone();
    tokio::spawn(async move {
        let project_name = store
            .find_project(snapshot.project_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.name);
        let display_tz = store.display_timezone().await;
        let mut detail = EventDetail::from_check(&snapshot, project_name, &base_url)
            .with_display_timezone(display_tz.as_deref());
        detail.cause = cause;
        let ev = NotificationEvent {
            check_id: snapshot.id,
            check_name: snapshot.name.clone(),
            event,
            at: now,
            project_id: snapshot.project_id,
            detail,
        };
        let options = DeliveryOptions {
            retry: RetryPolicy::default(),
            smtp: smtp.as_ref(),
        };
        deliver_event(&store, &ev, options, now).await;
    });
}
