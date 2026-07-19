use crate::api::error::ApiError;
use crate::apikey::hash_api_key;
use crate::models::User;
use crate::state::AppState;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::Json;
use chrono::Utc;
use serde::de::DeserializeOwned;

/// Extract a bearer token from the `Authorization` header, if present and
/// well-formed (`Authorization: Bearer <token>`).
fn bearer_token(parts: &Parts) -> Option<String> {
    let v = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    v.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

/// An API caller authenticated by an `Authorization: Bearer <api key>` header —
/// never the session cookie. Because this extractor only ever reads the bearer
/// header, routes that depend on it are structurally CSRF-safe and are mounted
/// as a sibling router outside the `csrf_guard` middleware (like `ping::routes`).
///
/// The wrapped [`User`] carries `is_admin`, so downstream resolution can allow
/// an admin key to reach cross-user resources through an audited choke point.
pub struct ApiUser(pub User);

impl FromRequestParts<AppState> for ApiUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(ApiError::unauthorized)?;
        let hash = hash_api_key(&token);
        let user_id = state
            .store
            .validate_api_key(&hash, Utc::now())
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        // A key can outlive its owner's account being disabled; re-check here so
        // a disabled user's keys stop working immediately, matching the session
        // path in `auth::resolve_user`.
        let user = state
            .store
            .find_user_by_id(user_id)
            .await?
            .filter(|u| !u.disabled)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(ApiUser(user))
    }
}

/// A JSON body extractor that fails as an [`ApiError`] envelope instead of
/// axum's default plain-text rejection, so malformed or schema-invalid request
/// bodies return the same `{"error":{code,message}}` shape as every other API
/// failure (`400 bad_request`).
pub struct ApiJson<T>(pub T);

impl<T> FromRequest<AppState> for ApiJson<T>
where
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|e| ApiError::bad_request(e.body_text()))?;
        Ok(ApiJson(value))
    }
}
