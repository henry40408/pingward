use crate::api::error::ApiError;
use crate::apikey::hash_api_key;
use crate::models::User;
use crate::state::AppState;
use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use chrono::Utc;
use serde::de::DeserializeOwned;

fn bearer_token(parts: &Parts) -> Option<String> {
    let v = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    v.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

/// An API caller authenticated by an `Authorization: Bearer <api key>` header —
/// never the session cookie. Reading only that header is what makes these
/// routes structurally CSRF-safe, so they mount outside `csrf_guard`.
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
        // A key outlives its owner being disabled, so re-check here — matching
        // the session path in `auth::resolve_user`.
        let user = state
            .store
            .find_user_by_id(user_id)
            .await?
            .filter(|u| !u.disabled)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(ApiUser(user))
    }
}

/// A JSON body extractor that rejects as an [`ApiError`] envelope instead of
/// axum's plain-text default, so a malformed body returns the same
/// `{"error":{code,message}}` shape as every other API failure.
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
