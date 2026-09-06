use super::*;

#[tokio::test]
async fn consent_rechecks_app_target_and_browser_credential_in_its_write() {
    let (router, pool, cookie) = fixture().await;
    insert_consent(&pool, "app-race").await;
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let queued = tokio::spawn({
        let cookie = cookie.clone();
        async move { post_consent(router, &cookie, "app-race").await }
    });
    tokio::task::yield_now().await;
    assert!(!queued.is_finished());
    sqlx::query("UPDATE oauth2_apps SET revoked_at=datetime('now') WHERE id='client'")
        .execute(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("COMMIT").execute(&mut *blocker).await.unwrap();
    assert_eq!(queued.await.unwrap(), StatusCode::BAD_REQUEST);

    let (router, pool, cookie) = fixture().await;
    insert_consent(&pool, "target-race").await;
    sqlx::query("DELETE FROM server_members WHERE server_id='server' AND user_id='user'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        post_consent(router, &cookie, "target-race").await,
        StatusCode::BAD_REQUEST
    );

    let (router, pool, cookie) = fixture().await;
    insert_consent(&pool, "credential-race").await;
    sqlx::query(
        "UPDATE auth_credentials SET revoked_at=unixepoch(),version=version+1
         WHERE user_id='user' AND kind='web_session'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        post_consent(router, &cookie, "credential-race").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn refresh_reuse_storage_failure_is_503_and_rolls_back_revocation() {
    let (router, pool, cookie) = fixture().await;
    let verifier = "f".repeat(43);
    let code = authorize(&router, &cookie, &verifier).await;
    let (status, first) = token_request(
        &router,
        form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "https://client.example/callback"),
            ("code_verifier", &verifier),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let old_refresh = first["refresh_token"].as_str().unwrap();
    let refresh_form = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", old_refresh),
    ]);
    let (status, second) = token_request(&router, refresh_form.clone()).await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query(
        "CREATE TRIGGER fail_reuse_revocation BEFORE UPDATE OF reuse_detected_at ON oauth2_tokens
         WHEN NEW.reuse_detected_at IS NOT NULL BEGIN SELECT RAISE(ABORT,'injected'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        token_request(&router, refresh_form).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let state: String = sqlx::query_scalar("SELECT state FROM oauth2_grants")
        .fetch_one(&pool)
        .await
        .unwrap();
    let reuse_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM oauth2_tokens WHERE reuse_detected_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "active");
    assert_eq!(reuse_count, 0);
    let response = router
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", second["access_token"].as_str().unwrap()),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn pkce_code_refresh_reuse_and_user_revocation_are_enforced() {
    let (router, pool, cookie) = fixture().await;
    let verifier = "a".repeat(43);
    let code = authorize(&router, &cookie, &verifier).await;
    let exchange = form(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", "https://client.example/callback"),
        ("code_verifier", &verifier),
    ]);
    let (status, first) = token_request(&router, exchange.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["token_type"], "Bearer");
    assert_eq!(
        token_request(&router, exchange).await.0,
        StatusCode::BAD_REQUEST
    );

    let access = first["access_token"].as_str().unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/servers")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Re-consent replaces the grant and revokes every prior token pair, so a
    // narrower or later grant cannot reactivate an older credential.
    let replacement_code = authorize_with_scope(&router, &cookie, &verifier, "identify").await;
    let (replacement_status, narrowed) = token_request(
        &router,
        form(&[
            ("grant_type", "authorization_code"),
            ("code", &replacement_code),
            ("redirect_uri", "https://client.example/callback"),
            ("code_verifier", &verifier),
        ]),
    )
    .await;
    assert_eq!(replacement_status, StatusCode::OK);
    let superseded = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(superseded.status(), StatusCode::UNAUTHORIZED);

    let narrowed_access = narrowed["access_token"].as_str().unwrap();
    let narrowed_servers = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/servers")
                .header(header::AUTHORIZATION, format!("Bearer {narrowed_access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(narrowed_servers.status(), StatusCode::FORBIDDEN);

    let regrant_code = authorize(&router, &cookie, &verifier).await;
    let (regrant_status, first) = token_request(
        &router,
        form(&[
            ("grant_type", "authorization_code"),
            ("code", &regrant_code),
            ("redirect_uri", "https://client.example/callback"),
            ("code_verifier", &verifier),
        ]),
    )
    .await;
    assert_eq!(regrant_status, StatusCode::OK);
    let still_superseded = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(still_superseded.status(), StatusCode::UNAUTHORIZED);
    let narrowed_after_regrant = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {narrowed_access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(narrowed_after_regrant.status(), StatusCode::UNAUTHORIZED);

    let old_refresh = first["refresh_token"].as_str().unwrap();
    let refresh_form = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", old_refresh),
    ]);
    let (status, second) = token_request(&router, refresh_form.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let new_access = second["access_token"].as_str().unwrap();
    assert_eq!(
        token_request(&router, refresh_form).await.0,
        StatusCode::BAD_REQUEST
    );
    let rejected = router
        .clone()
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {new_access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    let reuse_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM oauth2_tokens WHERE reuse_detected_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(reuse_count >= 2);

    let code = authorize(&router, &cookie, &verifier).await;
    let (_, third) = token_request(
        &router,
        form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "https://client.example/callback"),
            ("code_verifier", &verifier),
        ]),
    )
    .await;
    let grant: String =
        sqlx::query_scalar("SELECT id FROM oauth2_grants WHERE app_id='client' AND user_id='user'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let revoked = router
        .clone()
        .oneshot(
            Request::post(format!("/api/oauth/grants/{grant}/revoke"))
                .header(header::COOKIE, format!("concord_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let response = router
        .oneshot(
            Request::get("/api/oauth/userinfo")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", third["access_token"].as_str().unwrap()),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
