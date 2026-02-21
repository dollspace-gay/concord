/// Authentication configuration, loaded from environment variables.
#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub session_expiry_hours: i64,
    pub public_url: String,
}

impl AuthConfig {
    /// Load auth config from environment variables.
    ///
    /// In production, `JWT_SECRET` must be set. In development (PUBLIC_URL
    /// pointing at localhost), a random secret is generated if not provided.
    pub fn from_env() -> Self {
        let public_url =
            std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
        let is_dev = public_url.starts_with("http://localhost")
            || public_url.starts_with("http://127.0.0.1");

        let jwt_secret = match std::env::var("JWT_SECRET") {
            Ok(s) if !s.is_empty() => s,
            _ if is_dev => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                // Generate a per-process random-ish secret for dev
                let mut h = DefaultHasher::new();
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .hash(&mut h);
                std::process::id().hash(&mut h);
                format!("dev-auto-{:x}", h.finish())
            }
            _ => {
                eprintln!(
                    "FATAL: JWT_SECRET environment variable is required in production.\n\
                     Set JWT_SECRET to a strong random string (e.g., `openssl rand -hex 32`).\n\
                     For local development, use PUBLIC_URL=http://localhost:8080 to auto-generate."
                );
                std::process::exit(1);
            }
        };

        Self {
            jwt_secret,
            session_expiry_hours: std::env::var("SESSION_EXPIRY_HOURS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(720), // 30 days
            public_url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let _lock = ENV_LOCK.lock().unwrap();

        let keys = ["JWT_SECRET", "SESSION_EXPIRY_HOURS", "PUBLIC_URL"];
        let originals: Vec<_> = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();

        for key in &keys {
            unsafe {
                std::env::remove_var(key);
            }
        }

        for (k, v) in vars {
            unsafe {
                std::env::set_var(k, v);
            }
        }

        f();

        for (k, v) in originals {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    #[test]
    fn test_defaults_when_no_env_vars() {
        with_env(&[], || {
            let config = AuthConfig::from_env();
            // In dev mode (localhost), a random secret is auto-generated
            assert!(
                config.jwt_secret.starts_with("dev-auto-"),
                "Dev mode should auto-generate a secret, got: {}",
                config.jwt_secret
            );
            assert_eq!(config.session_expiry_hours, 720);
            assert_eq!(config.public_url, "http://localhost:8080");
        });
    }

    #[test]
    fn test_jwt_secret_from_env() {
        with_env(&[("JWT_SECRET", "super-secret-key")], || {
            let config = AuthConfig::from_env();
            assert_eq!(config.jwt_secret, "super-secret-key");
        });
    }

    #[test]
    fn test_session_expiry_from_env() {
        with_env(&[("SESSION_EXPIRY_HOURS", "48")], || {
            let config = AuthConfig::from_env();
            assert_eq!(config.session_expiry_hours, 48);
        });
    }

    #[test]
    fn test_session_expiry_invalid_falls_back_to_default() {
        with_env(&[("SESSION_EXPIRY_HOURS", "not-a-number")], || {
            let config = AuthConfig::from_env();
            assert_eq!(config.session_expiry_hours, 720);
        });
    }

    #[test]
    fn test_public_url_from_env() {
        with_env(
            &[
                ("PUBLIC_URL", "https://chat.example.com"),
                ("JWT_SECRET", "test-secret-for-prod-url"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert_eq!(config.public_url, "https://chat.example.com");
            },
        );
    }

    #[test]
    fn test_auth_config_clone() {
        with_env(&[("JWT_SECRET", "clone-test-secret")], || {
            let config = AuthConfig::from_env();
            let cloned = config.clone();
            assert_eq!(cloned.jwt_secret, config.jwt_secret);
            assert_eq!(cloned.session_expiry_hours, config.session_expiry_hours);
            assert_eq!(cloned.public_url, config.public_url);
        });
    }

    #[test]
    fn test_all_config_values_set() {
        with_env(
            &[
                ("JWT_SECRET", "my-jwt"),
                ("SESSION_EXPIRY_HOURS", "24"),
                ("PUBLIC_URL", "https://prod.example.com"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert_eq!(config.jwt_secret, "my-jwt");
                assert_eq!(config.session_expiry_hours, 24);
                assert_eq!(config.public_url, "https://prod.example.com");
            },
        );
    }
}
