//! API-key generation and hashing for the REST API.
//!
//! A key is `pw_` + 64 hex characters (two v4 UUIDs, ~244 bits). Only the
//! SHA-256 hash is persisted (`api_keys.token_hash`, UNIQUE) so authentication
//! is an indexed equality lookup; the plaintext is shown exactly once, at
//! creation. The hash is unsalted because it must be deterministic to be
//! looked up — safe because the input is a high-entropy random secret, not a
//! low-entropy password.

use sha2::{Digest, Sha256};

pub const API_KEY_PREFIX: &str = "pw_";

/// Body characters (after the prefix) kept as the non-secret, displayable
/// `prefix` column.
const DISPLAY_BODY_CHARS: usize = 8;

/// Returns `(full_token, display_prefix, token_hash)`. `full_token` is shown
/// to the user once and never stored.
pub fn generate_api_key() -> (String, String, String) {
    let body = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let full = format!("{API_KEY_PREFIX}{body}");
    let prefix = format!("{API_KEY_PREFIX}{}", &body[..DISPLAY_BODY_CHARS]);
    let hash = hash_api_key(&full);
    (full, prefix, hash)
}

/// SHA-256 of a token, lowercase hex — the value `WHERE token_hash = $1`
/// looks up.
pub fn hash_api_key(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_expected_shape() {
        let (full, prefix, hash) = generate_api_key();
        assert!(full.starts_with(API_KEY_PREFIX));
        assert_eq!(full.len(), API_KEY_PREFIX.len() + 64);
        assert!(
            full[API_KEY_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert_eq!(prefix.len(), API_KEY_PREFIX.len() + DISPLAY_BODY_CHARS);
        assert!(full.starts_with(&prefix));
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, hash_api_key(&full));
    }

    #[test]
    fn distinct_keys_and_hashes() {
        let (a, _, ah) = generate_api_key();
        let (b, _, bh) = generate_api_key();
        assert_ne!(a, b);
        assert_ne!(ah, bh);
    }

    #[test]
    fn hash_is_stable_and_specific() {
        assert_eq!(hash_api_key("pw_abc"), hash_api_key("pw_abc"));
        assert_ne!(hash_api_key("pw_abc"), hash_api_key("pw_abd"));
        // Known SHA-256 vector.
        assert_eq!(
            hash_api_key("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
