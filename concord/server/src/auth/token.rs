use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// JWT claims for web session tokens.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user_id
    pub exp: i64,    // expiry (unix timestamp)
    pub iat: i64,    // issued at
}

/// Create a JWT session token for a user.
pub fn create_session_token(
    user_id: &str,
    secret: &str,
    expiry_hours: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        exp: (now + Duration::hours(expiry_hours)).timestamp(),
        iat: now.timestamp(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Validate a JWT session token and return the claims.
pub fn validate_session_token(
    token: &str,
    secret: &str,
) -> Result<Claims, jsonwebtoken::errors::Error> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(token_data.claims)
}

/// Generate a random IRC access token (64 hex characters).
pub fn generate_irc_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex_encode(&bytes)
}

/// Hash an IRC token with argon2 for storage.
pub fn hash_irc_token(token: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(token.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify an IRC token against a stored hash.
pub fn verify_irc_token(token: &str, hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(token.as_bytes(), &parsed_hash)
        .is_ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_roundtrip() {
        let secret = "test-secret";
        let token = create_session_token("user123", secret, 1).unwrap();
        let claims = validate_session_token(&token, secret).unwrap();
        assert_eq!(claims.sub, "user123");
    }

    #[test]
    fn test_jwt_invalid_secret() {
        let token = create_session_token("user123", "secret1", 1).unwrap();
        assert!(validate_session_token(&token, "secret2").is_err());
    }

    #[test]
    fn test_irc_token_generation() {
        let token = generate_irc_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_irc_token_hash_verify() {
        let token = generate_irc_token();
        let hash = hash_irc_token(&token).unwrap();
        assert!(verify_irc_token(&token, &hash));
        assert!(!verify_irc_token("wrong-token", &hash));
    }
}
