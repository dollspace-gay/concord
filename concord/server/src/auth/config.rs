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
