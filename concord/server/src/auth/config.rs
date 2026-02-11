/// Authentication configuration, loaded from environment variables.
#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub session_expiry_hours: i64,
    pub public_url: String,
    pub github: Option<OAuthProviderConfig>,
    pub google: Option<OAuthProviderConfig>,
}

#[derive(Clone)]
pub struct OAuthProviderConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl AuthConfig {
    /// Load auth config from environment variables.
    /// Only providers with both CLIENT_ID and CLIENT_SECRET set will be enabled.
    pub fn from_env() -> Self {
        let github = match (
            std::env::var("GITHUB_CLIENT_ID"),
            std::env::var("GITHUB_CLIENT_SECRET"),
        ) {
            (Ok(id), Ok(secret)) if !id.is_empty() && !secret.is_empty() => {
                Some(OAuthProviderConfig {
                    client_id: id,
                    client_secret: secret,
                })
            }
            _ => None,
        };

        let google = match (
            std::env::var("GOOGLE_CLIENT_ID"),
            std::env::var("GOOGLE_CLIENT_SECRET"),
        ) {
            (Ok(id), Ok(secret)) if !id.is_empty() && !secret.is_empty() => {
                Some(OAuthProviderConfig {
                    client_id: id,
                    client_secret: secret,
                })
            }
            _ => None,
        };

        Self {
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "concord-dev-secret-change-me".to_string()),
            session_expiry_hours: std::env::var("SESSION_EXPIRY_HOURS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(720), // 30 days
            public_url: std::env::var("PUBLIC_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            github,
            google,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests that modify environment variables must be serialized to avoid races.
    // We use a mutex to ensure only one test modifies env vars at a time.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: clear all OAuth-related env vars and set specific ones.
    fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let _lock = ENV_LOCK.lock().unwrap();

        // Save originals
        let keys = [
            "JWT_SECRET",
            "SESSION_EXPIRY_HOURS",
            "PUBLIC_URL",
            "GITHUB_CLIENT_ID",
            "GITHUB_CLIENT_SECRET",
            "GOOGLE_CLIENT_ID",
            "GOOGLE_CLIENT_SECRET",
        ];
        let originals: Vec<_> = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();

        // Clear all
        for key in &keys {
            // SAFETY: tests run serially (not multi-threaded env access)
            unsafe {
                std::env::remove_var(key);
            }
        }

        // Set requested
        for (k, v) in vars {
            // SAFETY: tests run serially (not multi-threaded env access)
            unsafe {
                std::env::set_var(k, v);
            }
        }

        f();

        // Restore originals
        for (k, v) in originals {
            match v {
                // SAFETY: tests run serially (not multi-threaded env access)
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
            assert!(config.github.is_none());
            assert!(config.google.is_none());
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
    fn test_github_provider_enabled() {
        with_env(
            &[
                ("GITHUB_CLIENT_ID", "gh-id-123"),
                ("GITHUB_CLIENT_SECRET", "gh-secret-456"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert!(config.github.is_some());
                let gh = config.github.unwrap();
                assert_eq!(gh.client_id, "gh-id-123");
                assert_eq!(gh.client_secret, "gh-secret-456");
            },
        );
    }

    #[test]
    fn test_github_provider_disabled_when_id_missing() {
        with_env(&[("GITHUB_CLIENT_SECRET", "gh-secret")], || {
            let config = AuthConfig::from_env();
            assert!(config.github.is_none());
        });
    }

    #[test]
    fn test_github_provider_disabled_when_secret_missing() {
        with_env(&[("GITHUB_CLIENT_ID", "gh-id")], || {
            let config = AuthConfig::from_env();
            assert!(config.github.is_none());
        });
    }

    #[test]
    fn test_github_provider_disabled_when_id_empty() {
        with_env(
            &[
                ("GITHUB_CLIENT_ID", ""),
                ("GITHUB_CLIENT_SECRET", "gh-secret"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert!(config.github.is_none());
            },
        );
    }

    #[test]
    fn test_github_provider_disabled_when_secret_empty() {
        with_env(
            &[("GITHUB_CLIENT_ID", "gh-id"), ("GITHUB_CLIENT_SECRET", "")],
            || {
                let config = AuthConfig::from_env();
                assert!(config.github.is_none());
            },
        );
    }

    #[test]
    fn test_google_provider_enabled() {
        with_env(
            &[
                ("GOOGLE_CLIENT_ID", "goog-id"),
                ("GOOGLE_CLIENT_SECRET", "goog-secret"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert!(config.google.is_some());
                let g = config.google.unwrap();
                assert_eq!(g.client_id, "goog-id");
                assert_eq!(g.client_secret, "goog-secret");
            },
        );
    }

    #[test]
    fn test_google_provider_disabled_when_incomplete() {
        with_env(&[("GOOGLE_CLIENT_ID", "goog-id")], || {
            let config = AuthConfig::from_env();
            assert!(config.google.is_none());
        });
    }

    #[test]
    fn test_both_providers_enabled() {
        with_env(
            &[
                ("GITHUB_CLIENT_ID", "gh-id"),
                ("GITHUB_CLIENT_SECRET", "gh-secret"),
                ("GOOGLE_CLIENT_ID", "g-id"),
                ("GOOGLE_CLIENT_SECRET", "g-secret"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert!(config.github.is_some());
                assert!(config.google.is_some());
            },
        );
    }

    #[test]
    fn test_all_config_values_set() {
        with_env(
            &[
                ("JWT_SECRET", "my-jwt"),
                ("SESSION_EXPIRY_HOURS", "24"),
                ("PUBLIC_URL", "https://prod.example.com"),
                ("GITHUB_CLIENT_ID", "gh"),
                ("GITHUB_CLIENT_SECRET", "ghs"),
                ("GOOGLE_CLIENT_ID", "go"),
                ("GOOGLE_CLIENT_SECRET", "gos"),
            ],
            || {
                let config = AuthConfig::from_env();
                assert_eq!(config.jwt_secret, "my-jwt");
                assert_eq!(config.session_expiry_hours, 24);
                assert_eq!(config.public_url, "https://prod.example.com");
                assert!(config.github.is_some());
                assert!(config.google.is_some());
            },
        );
    }

    #[test]
    fn test_oauth_provider_config_clone() {
        let config = OAuthProviderConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
        };
        let cloned = config.clone();
        assert_eq!(cloned.client_id, "id");
        assert_eq!(cloned.client_secret, "secret");
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
}
