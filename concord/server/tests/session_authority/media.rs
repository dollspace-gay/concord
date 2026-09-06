use super::*;

#[tokio::test]
async fn stalled_multipart_upload_times_out_and_releases_its_reservation() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user-1')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','user-1','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let router = app(pool.clone(), auth).await;
    let prefix = b"--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"slow.txt\"\r\nContent-Type: text/plain\r\n\r\npartial";
    let body = futures_util::stream::once(async move {
        Ok::<Bytes, std::io::Error>(Bytes::from_static(prefix))
    })
    .chain(futures_util::stream::pending());

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/uploads?conversation_id={conversation}"))
                .header("cookie", format!("concord_session={token}"))
                .header(
                    "content-type",
                    "multipart/form-data; boundary=test-boundary",
                )
                .body(Body::from_stream(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let row: (String, i64) = sqlx::query_as(
        "SELECT media_state,reserved_bytes FROM attachments WHERE original_filename='slow.txt'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, ("failed".into(), 0));
}

#[tokio::test]
async fn managed_media_claims_revalidate_and_retire_replaced_assets() {
    let pool = database().await;
    for (id, name) in [
        ("owner", "owner"),
        ("manager", "manager"),
        ("member", "member"),
    ] {
        user(&pool, id, name).await;
    }
    sqlx::query("INSERT INTO servers(id,name,owner_id,icon_url) VALUES('server','Server','owner','/api/uploads/00000000-0000-4000-8000-000000000001')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role,avatar_url) VALUES('server','owner','owner',NULL),('server','manager','member',NULL),('server','member','member','/api/uploads/00000000-0000-4000-8000-000000000003')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('manage','server','Manager',?,0)")
        .bind(concord_server::engine::permissions::Permissions::MANAGE_SERVER.bits() as i64)
        .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO user_roles(server_id,user_id,role_id) VALUES('server','manager','manage')",
    )
    .execute(&pool)
    .await
    .unwrap();
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000001",
        "owner",
        "server_avatar",
        Some("server"),
        None,
        "attached",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000002",
        "manager",
        "server_avatar",
        Some("server"),
        None,
        "ready",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000003",
        "member",
        "server_member_avatar",
        Some("server"),
        Some("member"),
        "attached",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000004",
        "member",
        "server_member_avatar",
        Some("server"),
        Some("member"),
        "ready",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000005",
        "member",
        "server_avatar",
        Some("server"),
        None,
        "ready",
    )
    .await;

    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (manager_token, _) = auth.issue_web_session("manager").await.unwrap();
    let (member_token, _) = auth.issue_web_session("member").await.unwrap();
    let router = app(pool.clone(), auth).await;

    sqlx::query("INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('manager-ban','server','manager','owner')")
        .execute(&pool).await.unwrap();
    assert_eq!(
        patch_json(
            &router,
            &manager_token,
            "/api/servers/server/media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000002"}"#
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let untouched: String = sqlx::query_scalar(
        "SELECT media_state FROM attachments WHERE id='00000000-0000-4000-8000-000000000002'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(untouched, "ready");
    sqlx::query("DELETE FROM bans WHERE id='manager-ban'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        patch_json(
            &router,
            &manager_token,
            "/api/servers/server/media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000002"}"#
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let icon_states: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,media_state FROM attachments WHERE id IN ('00000000-0000-4000-8000-000000000001','00000000-0000-4000-8000-000000000002') ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        icon_states,
        vec![
            (
                "00000000-0000-4000-8000-000000000001".into(),
                "deleting".into()
            ),
            (
                "00000000-0000-4000-8000-000000000002".into(),
                "attached".into()
            )
        ]
    );

    assert_eq!(
        patch_json(
            &router,
            &member_token,
            "/api/servers/server/member-media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000005"}"#
        )
        .await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        patch_json(
            &router,
            &member_token,
            "/api/servers/server/member-media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000004"}"#
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let member_states:Vec<(String,String)>=sqlx::query_as("SELECT id,media_state FROM attachments WHERE id IN ('00000000-0000-4000-8000-000000000003','00000000-0000-4000-8000-000000000004') ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        member_states,
        vec![
            (
                "00000000-0000-4000-8000-000000000003".into(),
                "deleting".into()
            ),
            (
                "00000000-0000-4000-8000-000000000004".into(),
                "attached".into()
            )
        ]
    );
}

#[tokio::test]
async fn media_routes_enforce_direct_and_private_thread_authorization() {
    let pool = database().await;
    for (id, name) in [("alice", "alice"), ("bob", "bob"), ("eve", "eve")] {
        user(&pool, id, name).await;
    }
    sqlx::query("INSERT INTO conversations(id,kind) VALUES('dm','direct')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO conversation_participants(conversation_id,user_id) VALUES('dm','alice'),('dm','bob')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO direct_conversation_pairs(conversation_id,lower_user_id,upper_user_id) VALUES('dm','alice','bob')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO direct_message_preferences(user_id,allow_from) VALUES('alice','everyone'),('bob','everyone')")
        .execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','alice')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','alice','owner'),('server','bob','member'),('server','eve','member')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind(concord_server::engine::permissions::DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('parent','server','#parent')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','private_thread','parent')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO thread_members(thread_id,user_id) VALUES('thread','bob')")
        .execute(&pool)
        .await
        .unwrap();
    let thread_conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='thread'")
            .fetch_one(&pool)
            .await
            .unwrap();

    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (alice_token, _) = auth.issue_web_session("alice").await.unwrap();
    let (bob_token, _) = auth.issue_web_session("bob").await.unwrap();
    let (eve_token, _) = auth.issue_web_session("eve").await.unwrap();
    let router = app(pool.clone(), auth).await;

    assert_eq!(
        post_upload(&router, &alice_token, "dm", "dm.txt").await,
        StatusCode::CREATED
    );
    assert_eq!(
        post_upload(&router, &eve_token, "dm", "denied.txt").await,
        StatusCode::NOT_FOUND
    );
    let dm_attachment: String =
        sqlx::query_scalar("SELECT id FROM attachments WHERE original_filename='dm.txt'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE attachments SET media_state='attached' WHERE id=?")
        .bind(&dm_attachment)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &dm_attachment).await,
        StatusCode::OK
    );
    assert_eq!(
        get_upload(&router, &eve_token, &dm_attachment).await,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE conversation_participants SET left_at=datetime('now') WHERE conversation_id='dm' AND user_id='bob'")
        .execute(&pool).await.unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &dm_attachment).await,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        post_upload(&router, &bob_token, &thread_conversation, "thread.txt").await,
        StatusCode::CREATED
    );
    assert_eq!(
        post_upload(&router, &eve_token, &thread_conversation, "hidden.txt").await,
        StatusCode::NOT_FOUND
    );
    let thread_attachment: String =
        sqlx::query_scalar("SELECT id FROM attachments WHERE original_filename='thread.txt'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE attachments SET media_state='attached' WHERE id=?")
        .bind(&thread_attachment)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &thread_attachment).await,
        StatusCode::OK
    );
    assert_eq!(
        get_upload(&router, &eve_token, &thread_attachment).await,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','bob','alice')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &thread_attachment).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn already_expired_wait_completes_immediately() {
    tokio::time::timeout(
        Duration::from_millis(100),
        concord_server::auth::authority::wait_for_expiry(Some(chrono::Utc::now().timestamp() - 1)),
    )
    .await
    .expect("already-expired credential slept instead of completing");
}
