use crate::models::User;
use crate::state::AppState;
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, StatusCode, request::Parts};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Duration, Utc};
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

pub const SESSION_COOKIE_BASE: &str = "pingward_session";
/// Used when `Secure` is on: `__Host-` makes the browser enforce Secure, Path=/ and no
/// Domain, so a sibling subdomain or a downgraded response cannot overwrite the cookie.
pub const SESSION_COOKIE_HOST_PREFIXED: &str = "__Host-pingward_session";

/// Must stay conditional on `cookie_secure`: browsers refuse a `__Host-` cookie over
/// plaintext HTTP, which would make login fail silently.
pub fn session_cookie_name(cookie_secure: bool) -> &'static str {
    if cookie_secure {
        SESSION_COOKIE_HOST_PREFIXED
    } else {
        SESSION_COOKIE_BASE
    }
}

/// Idle window: `sessions.expires_at` is "last activity + this" (renewed only past the
/// half-life, see [`refreshed_expiry`]). Longer than OWASP's 15–30 min so a dashboard
/// tab left open is not logged out constantly; the absolute cap bounds it.
pub const SESSION_IDLE_TTL_HOURS: i64 = 72;

/// Absolute cap from `created_at`; no amount of activity extends it.
pub const SESSION_ABSOLUTE_MAX_DAYS: i64 = 30;

/// A `None` `created_at` (a pre-`0010` row, since deleted by `0012`/`0015`) is never
/// past the cap; the branch is defensive.
pub fn is_past_absolute_cap(created_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    created_at.is_some_and(|c| now >= c + Duration::days(SESSION_ABSOLUTE_MAX_DAYS))
}

/// Which way a renewal moved `expires_at`, logged as `renewal` on `session.renewed`.
/// A burst of clamps is a deployment signal (older build or longer idle TTL), not
/// user activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewalKind {
    /// Moved forward (possibly truncated at the absolute cap): ordinary activity.
    Slid,
    /// Moved backwards: the stored `expires_at` exceeded current policy.
    Clamped,
}

impl RenewalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slid => "slid",
            Self::Clamped => "clamped",
        }
    }
}

/// A renewal decision: the `expires_at` to write, and which way it moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionRenewal {
    pub expires_at: DateTime<Utc>,
    pub kind: RenewalKind,
}

impl SessionRenewal {
    fn slid(expires_at: DateTime<Utc>) -> Self {
        Self {
            expires_at,
            kind: RenewalKind::Slid,
        }
    }

    fn clamped(expires_at: DateTime<Utc>) -> Self {
        Self {
            expires_at,
            kind: RenewalKind::Clamped,
        }
    }
}

/// The new `expires_at`, or `None` when no write is needed. The target is
/// `min(now + idle, created_at + absolute)`. `None` if the stored expiry already
/// reaches the absolute cap, or more than half the idle window remains (the write
/// throttle). A stored expiry beyond the target (older build, second instance, or a
/// lowered `SESSION_IDLE_TTL_HOURS`) is clamped down at once, bypassing the throttle.
pub fn refreshed_expiry(
    created_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<SessionRenewal> {
    let idle = Duration::hours(SESSION_IDLE_TTL_HOURS);
    let cap = created_at.map(|c| c + Duration::days(SESSION_ABSOLUTE_MAX_DAYS));
    if cap.is_some_and(|cap| expires_at >= cap) {
        return None;
    }
    let next = cap.map_or(now + idle, |cap| (now + idle).min(cap));
    if expires_at > next {
        return Some(SessionRenewal::clamped(next));
    }
    if expires_at - now >= idle / 2 {
        return None;
    }
    Some(SessionRenewal::slid(next))
}

pub fn new_session_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A session's log identifier: the `/account` SHA-256 handle cut to 16 hex chars. Never
/// log the id itself — it is the bearer secret. Unkeyed is fine: a UUID v4's 122 bits
/// are beyond brute force, and keying would break the log ↔ `/account` correspondence.
pub fn session_log_handle(session_id: &str) -> String {
    crate::apikey::hash_api_key(session_id)[..16].to_string()
}

/// Longest username kept in a log line by [`log_username`].
const LOG_USERNAME_MAX_CHARS: usize = 64;

/// Truncate an attacker-chosen username for logging. Callers MUST render it with `Debug`
/// (`username = ?…`) so an embedded newline cannot forge a log entry.
pub fn log_username(raw: &str) -> String {
    let mut out: String = raw.chars().take(LOG_USERNAME_MAX_CHARS).collect();
    if out.chars().count() < raw.chars().count() {
        out.push('…');
    }
    out
}

/// True when `ip` matches a trusted-proxy pattern: a bare address or a CIDR block (a
/// Docker bridge proxy's address comes from a pool, so a literal breaks on recreate).
/// Compared canonically, so an IPv4-mapped peer matches a v4 pattern. Unparseable
/// patterns match nothing; DNS is never consulted, or a resolver would decide trust.
pub fn is_trusted_proxy(patterns: &[String], ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    patterns.iter().any(|p| proxy_pattern_matches(p, ip))
}

fn proxy_pattern_matches(pattern: &str, ip: IpAddr) -> bool {
    let pattern = pattern.trim();
    let Some((net, prefix)) = pattern.split_once('/') else {
        return pattern
            .parse::<IpAddr>()
            .is_ok_and(|p| p.to_canonical() == ip);
    };
    let (Ok(net), Ok(prefix)) = (net.trim().parse::<IpAddr>(), prefix.trim().parse::<u8>()) else {
        return false;
    };
    match (net, ip) {
        (IpAddr::V4(net), IpAddr::V4(ip)) => prefix_eq(&net.octets(), &ip.octets(), prefix, 32),
        (IpAddr::V6(net), IpAddr::V6(ip)) => prefix_eq(&net.octets(), &ip.octets(), prefix, 128),
        _ => false,
    }
}

fn prefix_eq(a: &[u8], b: &[u8], prefix: u8, max: u8) -> bool {
    if prefix > max {
        return false;
    }
    let whole = usize::from(prefix / 8);
    if a[..whole] != b[..whole] {
        return false;
    }
    let rest = prefix % 8;
    // Short-circuit before indexing: for /32 or /128 `whole` is one past the end.
    rest == 0 || {
        let mask = 0xffu8 << (8 - rest);
        (a[whole] & mask) == (b[whole] & mask)
    }
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
    if !is_trusted_proxy(&config.trusted_proxies, peer) {
        return None;
    }
    headers
        .get(header_name.as_str())
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
}

/// The client IP to record against a session or ping: the first `X-Forwarded-For` entry
/// when the peer is a trusted proxy (anyone else could spoof it), otherwise — or if the
/// header is unparseable — the peer.
pub fn client_ip(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    config: &crate::config::Config,
) -> Option<String> {
    // Store a v4 client on a dual-stack listener as `a.b.c.d`, not `::ffff:a.b.c.d`.
    let peer = peer_ip?.to_canonical();
    if !is_trusted_proxy(&config.trusted_proxies, peer) {
        return Some(peer.to_string());
    }
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<IpAddr>().ok());
    Some(forwarded.map_or_else(|| peer.to_string(), |ip| ip.to_canonical().to_string()))
}

/// Minimum length in characters (not bytes). NIST SP800-63B wants 15 without MFA, 8
/// with; pingward has no second factor, so revisit this if TOTP is added.
pub const MIN_PASSWORD_CHARS: usize = 15;

/// Maximum length in characters (OWASP: at least 64). A sanity bound on an
/// unauthenticated field; over-long is rejected, never truncated.
pub const MAX_PASSWORD_CHARS: usize = 128;

/// The single password policy, returning the user-facing message on failure. Length is
/// the only rule: no composition rules, no trimming. Every surface that sets a password
/// calls this, so a breached-password check (deliberately absent) would go here.
pub fn validate_password(plain: &str) -> Result<(), String> {
    let len = plain.chars().count();
    if len < MIN_PASSWORD_CHARS {
        return Err(format!(
            "Password must be at least {MIN_PASSWORD_CHARS} characters."
        ));
    }
    if len > MAX_PASSWORD_CHARS {
        return Err(format!(
            "Password must be at most {MAX_PASSWORD_CHARS} characters."
        ));
    }
    Ok(())
}

/// Hash a plaintext password into a PHC string (`$argon2id$...`).
pub fn hash_password(plain: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default().hash_password(plain.as_bytes(), &salt)?;
    Ok(phc.to_string())
}

/// A PHC string of a random secret, built once per process, for
/// [`verify_password_or_dummy`] to verify against.
fn dummy_password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password(&uuid::Uuid::new_v4().to_string())
            .expect("hashing a fixed-length uuid cannot fail")
    })
}

/// Verify `plain`, spending one argon2 verification even when `stored` is `None` (no
/// such user, or a passwordless forward-auth account), so response time does not
/// reveal which usernames exist.
pub fn verify_password_or_dummy(plain: &str, stored: Option<&str>) -> bool {
    if let Some(phc) = stored {
        verify_password(plain, phc)
    } else {
        // `black_box` so the call is not elided: the work is the point.
        std::hint::black_box(verify_password(plain, dummy_password_hash()));
        false
    }
}

/// A malformed stored hash is a non-match, never a panic.
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// The user from the session cookie, else from a trusted forward-auth header.
async fn resolve_user(parts: &mut Parts, state: &AppState) -> Option<User> {
    let now = Utc::now();
    let jar = CookieJar::from_headers(&parts.headers);
    // A bad signature is rejected before any database work.
    let cookie_name = session_cookie_name(state.config.cookie_secure);
    if let Some(session_id) =
        crate::secret::session_id_from_jar(&jar, &state.config.secret, cookie_name)
        && let Ok(Some(user)) = state.store.find_session_user(&session_id, now).await
        && !user.disabled
    {
        return Some(user);
    }
    let peer_ip = peer_ip(&parts.extensions);
    forward_auth_user(state, &parts.headers, peer_ip, now).await
}

/// The socket peer from `ConnectInfo`; `None` without it, so trusted-proxy checks fail
/// closed.
pub fn peer_ip(extensions: &axum::http::Extensions) -> Option<IpAddr> {
    extensions
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
}

/// The user named by a trusted forward-auth header, auto-provisioning a non-admin,
/// passwordless account on first sight; `None` if disabled or not applicable. Shared by
/// `resolve_user` and `web::forward_auth_session` so both agree on who is signed in.
pub async fn forward_auth_user(
    state: &AppState,
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    now: chrono::DateTime<Utc>,
) -> Option<User> {
    let username = forward_auth_username(headers, peer_ip, &state.config)?;
    match state.store.find_user_by_username(&username).await {
        Ok(Some(user)) => (!user.disabled).then_some(user),
        Ok(None) => {
            let id = state
                .store
                .create_user(&username, None, false, now)
                .await
                .ok()?;
            state.store.find_user_by_id(id).await.ok().flatten()
        }
        Err(_) => None,
    }
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

/// Like `CurrentUser`, but yields `None` instead of redirecting to `/login`.
pub struct OptionalUser(pub Option<User>);

impl FromRequestParts<AppState> for OptionalUser {
    type Rejection = Response;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(OptionalUser(resolve_user(parts, state).await))
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

    #[test]
    fn password_policy_enforces_both_bounds() {
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_CHARS)).is_ok());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_CHARS)).is_ok());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_CHARS - 1)).is_err());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_CHARS + 1)).is_err());
        assert!(validate_password("").is_err());
        // Tripwire: revisit these if MFA is ever added.
        const { assert!(MIN_PASSWORD_CHARS == 15) };
        const { assert!(MAX_PASSWORD_CHARS >= 64) };
    }

    #[test]
    fn password_policy_has_no_composition_rules() {
        for pw in [
            "correcthorsebatterystaple",
            "123456789012345",
            "correct horse battery staple",
            "正確的馬電池釘書針這樣就夠長了吧",
            "\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
        ] {
            assert!(validate_password(pw).is_ok(), "{pw:?} must be accepted");
        }
    }

    #[test]
    fn password_length_is_counted_in_characters_not_bytes() {
        // 14 characters but over 15 bytes: counting bytes would accept it.
        let cjk = "密碼密碼密碼密碼密碼密碼密碼";
        assert_eq!(cjk.chars().count(), MIN_PASSWORD_CHARS - 1);
        assert!(cjk.len() > MIN_PASSWORD_CHARS);
        assert!(validate_password(cjk).is_err());
    }

    /// Timing is too noisy to assert on; this pins the observable contract.
    #[test]
    fn verify_password_or_dummy_handles_a_missing_hash() {
        let phc = hash_password("correct horse battery").unwrap();
        assert!(verify_password_or_dummy(
            "correct horse battery",
            Some(&phc)
        ));
        assert!(!verify_password_or_dummy("wrong", Some(&phc)));
        assert!(!verify_password_or_dummy("anything at all", None));

        let dummy = dummy_password_hash();
        assert!(dummy.starts_with("$argon2"));
        assert!(!verify_password("", dummy));
        // Built once per process, not per miss.
        assert_eq!(dummy, dummy_password_hash());
    }

    #[test]
    fn log_username_truncates_and_stays_debug_escapable() {
        assert_eq!(log_username("alice"), "alice");
        let long = "a".repeat(LOG_USERNAME_MAX_CHARS * 2);
        let cut = log_username(&long);
        assert_eq!(cut.chars().count(), LOG_USERNAME_MAX_CHARS + 1);
        assert!(cut.ends_with('…'));
        let cjk = "漢".repeat(LOG_USERNAME_MAX_CHARS * 2);
        assert_eq!(
            log_username(&cjk).chars().count(),
            LOG_USERNAME_MAX_CHARS + 1
        );
        // The newline survives; the caller's `Debug` rendering neutralises it.
        let forged = "bob\nsession.created user_id=1";
        assert_eq!(log_username(forged), forged);
        assert!(!format!("{:?}", log_username(forged)).contains('\n'));
    }

    #[test]
    fn session_cookie_name_is_prefixed_only_when_secure() {
        assert_eq!(session_cookie_name(true), SESSION_COOKIE_HOST_PREFIXED);
        assert_eq!(session_cookie_name(false), SESSION_COOKIE_BASE);
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
    fn client_ip_prefers_forwarded_for_only_from_a_trusted_proxy() {
        let cfg = cfg_with_forward_auth(); // trusts 10.0.0.1
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        let proxy = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let stranger = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        assert_eq!(
            client_ip(&headers, Some(proxy), &cfg).as_deref(),
            Some("203.0.113.7")
        );
        // An untrusted peer cannot spoof its own address away.
        assert_eq!(
            client_ip(&headers, Some(stranger), &cfg).as_deref(),
            Some("8.8.8.8")
        );
        assert_eq!(client_ip(&headers, None, &cfg), None);
    }

    #[test]
    fn client_ip_takes_the_first_forwarded_entry_and_ignores_junk() {
        let cfg = cfg_with_forward_auth();
        let proxy = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let with = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert("x-forwarded-for", HeaderValue::from_static(v));
            client_ip(&h, Some(proxy), &cfg).unwrap()
        };
        assert_eq!(with("203.0.113.7, 10.0.0.1"), "203.0.113.7");
        assert_eq!(with("  203.0.113.7  "), "203.0.113.7");
        // A trusted proxy sending nonsense falls back to the peer, never junk.
        assert_eq!(with("not-an-ip"), "10.0.0.1");
        assert_eq!(with(""), "10.0.0.1");
        assert_eq!(
            client_ip(&HeaderMap::new(), Some(proxy), &cfg).as_deref(),
            Some("10.0.0.1")
        );
    }

    #[test]
    fn client_ip_without_trusted_proxies_always_uses_the_peer() {
        let cfg = Config::from_map(|_| None);
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(
            client_ip(&headers, Some(peer), &cfg).as_deref(),
            Some("10.0.0.1")
        );
    }

    fn cfg_trusting(patterns: &str) -> Config {
        let owned = patterns.to_string();
        Config::from_map(move |k| match k {
            "PINGWARD_TRUSTED_PROXIES" => Some(owned.clone()),
            _ => None,
        })
    }

    #[test]
    fn trusted_proxy_accepts_a_cidr_block() {
        let nets = vec!["172.16.0.0/12".to_string()];
        assert!(is_trusted_proxy(&nets, "172.18.0.5".parse().unwrap()));
        assert!(is_trusted_proxy(&nets, "172.31.255.255".parse().unwrap()));
        // Just outside the block on either side.
        assert!(!is_trusted_proxy(&nets, "172.15.255.255".parse().unwrap()));
        assert!(!is_trusted_proxy(&nets, "172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn trusted_proxy_handles_prefix_edges_and_v6() {
        let all = vec!["0.0.0.0/0".to_string()];
        assert!(is_trusted_proxy(&all, "8.8.8.8".parse().unwrap()));
        // /32 and /128 must not index one past the address.
        let single = vec!["10.0.0.1/32".to_string()];
        assert!(is_trusted_proxy(&single, "10.0.0.1".parse().unwrap()));
        assert!(!is_trusted_proxy(&single, "10.0.0.2".parse().unwrap()));
        let v6 = vec!["fd00::/8".to_string(), "::1/128".to_string()];
        assert!(is_trusted_proxy(&v6, "fd00:1234::9".parse().unwrap()));
        assert!(is_trusted_proxy(&v6, "::1".parse().unwrap()));
        assert!(!is_trusted_proxy(&v6, "2001:db8::1".parse().unwrap()));
        // Families never cross: a v4 peer is not inside a v6 block.
        assert!(!is_trusted_proxy(&v6, "10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn trusted_proxy_rejects_unparseable_patterns() {
        for junk in ["caddy", "10.0.0.1/", "10.0.0.1/33", "10.0.0.0/x", ""] {
            assert!(
                !is_trusted_proxy(&[junk.to_string()], "10.0.0.1".parse().unwrap()),
                "{junk} must not be trusted"
            );
        }
    }

    #[test]
    fn client_ip_matches_a_v4_mapped_peer_against_a_v4_pattern() {
        let cfg = cfg_trusting("172.18.0.0/16");
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        let proxy: IpAddr = "::ffff:172.18.0.5".parse().unwrap();
        assert!(is_trusted_proxy(&cfg.trusted_proxies, proxy));
        assert_eq!(
            client_ip(&headers, Some(proxy), &cfg).as_deref(),
            Some("203.0.113.7")
        );
        // And an untrusted mapped peer is stored in canonical v4 form.
        let stranger: IpAddr = "::ffff:8.8.8.8".parse().unwrap();
        assert_eq!(
            client_ip(&headers, Some(stranger), &cfg).as_deref(),
            Some("8.8.8.8")
        );
    }

    #[test]
    fn session_token_is_unique_uuid() {
        let a = new_session_token();
        let b = new_session_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36); // hyphenated uuid
    }

    #[test]
    fn session_log_handle_is_never_the_raw_id() {
        let id = new_session_token();
        let handle = session_log_handle(&id);
        assert_ne!(handle, id);
        assert_eq!(handle.len(), 16);
        assert!(!handle.contains(&id));
    }

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn refreshed_expiry_slides_only_past_the_half_life() {
        let now = ts(2026, 1, 1);
        let created = Some(now - Duration::days(1));
        let idle = Duration::hours(SESSION_IDLE_TTL_HOURS);

        // More than half the idle window remains: no write.
        let fresh_expiry = now + idle - Duration::hours(1);
        assert_eq!(refreshed_expiry(created, fresh_expiry, now), None);

        // Less than half remains: slides to now + idle.
        let stale_expiry = now + idle / 2 - Duration::hours(1);
        assert_eq!(
            refreshed_expiry(created, stale_expiry, now),
            Some(SessionRenewal::slid(now + idle))
        );
    }

    #[test]
    fn refreshed_expiry_clamps_to_the_absolute_cap() {
        let created = ts(2026, 1, 1);
        // now is close to the cap, so created + 30d < now + idle.
        let now = created + Duration::days(29) + Duration::hours(23);
        let cap = created + Duration::days(SESSION_ABSOLUTE_MAX_DAYS);
        let stale_expiry = now; // definitely under half the idle window
        let result = refreshed_expiry(Some(created), stale_expiry, now).unwrap();
        assert_eq!(result.expires_at, cap);
        assert!(result.expires_at <= cap);
        // Truncated but still forward: `Clamped` is only for a shrinking window.
        assert_eq!(result.kind, RenewalKind::Slid);
    }

    #[test]
    fn refreshed_expiry_stops_at_the_cap() {
        let created = ts(2026, 1, 1);
        let cap = created + Duration::days(SESSION_ABSOLUTE_MAX_DAYS);
        let now = cap; // already at the cap
        assert_eq!(refreshed_expiry(Some(created), cap, now), None);
        // Past the cap too.
        assert_eq!(
            refreshed_expiry(Some(created), cap + Duration::hours(1), now),
            None
        );
    }

    #[test]
    fn refreshed_expiry_without_created_at_has_no_cap() {
        let now = ts(2026, 1, 1);
        let idle = Duration::hours(SESSION_IDLE_TTL_HOURS);
        let stale_expiry = now + Duration::hours(1);
        assert_eq!(
            refreshed_expiry(None, stale_expiry, now),
            Some(SessionRenewal::slid(now + idle))
        );
    }

    #[test]
    fn refreshed_expiry_shortens_a_legacy_row_immediately() {
        // A near-30-day expiry would escape the half-life throttle for weeks.
        let now = ts(2026, 1, 1);
        let created = now - Duration::days(1);
        let idle = Duration::hours(SESSION_IDLE_TTL_HOURS);
        let cap = created + Duration::days(SESSION_ABSOLUTE_MAX_DAYS);
        let legacy_expiry = cap - Duration::seconds(1);
        assert_eq!(
            refreshed_expiry(Some(created), legacy_expiry, now),
            Some(SessionRenewal::clamped(now + idle))
        );
    }

    #[test]
    fn refreshed_expiry_does_not_shorten_a_freshly_created_row() {
        // Exactly `now + idle` is not "more than policy grants": no clamp, no write.
        let now = ts(2026, 1, 1);
        let created = Some(now);
        let idle = Duration::hours(SESSION_IDLE_TTL_HOURS);
        assert_eq!(refreshed_expiry(created, now + idle, now), None);
    }

    #[test]
    fn is_past_absolute_cap_boundary() {
        let created = ts(2026, 1, 1);
        let cap = created + Duration::days(SESSION_ABSOLUTE_MAX_DAYS);
        assert!(is_past_absolute_cap(Some(created), cap));
        assert!(!is_past_absolute_cap(
            Some(created),
            cap - Duration::seconds(1)
        ));
        assert!(!is_past_absolute_cap(None, cap + Duration::days(365)));
    }
}
