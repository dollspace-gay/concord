use sqlx::{SqliteConnection, SqlitePool};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::db::models::{
    CreateOAuth2AppParams, CreateOAuth2AuthParams, OAuth2AppRow, OAuth2AuthorizationRow,
};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ConsumedAuthorizationCode {
    pub id: String,
    pub user_id: String,
    pub server_id: Option<String>,
    pub scopes: String,
}

pub struct IssueAuthorizationCode<'a> {
    pub id: &'a str,
    pub code_hash: &'a str,
    pub app_id: &'a str,
    pub user_id: &'a str,
    pub server_id: Option<&'a str>,
    pub redirect_uri: &'a str,
    pub scopes: &'a str,
    pub code_challenge: &'a str,
    pub expires_at: &'a str,
}

pub async fn issue_authorization_code(
    pool: &SqlitePool,
    code: &IssueAuthorizationCode<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO oauth2_codes
         (id,code_hash,app_id,user_id,server_id,redirect_uri,scopes,
          code_challenge,code_challenge_method,expires_at)
         VALUES(?,?,?,?,?,?,?,?, 'S256',?)",
    )
    .bind(code.id)
    .bind(code.code_hash)
    .bind(code.app_id)
    .bind(code.user_id)
    .bind(code.server_id)
    .bind(code.redirect_uri)
    .bind(code.scopes)
    .bind(code.code_challenge)
    .bind(code.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Consumes an authorization code only when the client, exact redirect URI,
/// PKCE verifier, expiry, and one-use state all match.
pub async fn consume_authorization_code(
    connection: &mut SqliteConnection,
    code_hash: &str,
    app_id: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<Option<ConsumedAuthorizationCode>, sqlx::Error> {
    if !(43..=128).contains(&verifier.len())
        || !verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Ok(None);
    }
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    sqlx::query_as(
        "UPDATE oauth2_codes SET consumed_at=datetime('now')
         WHERE code_hash=? AND app_id=? AND redirect_uri=?
           AND code_challenge=? AND code_challenge_method='S256'
           AND consumed_at IS NULL AND expires_at>datetime('now')
           AND EXISTS(SELECT 1 FROM oauth2_apps a WHERE a.id=oauth2_codes.app_id
                      AND a.credential_state='active' AND a.revoked_at IS NULL)
           AND EXISTS(SELECT 1 FROM users u WHERE u.id=oauth2_codes.user_id
                      AND u.disabled_at IS NULL)
           AND (server_id IS NULL OR EXISTS(
               SELECT 1 FROM server_members m
               WHERE m.server_id=oauth2_codes.server_id AND m.user_id=oauth2_codes.user_id
               AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=m.server_id
                              AND b.user_id=m.user_id)))
         RETURNING id,user_id,server_id,scopes",
    )
    .bind(code_hash)
    .bind(app_id)
    .bind(redirect_uri)
    .bind(challenge)
    .fetch_optional(&mut *connection)
    .await
}

pub async fn create_app(
    pool: &SqlitePool,
    p: &CreateOAuth2AppParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO oauth2_apps
         (id,name,description,icon_url,owner_id,client_secret,redirect_uris,scopes,is_public,
          client_type,secret_credential_id,client_secret_hash,credential_state)
         VALUES (?,?,?,?,?,'',?,?,?,?,?,?,'active')",
    )
    .bind(p.id)
    .bind(p.name)
    .bind(p.description)
    .bind(p.icon_url)
    .bind(p.owner_id)
    .bind(p.redirect_uris)
    .bind(p.scopes)
    .bind(i64::from(p.client_type == "public"))
    .bind(p.client_type)
    .bind(format!("oauth-client:{}", p.id))
    .bind((p.client_type == "confidential").then_some(p.client_secret))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_app(pool: &SqlitePool, app_id: &str) -> Result<Option<OAuth2AppRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuth2AppRow>("SELECT * FROM oauth2_apps WHERE id = ?")
        .bind(app_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_apps_by_owner(
    pool: &SqlitePool,
    owner_id: &str,
) -> Result<Vec<OAuth2AppRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuth2AppRow>(
        "SELECT * FROM oauth2_apps WHERE owner_id = ? ORDER BY created_at DESC",
    )
    .bind(owner_id)
    .fetch_all(pool)
    .await
}

pub async fn update_app(
    pool: &SqlitePool,
    app_id: &str,
    name: &str,
    description: &str,
    icon_url: Option<&str>,
    redirect_uris: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE oauth2_apps SET name = ?, description = ?, icon_url = ?, redirect_uris = ? WHERE id = ?"
    )
    .bind(name)
    .bind(description)
    .bind(icon_url)
    .bind(redirect_uris)
    .bind(app_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_app(pool: &SqlitePool, app_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM oauth2_apps WHERE id = ?")
        .bind(app_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_authorization(
    pool: &SqlitePool,
    p: &CreateOAuth2AuthParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR REPLACE INTO oauth2_authorizations
         (id, app_id, user_id, server_id, scopes, access_token, refresh_token, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(p.id)
    .bind(p.app_id)
    .bind(p.user_id)
    .bind(p.server_id)
    .bind(p.scopes)
    .bind(p.access_token)
    .bind(p.refresh_token)
    .bind(p.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_authorization_by_token(
    pool: &SqlitePool,
    access_token: &str,
) -> Result<Option<OAuth2AuthorizationRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuth2AuthorizationRow>(
        "SELECT * FROM oauth2_authorizations WHERE access_token = ?",
    )
    .bind(access_token)
    .fetch_optional(pool)
    .await
}

pub async fn list_user_authorizations(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<OAuth2AuthorizationRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuth2AuthorizationRow>(
        "SELECT * FROM oauth2_authorizations WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn revoke_authorization(pool: &SqlitePool, auth_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM oauth2_authorizations WHERE id = ?")
        .bind(auth_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn revoke_all_for_app(
    pool: &SqlitePool,
    app_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM oauth2_authorizations WHERE app_id = ? AND user_id = ?")
        .bind(app_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
