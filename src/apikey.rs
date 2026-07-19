//! API-key generation and hashing for the programmatic REST API.
//!
//! A key is `pw_` followed by 64 hex characters (two v4 UUIDs, ~244 bits of
//! randomness). Only the SHA-256 hash is persisted (`api_keys.token_hash`,
//! UNIQUE) so authentication is an indexed equality lookup; the plaintext is
//! shown to the user exactly once, at creation. The hash is unsalted on
//! purpose — it must be deterministic to be looked up — which is safe because
//! the input is a high-entropy random secret, not a low-entropy password.

use sha2::{Digest, Sha256};

/// Prefix every key carries, both as a namespace and a quick visual cue.
pub const API_KEY_PREFIX: &str = "pw_";

/// Number of body characters (after the prefix) kept as the non-secret,
/// displayable `prefix` column — enough to tell keys apart in a list.
const DISPLAY_BODY_CHARS: usize = 8;

/// Mint a new API key. Returns `(full_token, display_prefix, token_hash)`:
/// the caller shows `full_token` to the user once, and stores `display_prefix`
/// + `token_hash` in the database.
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

/// SHA-256 of a token, lowercase hex. Deterministic, so the same token always
/// hashes to the same value used for the `WHERE token_hash = $1` lookup.
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
        // prefix + 64 hex body.
        assert_eq!(full.len(), API_KEY_PREFIX.len() + 64);
        assert!(
            full[API_KEY_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        // Display prefix is the shared prefix plus the first few body chars.
        assert_eq!(prefix.len(), API_KEY_PREFIX.len() + DISPLAY_BODY_CHARS);
        assert!(full.starts_with(&prefix));
        // Hash is 64 hex chars (SHA-256) and matches a re-hash of the token.
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
