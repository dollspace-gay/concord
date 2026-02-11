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

    // ── Additional JWT tests ──

    #[test]
    fn test_jwt_claims_contain_correct_user_id() {
        let secret = "my-secret";
        let token = create_session_token("user-abc-123", secret, 24).unwrap();
        let claims = validate_session_token(&token, secret).unwrap();
        assert_eq!(claims.sub, "user-abc-123");
    }

    #[test]
    fn test_jwt_expiry_is_in_future() {
        let secret = "test";
        let token = create_session_token("u1", secret, 1).unwrap();
        let claims = validate_session_token(&token, secret).unwrap();
        let now = Utc::now().timestamp();
        // exp should be roughly 1 hour from now (within 10s tolerance)
        assert!(claims.exp > now);
        assert!(claims.exp <= now + 3610);
    }

    #[test]
    fn test_jwt_iat_is_recent() {
        let secret = "test";
        let token = create_session_token("u1", secret, 1).unwrap();
        let claims = validate_session_token(&token, secret).unwrap();
        let now = Utc::now().timestamp();
        // iat should be very close to now (within 5 seconds)
        assert!((claims.iat - now).abs() < 5);
    }

    #[test]
    fn test_jwt_different_users_produce_different_tokens() {
        let secret = "shared-secret";
        let t1 = create_session_token("user1", secret, 1).unwrap();
        let t2 = create_session_token("user2", secret, 1).unwrap();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_jwt_empty_secret_still_works() {
        let secret = "";
        let token = create_session_token("u1", secret, 1).unwrap();
        let claims = validate_session_token(&token, secret).unwrap();
        assert_eq!(claims.sub, "u1");
    }

    #[test]
    fn test_jwt_long_expiry() {
        let secret = "test";
        let token = create_session_token("u1", secret, 720).unwrap(); // 30 days
        let claims = validate_session_token(&token, secret).unwrap();
        let now = Utc::now().timestamp();
        // exp should be roughly 720 hours from now
        assert!(claims.exp > now + 719 * 3600);
    }

    #[test]
    fn test_jwt_validate_with_empty_string_fails() {
        assert!(validate_session_token("", "secret").is_err());
    }

    #[test]
    fn test_jwt_validate_with_garbage_fails() {
        assert!(validate_session_token("not-a-jwt-token", "secret").is_err());
    }

    #[test]
    fn test_jwt_validate_with_tampered_token_fails() {
        let token = create_session_token("u1", "secret", 1).unwrap();
        // Flip a character in the middle of the token
        let mut chars: Vec<char> = token.chars().collect();
        let mid = chars.len() / 2;
        chars[mid] = if chars[mid] == 'a' { 'b' } else { 'a' };
        let tampered: String = chars.into_iter().collect();
        assert!(validate_session_token(&tampered, "secret").is_err());
    }

    // ── Additional IRC token tests ──

    #[test]
    fn test_irc_token_uniqueness() {
        let t1 = generate_irc_token();
        let t2 = generate_irc_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_irc_token_is_hex() {
        let token = generate_irc_token();
        for c in token.chars() {
            assert!(c.is_ascii_hexdigit(), "Expected hex char but got '{}'", c);
        }
    }

    #[test]
    fn test_irc_token_is_lowercase_hex() {
        let token = generate_irc_token();
        // hex_encode uses lowercase format specifier
        assert_eq!(token, token.to_lowercase());
    }

    #[test]
    fn test_irc_token_hash_is_argon2_format() {
        let token = generate_irc_token();
        let hash = hash_irc_token(&token).unwrap();
        // argon2 hashes start with $argon2
        assert!(
            hash.starts_with("$argon2"),
            "Hash should start with $argon2, got: {}",
            &hash[..20]
        );
    }

    #[test]
    fn test_irc_token_same_token_different_hashes() {
        // Each hash uses a different salt, so same input -> different hashes
        let token = generate_irc_token();
        let h1 = hash_irc_token(&token).unwrap();
        let h2 = hash_irc_token(&token).unwrap();
        assert_ne!(
            h1, h2,
            "Same token should produce different hashes due to random salt"
        );
        // But both should verify
        assert!(verify_irc_token(&token, &h1));
        assert!(verify_irc_token(&token, &h2));
    }

    #[test]
    fn test_verify_irc_token_with_invalid_hash_returns_false() {
        assert!(!verify_irc_token("sometoken", "not-a-valid-hash"));
    }

    #[test]
    fn test_verify_irc_token_with_empty_hash_returns_false() {
        assert!(!verify_irc_token("sometoken", ""));
    }

    #[test]
    fn test_hex_encode_known_values() {
        assert_eq!(hex_encode(&[0x00]), "00");
        assert_eq!(hex_encode(&[0xff]), "ff");
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[]), "");
    }
}
