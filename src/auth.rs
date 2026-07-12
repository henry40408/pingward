use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

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
}
