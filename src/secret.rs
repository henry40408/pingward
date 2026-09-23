//! Keyed derivation for session cookies, CSRF tokens and the flash cookie.
//!
//! One process secret (`PINGWARD_SECRET`, else random per boot) backs them all:
//! the session cookie is `<session_id>.<hmac>` and the CSRF token is derived from
//! the id (hence no `csrf_token` column). The HMACs are domain-separated by
//! prefix; without it the CSRF token would equal the cookie's signature and every
//! form would print it. Rotating the secret (or restarting without one) ends every
//! browser session; API keys are unaffected (see [`crate::apikey`]).

use axum_extra::extract::cookie::CookieJar;
use hmac::{Hmac, KeyInit, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const SESSION_DOMAIN: &[u8] = b"session:";
const CSRF_DOMAIN: &[u8] = b"csrf:";
const FLASH_DOMAIN: &[u8] = b"flash:";

/// Separates payload from signature; `verify` splits on the last one.
const SIG_SEPARATOR: char = '.';

const GENERATED_SECRET_BYTES: usize = 32;

/// Shortest `PINGWARD_SECRET` accepted, in bytes.
pub const MIN_SECRET_LEN: usize = 16;

/// Where the process's secret came from, for the one-time startup warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSource {
    Env,
    /// Unset or blank; a random secret was generated for this process only.
    Generated,
    /// Shorter than [`MIN_SECRET_LEN`], so ignored and a random secret generated.
    Rejected,
}

/// Resolve the process secret; the raw value's bytes are used as-is (no decoding).
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

/// HMAC over `domain ++ message`.
fn mac(secret: &[u8], domain: &[u8], message: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(domain);
    mac.update(message.as_bytes());
    mac
}

fn sign(secret: &[u8], domain: &[u8], value: &str) -> String {
    let sig = hex_encode(&mac(secret, domain, value).finalize().into_bytes());
    format!("{value}{SIG_SEPARATOR}{sig}")
}

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

/// Decode hex (either case); `None` for odd length or a non-hex byte.
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

/// Recover the session id from a cookie value, which is not itself the id.
pub fn verify_session(secret: &[u8], cookie_value: &str) -> Option<String> {
    verify(secret, SESSION_DOMAIN, cookie_value)
}

/// Sign the one-shot flash cookie, so a sibling subdomain cannot plant a message
/// the server never sent.
pub fn sign_flash(secret: &[u8], value: &str) -> String {
    sign(secret, FLASH_DOMAIN, value)
}

pub fn verify_flash(secret: &[u8], cookie_value: &str) -> Option<String> {
    verify(secret, FLASH_DOMAIN, cookie_value)
}

/// The session's CSRF token (`_csrf` field or `X-CSRF-Token` header).
pub fn derive_csrf(secret: &[u8], session_id: &str) -> String {
    hex_encode(&mac(secret, CSRF_DOMAIN, session_id).finalize().into_bytes())
}

/// Constant-time check of a submitted CSRF token.
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
        assert!(verify_flash(SECRET, "settings").is_none());
        assert!(verify_flash(SECRET, "password_reset_keys:1:9").is_none());
        let signed = sign_flash(SECRET, "password_reset_keys:1:2");
        let (_, sig) = signed.rsplit_once(SIG_SEPARATOR).unwrap();
        assert!(verify_flash(SECRET, &format!("password_reset_keys:1:9.{sig}")).is_none());
        assert!(verify_flash(b"another-secret-16-plus", &signed).is_none());
    }

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
        let (b, _) = resolve(None);
        assert_ne!(a, b);
    }
}
