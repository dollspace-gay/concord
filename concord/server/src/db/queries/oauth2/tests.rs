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
        client_type: "confidential",
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
    assert!(a.client_secret.is_empty());
    let hardened: (String, String, String) = sqlx::query_as(
        "SELECT client_type,client_secret_hash,credential_state FROM oauth2_apps WHERE id='app1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        hardened,
        ("confidential".into(), "secret123".into(), "active".into())
    );
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
            client_type: "confidential",
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

#[tokio::test]
async fn authorization_code_requires_exact_redirect_pkce_and_one_use() {
    let pool = setup_db().await;
    setup_user(&pool).await;
    create_app(&pool, &app_params("app1")).await.unwrap();
    let verifier = "verifier-with-at-least-forty-three-characters-12345";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    issue_authorization_code(
        &pool,
        &IssueAuthorizationCode {
            id: "code1",
            code_hash: "hashed-code",
            app_id: "app1",
            user_id: "u1",
            server_id: None,
            redirect_uri: "https://client.example/callback",
            scopes: "identify",
            code_challenge: &challenge,
            expires_at: "2999-01-01 00:00:00",
        },
    )
    .await
    .unwrap();

    let mut transaction = pool.begin().await.unwrap();
    assert!(
        consume_authorization_code(
            &mut transaction,
            "hashed-code",
            "app1",
            "https://client.example/callback/",
            verifier
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        consume_authorization_code(
            &mut transaction,
            "hashed-code",
            "app1",
            "https://client.example/callback",
            "wrong-verifier"
        )
        .await
        .unwrap()
        .is_none()
    );
    let consumed = consume_authorization_code(
        &mut transaction,
        "hashed-code",
        "app1",
        "https://client.example/callback",
        verifier,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(consumed.user_id, "u1");
    transaction.rollback().await.unwrap();

    let mut transaction = pool.begin().await.unwrap();
    let consumed = consume_authorization_code(
        &mut transaction,
        "hashed-code",
        "app1",
        "https://client.example/callback",
        verifier,
    )
    .await
    .unwrap();
    assert!(consumed.is_some());
    transaction.commit().await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    assert!(
        consume_authorization_code(
            &mut transaction,
            "hashed-code",
            "app1",
            "https://client.example/callback",
            verifier
        )
        .await
        .unwrap()
        .is_none()
    );
    transaction.rollback().await.unwrap();
}
