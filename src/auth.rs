use crate::models::User;
use crate::state::AppState;
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, StatusCode, request::Parts};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use std::net::{IpAddr, SocketAddr};

pub const SESSION_COOKIE: &str = "pingward_session";
pub const SESSION_TTL_DAYS: i64 = 30;

pub fn new_session_token() -> String {
    uuid::Uuid::new_v4().to_string()
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
    let peer_ip = parts
        .extensions
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    if let Some(username) = forward_auth_username(&parts.headers, peer_ip, &state.config) {
        match state.store.find_user_by_username(&username).await {
            Ok(Some(user)) => {
                if !user.disabled {
                    return Some(user);
                }
            }
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
}
