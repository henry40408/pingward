use crate::models::User;
use crate::state::AppState;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::{request::Parts, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use std::net::{IpAddr, SocketAddr};

pub const SESSION_COOKIE: &str = "pingward_session";
pub const SESSION_TTL_DAYS: i64 = 30;

pub fn new_session_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Returns the forward-auth username iff forward-auth is configured, the header
/// is present and valid UTF-8, and `peer_ip` is a configured trusted proxy.
pub fn forward_auth_username(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    config: &crate::config::Config,
) -> Option<String> {
    let header_name = config.forward_auth_header.as_ref()?;
    let peer = peer_ip?;
    if !config
        .trusted_proxies
        .iter()
        .any(|p| p == &peer.to_string())
    {
        return None;
    }
    headers
        .get(header_name.as_str())
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
}

/// Hash a plaintext password into a PHC string (`$argon2id$...`).
pub fn hash_password(plain: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default().hash_password(plain.as_bytes(), &salt)?;
    Ok(phc.to_string())
}

/// Verify a plaintext password against a stored PHC string. A malformed
/// stored hash is treated as a non-match (never panics).
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Resolve the authenticated user from the session cookie, or (failing that)
/// from a trusted forward-auth header — auto-provisioning a non-admin,
/// password-less user for a first-seen forward-auth identity.
async fn resolve_user(parts: &mut Parts, state: &AppState) -> Option<User> {
    let now = Utc::now();
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        if let Ok(Some(user)) = state.store.find_session_user(cookie.value(), now).await {
            return Some(user);
        }
    }
    // forward-auth fallback
    let peer_ip = parts
        .extensions
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    if let Some(username) = forward_auth_username(&parts.headers, peer_ip, &state.config) {
        match state.store.find_user_by_username(&username).await {
            Ok(Some(user)) => return Some(user),
            Ok(None) => {
                if let Ok(id) = state.store.create_user(&username, None, false, now).await {
                    return state.store.find_user_by_id(id).await.ok().flatten();
                }
            }
            Err(_) => {}
        }
    }
    None
}

pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match resolve_user(parts, state).await {
            Some(user) => Ok(CurrentUser(user)),
            None => Err(Redirect::to("/login").into_response()),
        }
    }
}

pub struct AdminUser(pub User);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = Response;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        if user.is_admin {
            Ok(AdminUser(user))
        } else {
            Err((StatusCode::FORBIDDEN, "admin only").into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrips() {
        let phc = hash_password("hunter2").unwrap();
        assert!(phc.starts_with("$argon2"));
        assert!(verify_password("hunter2", &phc));
        assert!(!verify_password("wrong", &phc));
    }

    #[test]
    fn verify_rejects_garbage_hash() {
        assert!(!verify_password("hunter2", "not-a-phc-string"));
    }

    use crate::config::Config;
    use axum::http::{HeaderMap, HeaderValue};
    use std::net::{IpAddr, Ipv4Addr};

    fn cfg_with_forward_auth() -> Config {
        Config::from_map(|k| match k {
            "PINGWARD_FORWARD_AUTH_HEADER" => Some("X-Forwarded-User".into()),
            "PINGWARD_TRUSTED_PROXIES" => Some("10.0.0.1".into()),
            _ => None,
        })
    }

    #[test]
    fn forward_auth_honored_only_from_trusted_proxy() {
        let cfg = cfg_with_forward_auth();
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", HeaderValue::from_static("alice"));
        let trusted = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let untrusted = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        assert_eq!(
            forward_auth_username(&headers, Some(trusted), &cfg),
            Some("alice".into())
        );
        assert_eq!(forward_auth_username(&headers, Some(untrusted), &cfg), None);
        assert_eq!(forward_auth_username(&headers, None, &cfg), None);
    }

    #[test]
    fn forward_auth_disabled_when_unconfigured() {
        let cfg = Config::from_map(|_| None);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", HeaderValue::from_static("alice"));
        let trusted = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(forward_auth_username(&headers, Some(trusted), &cfg), None);
    }

    #[test]
    fn session_token_is_unique_uuid() {
        let a = new_session_token();
        let b = new_session_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36); // hyphenated uuid
    }
}
