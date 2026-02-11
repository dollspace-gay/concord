use sqlx::SqlitePool;

use crate::db::models::{
    CreateOAuth2AppParams, CreateOAuth2AuthParams, OAuth2AppRow, OAuth2AuthorizationRow,
};

pub async fn create_app(
    pool: &SqlitePool,
    p: &CreateOAuth2AppParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO oauth2_apps (id, name, description, icon_url, owner_id, client_secret, redirect_uris, scopes)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(p.id)
    .bind(p.name)
    .bind(p.description)
    .bind(p.icon_url)
    .bind(p.owner_id)
    .bind(p.client_secret)
    .bind(p.redirect_uris)
    .bind(p.scopes)
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
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_user(pool: &SqlitePool) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: "u1",
                username: "alice",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u1",
                provider: "github",
                provider_id: "gh-u1",
            },
        )
        .await
        .unwrap();
    }

    fn app_params<'a>(id: &'a str) -> CreateOAuth2AppParams<'a> {
        CreateOAuth2AppParams {
            id,
            name: "Test App",
            description: "A test app",
            icon_url: None,
            owner_id: "u1",
            client_secret: "secret123",
            redirect_uris: "https://example.com/callback",
            scopes: "messages.read servers.read",
        }
    }

    #[tokio::test]
    async fn test_create_and_get_app() {
        let pool = setup_db().await;
        setup_user(&pool).await;

        create_app(&pool, &app_params("app1")).await.unwrap();

        let app = get_app(&pool, "app1").await.unwrap();
        assert!(app.is_some());
        let a = app.unwrap();
        assert_eq!(a.name, "Test App");
        assert_eq!(a.owner_id, "u1");
        assert_eq!(a.client_secret, "secret123");
    }

    #[tokio::test]
    async fn test_list_apps_by_owner() {
        let pool = setup_db().await;
        setup_user(&pool).await;

        create_app(&pool, &app_params("app1")).await.unwrap();
        create_app(
            &pool,
            &CreateOAuth2AppParams {
                id: "app2",
                name: "App 2",
                description: "Second",
                icon_url: None,
                owner_id: "u1",
                client_secret: "s2",
                redirect_uris: "https://x.com",
                scopes: "read",
            },
        )
        .await
        .unwrap();

        let apps = list_apps_by_owner(&pool, "u1").await.unwrap();
        assert_eq!(apps.len(), 2);
    }

    #[tokio::test]
    async fn test_update_app() {
        let pool = setup_db().await;
        setup_user(&pool).await;
        create_app(&pool, &app_params("app1")).await.unwrap();

        update_app(
            &pool,
            "app1",
            "Updated App",
            "New desc",
            Some("https://icon.png"),
            "https://new.com/cb",
        )
        .await
        .unwrap();

        let app = get_app(&pool, "app1").await.unwrap().unwrap();
        assert_eq!(app.name, "Updated App");
        assert_eq!(app.description, "New desc");
        assert_eq!(app.icon_url, Some("https://icon.png".to_string()));
    }

    #[tokio::test]
    async fn test_delete_app() {
        let pool = setup_db().await;
        setup_user(&pool).await;
        create_app(&pool, &app_params("app1")).await.unwrap();

        delete_app(&pool, "app1").await.unwrap();

        let app = get_app(&pool, "app1").await.unwrap();
        assert!(app.is_none());
    }

    #[tokio::test]
    async fn test_create_and_get_authorization() {
        let pool = setup_db().await;
        setup_user(&pool).await;
        create_app(&pool, &app_params("app1")).await.unwrap();

        create_authorization(
            &pool,
            &CreateOAuth2AuthParams {
                id: "auth1",
                app_id: "app1",
                user_id: "u1",
                server_id: None,
                scopes: "read",
                access_token: "access-tok-1",
                refresh_token: Some("refresh-tok-1"),
                expires_at: "2027-01-01T00:00:00Z",
            },
        )
        .await
        .unwrap();

        let auth = get_authorization_by_token(&pool, "access-tok-1")
            .await
            .unwrap();
        assert!(auth.is_some());
        let a = auth.unwrap();
        assert_eq!(a.app_id, "app1");
        assert_eq!(a.user_id, "u1");
    }

    #[tokio::test]
    async fn test_list_user_authorizations() {
        let pool = setup_db().await;
        setup_user(&pool).await;
        create_app(&pool, &app_params("app1")).await.unwrap();

        create_authorization(
            &pool,
            &CreateOAuth2AuthParams {
                id: "auth1",
                app_id: "app1",
                user_id: "u1",
                server_id: None,
                scopes: "read",
                access_token: "tok1",
                refresh_token: None,
                expires_at: "2027-01-01T00:00:00Z",
            },
        )
        .await
        .unwrap();

        let auths = list_user_authorizations(&pool, "u1").await.unwrap();
        assert_eq!(auths.len(), 1);
    }

    #[tokio::test]
    async fn test_revoke_authorization() {
        let pool = setup_db().await;
        setup_user(&pool).await;
        create_app(&pool, &app_params("app1")).await.unwrap();
        create_authorization(
            &pool,
            &CreateOAuth2AuthParams {
                id: "auth1",
                app_id: "app1",
                user_id: "u1",
                server_id: None,
                scopes: "read",
                access_token: "tok1",
                refresh_token: None,
                expires_at: "2027-01-01T00:00:00Z",
            },
        )
        .await
        .unwrap();

        revoke_authorization(&pool, "auth1").await.unwrap();

        let auth = get_authorization_by_token(&pool, "tok1").await.unwrap();
        assert!(auth.is_none());
    }

    #[tokio::test]
    async fn test_revoke_all_for_app() {
        let pool = setup_db().await;
        setup_user(&pool).await;
        create_app(&pool, &app_params("app1")).await.unwrap();

        create_authorization(
            &pool,
            &CreateOAuth2AuthParams {
                id: "auth1",
                app_id: "app1",
                user_id: "u1",
                server_id: None,
                scopes: "read",
                access_token: "tok1",
                refresh_token: None,
                expires_at: "2027-01-01T00:00:00Z",
            },
        )
        .await
        .unwrap();
        create_authorization(
            &pool,
            &CreateOAuth2AuthParams {
                id: "auth2",
                app_id: "app1",
                user_id: "u1",
                server_id: None,
                scopes: "write",
                access_token: "tok2",
                refresh_token: None,
                expires_at: "2027-01-01T00:00:00Z",
            },
        )
        .await
        .unwrap();

        revoke_all_for_app(&pool, "app1", "u1").await.unwrap();

        let auths = list_user_authorizations(&pool, "u1").await.unwrap();
        assert!(auths.is_empty());
    }
}
