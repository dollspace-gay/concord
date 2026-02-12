/// Authentication configuration, loaded from environment variables.
#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub session_expiry_hours: i64,
    pub public_url: String,
}

impl AuthConfig {
    /// Load auth config from environment variables.
    pub fn from_env() -> Self {
        Self {
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "concord-dev-secret-change-me".to_string()),
            session_expiry_hours: std::env::var("SESSION_EXPIRY_HOURS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(720), // 30 days
            public_url: std::env::var("PUBLIC_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
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
            assert_eq!(config.jwt_secret, "concord-dev-secret-change-me");
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
        with_env(&[("PUBLIC_URL", "https://chat.example.com")], || {
            let config = AuthConfig::from_env();
            assert_eq!(config.public_url, "https://chat.example.com");
        });
    }

    #[test]
    fn test_auth_config_clone() {
        with_env(&[], || {
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
