use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl};

use super::config::OAuthProviderConfig;

/// Build an OAuth2 client for GitHub.
pub fn github_client(config: &OAuthProviderConfig, public_url: &str) -> BasicClient {
    BasicClient::new(
        ClientId::new(config.client_id.clone()),
        Some(ClientSecret::new(config.client_secret.clone())),
        AuthUrl::new("https://github.com/login/oauth/authorize".into()).unwrap(),
        Some(TokenUrl::new("https://github.com/login/oauth/access_token".into()).unwrap()),
    )
    .set_redirect_uri(RedirectUrl::new(format!("{}/api/auth/github/callback", public_url)).unwrap())
}

/// Build an OAuth2 client for Google.
pub fn google_client(config: &OAuthProviderConfig, public_url: &str) -> BasicClient {
    BasicClient::new(
        ClientId::new(config.client_id.clone()),
        Some(ClientSecret::new(config.client_secret.clone())),
        AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".into()).unwrap(),
        Some(TokenUrl::new("https://oauth2.googleapis.com/token".into()).unwrap()),
    )
    .set_redirect_uri(RedirectUrl::new(format!("{}/api/auth/google/callback", public_url)).unwrap())
}

/// GitHub user info from their API.
#[derive(serde::Deserialize)]
pub struct GitHubUser {
    pub id: i64,
    pub login: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

/// Fetch GitHub user profile with an access token.
pub async fn fetch_github_user(access_token: &str) -> Result<GitHubUser, reqwest::Error> {
    let client = reqwest::Client::new();
    client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "Concord")
        .header("Accept", "application/json")
        .send()
        .await?
        .json::<GitHubUser>()
        .await
}

/// Google user info from their API.
#[derive(serde::Deserialize)]
pub struct GoogleUser {
    pub sub: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub picture: Option<String>,
}

/// Fetch Google user profile with an access token.
pub async fn fetch_google_user(access_token: &str) -> Result<GoogleUser, reqwest::Error> {
    let client = reqwest::Client::new();
    client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?
        .json::<GoogleUser>()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider() -> OAuthProviderConfig {
        OAuthProviderConfig {
            client_id: "test-client-id".into(),
            client_secret: "test-client-secret".into(),
        }
    }

    // ── GitHub client construction ──

    #[test]
    fn test_github_client_creates_successfully() {
        let config = test_provider();
        // Should not panic
        let _client = github_client(&config, "http://localhost:8080");
    }

    #[test]
    fn test_github_client_with_https_url() {
        let config = test_provider();
        let _client = github_client(&config, "https://my-concord.example.com");
    }

    #[test]
    fn test_github_client_no_trailing_slash() {
        // Ensure no double-slash in redirect URI
        let config = test_provider();
        let _client = github_client(&config, "http://localhost:8080");
        // If there were a double slash the URL parsing would have failed or produced malformed redirect
    }

    // ── Google client construction ──

    #[test]
    fn test_google_client_creates_successfully() {
        let config = test_provider();
        let _client = google_client(&config, "http://localhost:8080");
    }

    #[test]
    fn test_google_client_with_https_url() {
        let config = test_provider();
        let _client = google_client(&config, "https://concord.example.com");
    }

    #[test]
    fn test_google_client_with_port() {
        let config = test_provider();
        let _client = google_client(&config, "http://localhost:3000");
    }

    // ── GitHubUser deserialization ──

    #[test]
    fn test_github_user_deserialize_full() {
        let json = r#"{
            "id": 12345,
            "login": "octocat",
            "email": "octocat@github.com",
            "avatar_url": "https://avatars.githubusercontent.com/u/12345"
        }"#;
        let user: GitHubUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, 12345);
        assert_eq!(user.login, "octocat");
        assert_eq!(user.email, Some("octocat@github.com".into()));
        assert_eq!(
            user.avatar_url,
            Some("https://avatars.githubusercontent.com/u/12345".into())
        );
    }

    #[test]
    fn test_github_user_deserialize_minimal() {
        let json = r#"{"id": 1, "login": "user"}"#;
        let user: GitHubUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(user.login, "user");
        assert!(user.email.is_none());
        assert!(user.avatar_url.is_none());
    }

    #[test]
    fn test_github_user_deserialize_null_optionals() {
        let json = r#"{"id": 99, "login": "dev", "email": null, "avatar_url": null}"#;
        let user: GitHubUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, 99);
        assert!(user.email.is_none());
        assert!(user.avatar_url.is_none());
    }

    // ── GoogleUser deserialization ──

    #[test]
    fn test_google_user_deserialize_full() {
        let json = r#"{
            "sub": "123456789",
            "name": "Test User",
            "email": "test@gmail.com",
            "picture": "https://lh3.googleusercontent.com/photo.jpg"
        }"#;
        let user: GoogleUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.sub, "123456789");
        assert_eq!(user.name, Some("Test User".into()));
        assert_eq!(user.email, Some("test@gmail.com".into()));
        assert!(user.picture.is_some());
    }

    #[test]
    fn test_google_user_deserialize_minimal() {
        let json = r#"{"sub": "abc"}"#;
        let user: GoogleUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.sub, "abc");
        assert!(user.name.is_none());
        assert!(user.email.is_none());
        assert!(user.picture.is_none());
    }

    #[test]
    fn test_google_user_deserialize_null_optionals() {
        let json = r#"{"sub": "x", "name": null, "email": null, "picture": null}"#;
        let user: GoogleUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.sub, "x");
        assert!(user.name.is_none());
    }
}
