//! API key tokens: `ugf_<id:8>_<secret:32>`, base62. Only `sha256(secret)` is stored.

use rand::Rng;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
pub const PREFIX: &str = "ugf_";
const ID_LEN: usize = 8;
const SECRET_LEN: usize = 32;

pub struct NewKey {
    pub id: String,
    pub token: String,
    pub secret_hash: [u8; 32],
}

fn random_base62(n: usize) -> String {
    let mut rng = rand::rng();
    (0..n).map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char).collect()
}

pub fn generate() -> NewKey {
    let id = random_base62(ID_LEN);
    let secret = random_base62(SECRET_LEN);
    NewKey { token: format!("{PREFIX}{id}_{secret}"), secret_hash: hash_secret(&secret), id }
}

pub fn hash_secret(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// Split a presented token into (id, secret). Anything malformed is `None`.
pub fn parse(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix(PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    let ok = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_alphanumeric());
    (ok(id, ID_LEN) && ok(secret, SECRET_LEN)).then_some((id, secret))
}

pub fn secret_matches(secret: &str, stored_hash: &[u8]) -> bool {
    hash_secret(secret).as_slice().ct_eq(stored_hash).into()
}

/// Everything needed to create a key.
pub struct MintParams {
    pub name: String,
    pub scopes: super::Scopes,
    pub deny_tags: Vec<String>,
    pub rate_limit: Option<u32>,
    pub expires_at: Option<i64>,
    pub created_by: i64,
}

/// Validate the name and rate limit shared by the API and the CLI.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("name must be 1 to 100 characters".into());
    }
    Ok(name.to_string())
}

pub fn validate_rate_limit(r: Option<i64>) -> Result<Option<u32>, String> {
    match r {
        None => Ok(None),
        Some(n) if (1..=1_000_000).contains(&n) => Ok(Some(n as u32)),
        Some(_) => Err("rate_limit must be between 1 and 1000000 requests per minute".into()),
    }
}

/// Create a key. Returns the stored row and the full token, which is shown
/// once and never stored.
pub async fn mint(
    db: &crate::db::Db,
    p: MintParams,
) -> Result<(crate::db::keys::KeyRow, String), sqlx::Error> {
    let k = generate();
    let row = crate::db::keys::KeyRow {
        id: k.id,
        name: p.name,
        secret_hash: k.secret_hash.to_vec(),
        scopes: p.scopes.to_db(),
        deny_tags: serde_json::to_string(&p.deny_tags).unwrap_or_else(|_| "[]".into()),
        rate_limit: p.rate_limit.map(i64::from),
        created_by: p.created_by,
        created_at: crate::db::now_ms(),
        expires_at: p.expires_at,
        last_used_at: None,
        revoked_at: None,
    };
    crate::db::keys::insert(db, &row).await?;
    Ok((row, k.token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_round_trip() {
        let k = generate();
        assert!(k.token.starts_with("ugf_"));
        let (id, secret) = parse(&k.token).unwrap();
        assert_eq!(id, k.id);
        assert!(secret_matches(secret, &k.secret_hash));
        assert!(!secret_matches("x", &k.secret_hash));
    }

    #[test]
    fn rejects_malformed_tokens() {
        assert!(parse("").is_none());
        assert!(parse("ugf_short_x").is_none());
        assert!(parse("nope_abcdefgh_abcdefghijklmnopqrstuvwxyzABCDEF").is_none());
        assert!(parse("ugf_abcdefgh_abcdefghijklmnopqrstuvwxyzABCDE!").is_none());
        assert!(parse("ugf_abcdefgh_abcdefghijklmnopqrstuvwxyzABCDEF").is_some());
    }

    #[test]
    fn tokens_are_unique() {
        assert_ne!(generate().token, generate().token);
    }
}
