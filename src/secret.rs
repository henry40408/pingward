//! Keyed derivation for session cookies and CSRF tokens.
//!
//! One process-wide secret (`PINGWARD_SECRET`, or a random one generated at
//! boot) backs both browser-facing credentials:
//!
//! - the session cookie is `<session_id>.<hmac>`, so a cookie with a bad
//!   signature is rejected before any database work, and a leaked `sessions.id`
//!   is not usable on its own;
//! - the CSRF synchronizer token is derived from the same session id, which is
//!   why `sessions` needs no `csrf_token` column.
//!
//! Both tags are domain-separated. Without the prefixes they would be the same
//! value, and every rendered form embeds the CSRF token — which would print the
//! session cookie's signature into the page body.
//!
//! Rotating the secret (including the implicit rotation of a restart with no
//! `PINGWARD_SECRET` set) invalidates every signature at once, so all browser
//! sessions end. API keys are unaffected: they are random bearer tokens matched
//! by SHA-256 digest (see [`crate::apikey`]) and never touch this secret.

use axum_extra::extract::cookie::CookieJar;
use hmac::{Hmac, KeyInit, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;

use crate::auth::SESSION_COOKIE;

type HmacSha256 = Hmac<Sha256>;

/// Domain-separation prefix for the session cookie signature.
const SESSION_DOMAIN: &[u8] = b"session:";
/// Domain-separation prefix for the CSRF synchronizer token.
const CSRF_DOMAIN: &[u8] = b"csrf:";

/// Separates the session id from its signature in the cookie value. Session ids
/// are hyphenated UUIDs, which never contain it, so `rsplit_once` cannot cut
/// into the id itself.
const SIG_SEPARATOR: char = '.';

/// Bytes of randomness in a generated secret.
const GENERATED_SECRET_BYTES: usize = 32;

/// Shortest `PINGWARD_SECRET` accepted. Anything shorter is treated as
/// misconfiguration rather than silently used, since a guessable secret lets an
/// attacker mint both session cookies and CSRF tokens.
pub const MIN_SECRET_LEN: usize = 16;

/// Where the process's secret came from, for the one-time startup warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSource {
    /// Read from `PINGWARD_SECRET`.
    Env,
    /// `PINGWARD_SECRET` was unset or blank; a random secret was generated for
    /// this process only.
    Generated,
    /// `PINGWARD_SECRET` was set but shorter than [`MIN_SECRET_LEN`], so it was
    /// rejected and a random secret generated instead. Distinguished from
    /// [`SecretSource::Generated`] so the startup warning can say the value was
    /// ignored rather than missing.
    Rejected,
}

/// Resolve the process secret from an optional `PINGWARD_SECRET` value.
///
/// The raw value is used as-is (no base64 decoding) so any sufficiently long
/// string works; generate one with `openssl rand -hex 32`.
pub fn resolve(raw: Option<&str>) -> (Vec<u8>, SecretSource) {
    match raw {
        Some(v) if v.len() >= MIN_SECRET_LEN => (v.as_bytes().to_vec(), SecretSource::Env),
        Some(_) => (generate(), SecretSource::Rejected),
        None => (generate(), SecretSource::Generated),
    }
}

/// A fresh random secret, used when none is configured.
fn generate() -> Vec<u8> {
    let mut buf = vec![0u8; GENERATED_SECRET_BYTES];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// Keyed MAC over `domain ++ session_id`, ready to finalize or verify.
fn mac(secret: &[u8], domain: &[u8], session_id: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(domain);
    mac.update(session_id.as_bytes());
    mac
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Decode lowercase-or-uppercase hex. `None` for odd length or a non-hex byte,
/// which is how a malformed signature or CSRF token gets rejected before any
/// comparison happens.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Build the cookie value for `session_id`: the id plus its signature.
pub fn sign_session(secret: &[u8], session_id: &str) -> String {
    let sig = hex_encode(
        &mac(secret, SESSION_DOMAIN, session_id)
            .finalize()
            .into_bytes(),
    );
    format!("{session_id}{SIG_SEPARATOR}{sig}")
}

/// Recover the session id from a cookie value, or `None` when the value is
/// malformed or the signature does not verify. Callers must use this rather
/// than the raw cookie value — the value is no longer the id.
pub fn verify_session(secret: &[u8], cookie_value: &str) -> Option<String> {
    let (session_id, sig) = cookie_value.rsplit_once(SIG_SEPARATOR)?;
    let sig = hex_decode(sig)?;
    mac(secret, SESSION_DOMAIN, session_id)
        .verify_slice(&sig)
        .ok()?;
    Some(session_id.to_string())
}

/// The CSRF synchronizer token for a session, embedded in rendered forms as
/// `_csrf` and accepted as the `X-CSRF-Token` header.
pub fn derive_csrf(secret: &[u8], session_id: &str) -> String {
    hex_encode(&mac(secret, CSRF_DOMAIN, session_id).finalize().into_bytes())
}

/// Constant-time check of a submitted CSRF token against the session's own.
pub fn verify_csrf(secret: &[u8], session_id: &str, submitted: &str) -> bool {
    let Some(bytes) = hex_decode(submitted) else {
        return false;
    };
    mac(secret, CSRF_DOMAIN, session_id)
        .verify_slice(&bytes)
        .is_ok()
}

/// The verified session id carried by a request's cookies, if any.
pub fn session_id_from_jar(jar: &CookieJar, secret: &[u8]) -> Option<String> {
    verify_session(secret, jar.get(SESSION_COOKIE)?.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-at-least-16-bytes";
    const ID: &str = "0b3c9a1e-4f2d-4a7b-9c8e-1d2f3a4b5c6d";

    #[test]
    fn signed_cookie_round_trips() {
        let cookie = sign_session(SECRET, ID);
        assert!(cookie.starts_with(ID));
        assert_eq!(verify_session(SECRET, &cookie).as_deref(), Some(ID));
    }

    #[test]
    fn signature_is_required() {
        // The bare id — the pre-signing cookie format — must not verify.
        assert!(verify_session(SECRET, ID).is_none());
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let cookie = sign_session(SECRET, ID);
        let mut bad = cookie.clone();
        bad.pop();
        bad.push(if cookie.ends_with('0') { '1' } else { '0' });
        assert!(verify_session(SECRET, &bad).is_none());
    }

    #[test]
    fn tampered_id_is_rejected() {
        let cookie = sign_session(SECRET, ID);
        let (_, sig) = cookie.rsplit_once(SIG_SEPARATOR).unwrap();
        assert!(verify_session(SECRET, &format!("other-id.{sig}")).is_none());
    }

    #[test]
    fn malformed_values_are_rejected() {
        for bad in ["", ".", "id.", "id.zz", "id.abc", ID] {
            assert!(verify_session(SECRET, bad).is_none(), "must reject {bad:?}");
        }
    }

    #[test]
    fn a_different_secret_invalidates_the_cookie() {
        let cookie = sign_session(SECRET, ID);
        assert!(verify_session(b"another-secret-16-plus", &cookie).is_none());
    }

    #[test]
    fn csrf_token_verifies_and_is_session_scoped() {
        let token = derive_csrf(SECRET, ID);
        assert!(verify_csrf(SECRET, ID, &token));
        assert!(!verify_csrf(SECRET, "some-other-session", &token));
        assert!(!verify_csrf(b"another-secret-16-plus", ID, &token));
        assert!(!verify_csrf(SECRET, ID, "not-hex"));
    }

    /// Domain separation: the CSRF token must never equal the session
    /// signature, or rendering a form would leak the cookie's signature.
    #[test]
    fn csrf_token_differs_from_the_session_signature() {
        let cookie = sign_session(SECRET, ID);
        let (_, sig) = cookie.rsplit_once(SIG_SEPARATOR).unwrap();
        assert_ne!(sig, derive_csrf(SECRET, ID));
    }

    #[test]
    fn resolve_uses_a_long_enough_env_value() {
        let raw = "x".repeat(MIN_SECRET_LEN);
        let (secret, source) = resolve(Some(&raw));
        assert_eq!(secret, raw.as_bytes());
        assert_eq!(source, SecretSource::Env);
    }

    #[test]
    fn resolve_rejects_a_short_env_value() {
        let (secret, source) = resolve(Some("tooshort"));
        assert_eq!(source, SecretSource::Rejected);
        assert_eq!(secret.len(), GENERATED_SECRET_BYTES);
    }

    #[test]
    fn resolve_generates_when_unset() {
        let (a, source) = resolve(None);
        assert_eq!(source, SecretSource::Generated);
        assert_eq!(a.len(), GENERATED_SECRET_BYTES);
        // Two boots must not share a secret, or "restart logs everyone out"
        // would silently stop holding.
        let (b, _) = resolve(None);
        assert_ne!(a, b);
    }
}
