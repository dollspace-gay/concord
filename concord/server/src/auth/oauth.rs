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
