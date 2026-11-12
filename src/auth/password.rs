//! Argon2id password hashing (PHC strings).

use anyhow::{anyhow, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;

pub const MIN_LEN: usize = 8;
pub const MAX_LEN: usize = 1024;

pub fn validate_new(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < MIN_LEN {
        return Err(format!("password must be at least {MIN_LEN} characters"));
    }
    if n > MAX_LEN {
        return Err(format!("password must be at most {MAX_LEN} characters"));
    }
    Ok(())
}

pub fn hash(password: &str) -> Result<String> {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt).map_err(|e| anyhow!("salt: {e}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("hashing password: {e}"))
}

pub fn verify(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// Burn the same CPU as a real check, so an unknown username is not
/// distinguishable from a wrong password by timing.
pub fn verify_dummy(password: &str) {
    use std::sync::OnceLock;
    static DUMMY: OnceLock<String> = OnceLock::new();
    let phc = DUMMY.get_or_init(|| hash("unigif-dummy-password").expect("hashing a constant"));
    let _ = verify(password, phc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("correct horse", &h));
        assert!(!verify("wrong horse", &h));
        assert!(!verify("x", "not a phc string"));
        // Salted: same password, different hashes.
        assert_ne!(h, hash("correct horse").unwrap());
    }

    #[test]
    fn length_rules() {
        assert!(validate_new("short").is_err());
        assert!(validate_new("long enough").is_ok());
        assert!(validate_new(&"x".repeat(MAX_LEN + 1)).is_err());
    }
}
