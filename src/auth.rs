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

pub const SESSION_COOKIE: &str = "pingward_session";

/// Idle window: `sessions.expires_at` is always "last activity + this".
///
/// OWASP's suggested 15–30 minutes targets high-value applications; pingward
/// is a self-hosted monitoring dashboard whose users routinely leave the
/// board open in a tab for days, so that figure would produce a stream of
/// spurious logouts. The requirement is that *both* an idle and an absolute
/// layer exist; 72 hours is the value chosen for this one.
pub const SESSION_IDLE_TTL_HOURS: i64 = 72;

/// Absolute cap, measured from `created_at`. No amount of activity extends it.
/// Kept at the previous 30 days so that no existing session's maximum lifetime
/// is shortened by the upgrade.
pub const SESSION_ABSOLUTE_MAX_DAYS: i64 = 30;

/// Whether a session has passed its absolute cap.
///
/// A `None` `created_at` (a pre-`0010` row, whose `created_at = ''` yields
/// `None` from `parse_ts`) is treated as *not* past the cap — only the idle
/// window governs it. `0012_session_secret.sql` already ran `DELETE FROM
/// sessions`, so no such row can exist in a migrated database; this branch is
/// defensive.
pub fn is_past_absolute_cap(created_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    created_at.is_some_and(|c| now >= c + Duration::days(SESSION_ABSOLUTE_MAX_DAYS))
}

/// The new `expires_at` when the session should slide, else `None`.
///
/// Already at/past the absolute cap → `None`; more than half the idle window
/// still remaining → `None` (this is the write throttle); otherwise
/// `min(now + idle, created_at + absolute)`.
pub fn refreshed_expiry(
    created_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let idle = Duration::hours(SESSION_IDLE_TTL_HOURS);
    let cap = created_at.map(|c| c + Duration::days(SESSION_ABSOLUTE_MAX_DAYS));
    if cap.is_some_and(|cap| expires_at >= cap) {
        return None;
    }
    if expires_at - now >= idle / 2 {
        return None;
    }
    let next = now + idle;
    Some(cap.map_or(next, |cap| next.min(cap)))
}

pub fn new_session_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The identifier used for a session in log events: the same SHA-256 handle
/// `/account` uses to identify a row, truncated to 16 hex characters to keep
/// log volume down. **Never log the session id itself** — it is the bearer
/// secret the cookie signature is attached to.
///
/// Reusing that handle (rather than a second, separate hash) means a log line
/// maps directly onto the row a user can see and revoke on /account.
///
/// To satisfy OWASP's literal "salted hash" wording more strictly, this could
/// become a keyed digest under the process secret (a `LOG_DOMAIN = b"log:"`
/// alongside the existing domains in src/secret.rs). UUID v4's 122 bits of
/// entropy already put an unsalted SHA-256 beyond brute force, so the existing
/// handle is used instead; switching to a keyed version would break the
/// log ↔ /account correspondence, which is the deliberate trade-off here.
pub fn session_log_handle(session_id: &str) -> String {
    crate::apikey::hash_api_key(session_id)[..16].to_string()
}

/// True when `ip` is covered by one of the configured trusted-proxy patterns.
///
/// A pattern is either a bare address (`10.0.0.1`) or a CIDR block
/// (`172.16.0.0/12`, `fd00::/8`). CIDR is what a container deployment needs:
/// behind a reverse proxy on a Docker bridge network, the peer address is
/// handed out from the network's pool and changes whenever the network is
/// recreated, so pinning a single literal address silently stops matching.
///
/// Both sides are compared in canonical form, so an IPv4-mapped IPv6 peer
/// (`::ffff:172.18.0.5` — how a dual-stack listener reports an IPv4 client)
/// matches an IPv4 pattern. A pattern that does not parse — a hostname, say —
/// matches nothing; DNS is never consulted, because a name the operator does
/// not control would let its resolver decide who is trusted.
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

/// Compare the leading `prefix` bits of two same-length address byte strings.
fn prefix_eq(a: &[u8], b: &[u8], prefix: u8, max: u8) -> bool {
    if prefix > max {
        return false;
    }
    let whole = usize::from(prefix / 8);
    if a[..whole] != b[..whole] {
        return false;
    }
    let rest = prefix % 8;
    // Short-circuits before indexing: when `rest` is 0, `whole` may be one past
    // the last byte (a /32 or /128).
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

/// Resolve the client IP to record against a session or a ping.
///
/// The socket peer is the answer only when pingward is reached directly. The
/// expected deployment is behind a reverse proxy, where every peer is the
/// proxy — recording that would stamp every session and every ping with the
/// same address and make the column useless for spotting a session you did not
/// start or a host you did not expect a ping from. So when the peer is a
/// configured trusted proxy, the first `X-Forwarded-For` entry (the original
/// client) wins instead.
///
/// The trust check is what makes this safe: a request arriving from anywhere
/// else can set `X-Forwarded-For` freely and is ignored, exactly as
/// [`forward_auth_username`] treats its header. A trusted proxy that sends
/// something unparseable falls back to the peer rather than storing junk.
pub fn client_ip(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    config: &crate::config::Config,
) -> Option<String> {
    // Canonical form throughout, so a v4 client seen through a dual-stack
    // listener is stored as `203.0.113.7`, not `::ffff:203.0.113.7`.
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
    // The cookie is `<id>.<hmac>`; a bad signature short-circuits here, so a
    // forged or stale cookie never reaches the database.
    if let Some(session_id) = crate::secret::session_id_from_jar(&jar, &state.config.secret)
        && let Ok(Some(user)) = state.store.find_session_user(&session_id, now).await
        && !user.disabled
    {
        return Some(user);
    }
    // forward-auth fallback
    let peer_ip = peer_ip(&parts.extensions);
    forward_auth_user(state, &parts.headers, peer_ip, now).await
}

/// The socket peer of the request, as `into_make_service_with_connect_info`
/// records it in `main.rs`. `None` when the router is driven without connect
/// info, which makes every trusted-proxy check fail closed.
pub fn peer_ip(extensions: &axum::http::Extensions) -> Option<IpAddr> {
    extensions
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
}

/// Resolve the user named by a trusted forward-auth header, auto-provisioning a
/// non-admin, password-less account for a first-seen identity.
///
/// Returns `None` when forward-auth is not configured, the peer is not a
/// trusted proxy, the header is absent, or the named account is disabled.
/// Shared by [`resolve_user`] and `web::forward_auth_session`, which must agree
/// on exactly who a given request belongs to.
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

/// Like `CurrentUser`, but infallible: resolves the current user via session
/// cookie or trusted forward-auth header, yielding `None` instead of
/// redirecting when no user can be resolved. Useful for handlers (e.g. the
/// dashboard landing page) that need to branch on "no user" themselves
/// rather than being redirected to `/login`.
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
        // The original client is the leftmost entry; later hops are proxies.
        assert_eq!(with("203.0.113.7, 10.0.0.1"), "203.0.113.7");
        assert_eq!(with("  203.0.113.7  "), "203.0.113.7");
        // A trusted proxy sending nonsense falls back to the peer, never junk.
        assert_eq!(with("not-an-ip"), "10.0.0.1");
        assert_eq!(with(""), "10.0.0.1");
        // No header at all: the peer is all we have.
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
        // The Docker-bridge case: the proxy's address comes from a pool, so the
        // whole range has to be trusted, not one literal address.
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
        // /32 and /128 exercise the "no partial byte" path, which must not
        // index one past the address.
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
        // A dual-stack listener reports an IPv4 client as `::ffff:a.b.c.d`;
        // the operator writes the plain v4 address in the env var.
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

    /// Regression lock: `session_log_handle` must never leak the raw session
    /// id it derives from — it is the bearer secret backing the cookie.
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
            Some(now + idle)
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
        assert_eq!(result, cap);
        assert!(result <= cap);
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
        assert_eq!(refreshed_expiry(None, stale_expiry, now), Some(now + idle));
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
