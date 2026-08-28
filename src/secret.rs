//! Keyed derivation for session cookies and CSRF tokens.
//!
//! One process-wide secret (`PINGWARD_SECRET`, or a random one per boot) backs
//! both browser credentials: the session cookie is `<session_id>.<hmac>`, and
//! the CSRF token is derived from the same id — hence no `csrf_token` column.
//!
//! The tags are domain-separated. Without the prefixes both values would be
//! identical, and every rendered form would print the cookie's signature.
//!
//! Rotating the secret (including the implicit rotation of a restart with no
//! `PINGWARD_SECRET` set) ends every browser session. API keys are unaffected;
//! they are matched by SHA-256 digest (see [`crate::apikey`]).

use axum_extra::extract::cookie::CookieJar;
use hmac::{Hmac, KeyInit, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const SESSION_DOMAIN: &[u8] = b"session:";
const CSRF_DOMAIN: &[u8] = b"csrf:";
const FLASH_DOMAIN: &[u8] = b"flash:";

/// Separates payload from signature. Session ids are hyphenated UUIDs, which
/// never contain it, so `rsplit_once` cannot cut into the id.
const SIG_SEPARATOR: char = '.';

const GENERATED_SECRET_BYTES: usize = 32;

/// Shortest `PINGWARD_SECRET` accepted; a guessable secret mints both session
/// cookies and CSRF tokens.
pub const MIN_SECRET_LEN: usize = 16;

/// Where the process's secret came from, for the one-time startup warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSource {
    Env,
    /// Unset or blank; a random secret was generated for this process only.
    Generated,
    /// Set but shorter than [`MIN_SECRET_LEN`], so it was ignored and a random
    /// secret generated. Separate from [`SecretSource::Generated`] so the
    /// startup warning can say "ignored" rather than "missing".
    Rejected,
}

/// Resolve the process secret. The raw value is used as-is (no base64
/// decoding); generate one with `openssl rand -hex 32`.
pub fn resolve(raw: Option<&str>) -> (Vec<u8>, SecretSource) {
    match raw {
        Some(v) if v.len() >= MIN_SECRET_LEN => (v.as_bytes().to_vec(), SecretSource::Env),
        Some(_) => (generate(), SecretSource::Rejected),
        None => (generate(), SecretSource::Generated),
    }
}

fn generate() -> Vec<u8> {
    let mut buf = vec![0u8; GENERATED_SECRET_BYTES];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// Keyed MAC over `domain ++ message`, ready to finalize or verify.
fn mac(secret: &[u8], domain: &[u8], message: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(domain);
    mac.update(message.as_bytes());
    mac
}

/// Attach a signature to `value` as `<payload>.<hmac>`.
fn sign(secret: &[u8], domain: &[u8], value: &str) -> String {
    let sig = hex_encode(&mac(secret, domain, value).finalize().into_bytes());
    format!("{value}{SIG_SEPARATOR}{sig}")
}

/// Recover the payload from `<payload>.<hmac>`, if the signature verifies.
fn verify(secret: &[u8], domain: &[u8], signed: &str) -> Option<String> {
    let (value, sig) = signed.rsplit_once(SIG_SEPARATOR)?;
    let sig = hex_decode(sig)?;
    mac(secret, domain, value).verify_slice(&sig).ok()?;
    Some(value.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Decode hex, either case. `None` for odd length or a non-hex byte, which is
/// how a malformed signature is rejected before any comparison.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

pub fn sign_session(secret: &[u8], session_id: &str) -> String {
    sign(secret, SESSION_DOMAIN, session_id)
}

/// Recover the session id from a cookie value. Callers must use this rather
/// than the raw cookie value — the value is no longer the id.
pub fn verify_session(secret: &[u8], cookie_value: &str) -> Option<String> {
    verify(secret, SESSION_DOMAIN, cookie_value)
}

/// Sign the one-shot flash cookie's payload.
///
/// The cookie carries no authority; the signature adds provenance. Without it
/// a sibling subdomain can plant a flash this origin never set, so the user
/// reads a message the server never sent (a fabricated "N API keys still work"
/// count). Payloads never contain [`SIG_SEPARATOR`].
pub fn sign_flash(secret: &[u8], value: &str) -> String {
    sign(secret, FLASH_DOMAIN, value)
}

pub fn verify_flash(secret: &[u8], cookie_value: &str) -> Option<String> {
    verify(secret, FLASH_DOMAIN, cookie_value)
}

/// The session's CSRF token, embedded in forms as `_csrf` and accepted as the
/// `X-CSRF-Token` header.
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
pub fn session_id_from_jar(jar: &CookieJar, secret: &[u8], cookie_name: &str) -> Option<String> {
    verify_session(secret, jar.get(cookie_name)?.value())
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
        // The bare id, the pre-signing cookie format.
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

    /// Or rendering a form would leak the cookie's signature.
    #[test]
    fn csrf_token_differs_from_the_session_signature() {
        let cookie = sign_session(SECRET, ID);
        let (_, sig) = cookie.rsplit_once(SIG_SEPARATOR).unwrap();
        assert_ne!(sig, derive_csrf(SECRET, ID));
    }

    #[test]
    fn flash_value_round_trips() {
        let signed = sign_flash(SECRET, "settings");
        assert!(signed.starts_with("settings."));
        assert_eq!(verify_flash(SECRET, &signed).as_deref(), Some("settings"));
    }

    #[test]
    fn an_unsigned_or_tampered_flash_is_rejected() {
        // The pre-signing format: what a sibling subdomain would plant.
        assert!(verify_flash(SECRET, "settings").is_none());
        assert!(verify_flash(SECRET, "password_reset_keys:1:9").is_none());
        let signed = sign_flash(SECRET, "password_reset_keys:1:2");
        let (_, sig) = signed.rsplit_once(SIG_SEPARATOR).unwrap();
        // The counts are what a planted cookie would want to change.
        assert!(verify_flash(SECRET, &format!("password_reset_keys:1:9.{sig}")).is_none());
        assert!(verify_flash(b"another-secret-16-plus", &signed).is_none());
    }

    /// Or a captured session cookie could be replayed as a flash.
    #[test]
    fn flash_and_session_signatures_do_not_cross_verify() {
        assert!(verify_flash(SECRET, &sign_session(SECRET, ID)).is_none());
        assert!(verify_session(SECRET, &sign_flash(SECRET, ID)).is_none());
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
        // Or "restart logs everyone out" would silently stop holding.
        let (b, _) = resolve(None);
        assert_ne!(a, b);
    }
}
