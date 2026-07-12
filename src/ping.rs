use crate::error::AppError;
use crate::models::{CheckStatus, PingKind};
use crate::scheduler::due_time;
use crate::store::Store;
use axum::{
    body::Bytes,
    extract::{ConnectInfo, FromRequestParts, Path, State},
    http::{request::Parts, StatusCode},
    routing::get,
    Router,
};
use chrono::Utc;
use std::convert::Infallible;
use std::net::SocketAddr;

const MAX_BODY: usize = 10 * 1024;

fn truncate(bytes: &Bytes) -> String {
    let end = bytes.len().min(MAX_BODY);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Client IP, read directly from the `ConnectInfo<SocketAddr>` extension if
/// present.
///
/// The brief specifies `Option<ConnectInfo<SocketAddr>>` as the extractor so
/// the handler still works under `axum-test` (which never populates
/// `ConnectInfo`). As of axum 0.8.9, `Option<T>` only implements
/// `FromRequestParts` for extractors that explicitly opt in via the
/// `OptionalFromRequestParts` trait, and `ConnectInfo` does not — so
/// `Option<ConnectInfo<SocketAddr>>` does not implement `FromRequestParts` and
/// the handlers fail to compile (confirmed with a minimal repro against the
/// pinned axum 0.8.9). This local wrapper reads the extension manually and
/// is infallible, preserving the brief's "optional connect info" behavior.
struct ClientIp(Option<SocketAddr>);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0),
        ))
    }
}

pub fn routes() -> Router<Store> {
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
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Success, None, &body, conn).await
}
async fn fail(
    State(store): State<Store>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Fail, None, &body, conn).await
}
async fn start(
    State(store): State<Store>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Start, None, &body, conn).await
}
async fn log(
    State(store): State<Store>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Log, None, &body, conn).await
}
async fn exitcode(
    State(store): State<Store>,
    Path((uuid, code)): Path<(String, i64)>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let kind = if code == 0 {
        PingKind::Success
    } else {
        PingKind::Fail
    };
    apply(&store, &uuid, kind, Some(code), &body, conn).await
}

async fn apply(
    store: &Store,
    uuid: &str,
    kind: PingKind,
    exit_code: Option<i64>,
    body: &Bytes,
    conn: ClientIp,
) -> Result<StatusCode, AppError> {
    let check = resolve(store, uuid).await?;
    let now = Utc::now();
    let ip = conn.0.map(|addr| addr.ip().to_string());
    store
        .insert_ping(
            check.id,
            kind,
            exit_code,
            &truncate(body),
            ip.as_deref(),
            now,
        )
        .await?;

    match kind {
        PingKind::Success => {
            let mut updated = check.clone();
            updated.last_ping_at = Some(now);
            let next = due_time(&updated);
            store
                .mark_ping(check.id, CheckStatus::Up, Some(now), None, next)
                .await?;
        }
        PingKind::Fail => {
            store
                .mark_ping(check.id, CheckStatus::Down, Some(now), None, None)
                .await?;
        }
        PingKind::Start => {
            store
                .mark_ping(check.id, check.status, None, Some(now), check.next_due_at)
                .await?;
        }
        PingKind::Log => { /* recorded only */ }
        PingKind::Exitcode => unreachable!("exitcode maps to Success/Fail above"),
    }
    Ok(StatusCode::OK)
}
