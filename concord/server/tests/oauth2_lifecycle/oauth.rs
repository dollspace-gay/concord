use super::*;

#[tokio::test]
async fn public_client_uses_pkce_without_a_secret() {
    let (router, pool, cookie) = fixture().await;
    sqlx::query(
        "UPDATE oauth2_apps SET client_type='public',is_public=1,client_secret_hash=NULL
         WHERE id='client'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let verifier = "p".repeat(43);
    let code = authorize(&router, &cookie, &verifier).await;
    let (status, body) = public_token_request(
        &router,
        form(&[
            ("grant_type", "authorization_code"),
            ("client_id", "client"),
            ("code", &code),
            ("redirect_uri", "https://client.example/callback"),
            ("code_verifier", &verifier),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let access = body["access_token"].as_str().unwrap();
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

    let code = authorize(&router, &cookie, &verifier).await;
    let (status, _) = public_token_request(
        &router,
        form(&[
            ("grant_type", "authorization_code"),
            ("client_id", "client"),
            ("client_secret", "must-not-be-sent"),
            ("code", &code),
            ("redirect_uri", "https://client.example/callback"),
            ("code_verifier", &verifier),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authorization_rejects_a_normalized_but_unregistered_redirect() {
    let (router, _pool, cookie) = fixture().await;
    let verifier = "r".repeat(43);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let query = form(&[
        ("response_type", "code"),
        ("client_id", "client"),
        ("redirect_uri", "https://client.example:443/callback"),
        ("scope", "identify"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    let response = router
        .oneshot(
            Request::get(format!("/oauth/authorize?{query}"))
                .header(header::COOKIE, format!("concord_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
