use super::*;

#[tokio::test]
async fn nonmember_and_active_ban_are_denied_before_default_role() {
    let (pool, service) = fixture().await;
    assert!(matches!(
        service
            .authorize_channel("outsider", "public", ChannelAction::View)
            .await,
        Err(AuthorizationError::Unavailable)
    ));
    sqlx::query(
        "INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','member','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        service
            .authorize_channel("member", "public", ChannelAction::View)
            .await,
        Err(AuthorizationError::Unavailable)
    ));
}

#[tokio::test]
async fn channel_subscription_is_not_a_private_visibility_grant() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO channels(id,server_id,name,is_private) VALUES('private','server','#private',1)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channel_members(channel_id,user_id) VALUES('private','member')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        service
            .authorize_channel("member", "private", ChannelAction::View)
            .await,
        Err(AuthorizationError::Unavailable)
    ));
    sqlx::query("INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) VALUES('private','user','member')")
        .execute(&pool).await.unwrap();
    service
        .authorize_channel("member", "private", ChannelAction::View)
        .await
        .unwrap();
}

#[tokio::test]
async fn sql_failure_is_not_replaced_with_default_permissions() {
    let (pool, service) = fixture().await;
    sqlx::query("DROP TABLE roles")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        service
            .authorize_channel("member", "public", ChannelAction::View)
            .await,
        Err(AuthorizationError::Database(_))
    ));
}

#[tokio::test]
async fn search_excludes_channels_without_history_permission() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('legacy-id','server','public','owner','owner','classified needle')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('deny-history','public','role','everyone',?)")
        .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64)
        .execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("member").await.unwrap();

    let (rows, total, stamp) = service
        .search_messages(
            &auth,
            &actor,
            MessageSearch {
                server_id: "server",
                query: Some("needle"),
                requested_channel_id: None,
                sender: None,
                has_attachment: false,
                has_link: false,
                before: None,
                after: None,
                after_inclusive: false,
                limit: 50,
                offset: 0,
                cursor_created_at: None,
                cursor_message_id: None,
            },
        )
        .await
        .unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 0);
    assert!(service.stamp_is_current(&stamp).await.unwrap());
    assert!(matches!(
        service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("needle"),
                    requested_channel_id: Some("public"),
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 50,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await,
        Err(AuthorizationError::Unavailable)
    ));
}

#[tokio::test]
async fn bot_grant_shrink_invalidates_a_held_authorization_stamp() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot','bot',1)")
        .execute(&pool)
        .await
        .unwrap();
    crate::db::queries::bots::add_bot_to_server_with_grants(
        &pool,
        "server",
        "bot",
        "owner",
        "messages commands",
    )
    .await
    .unwrap();

    // Model a response that passed authorization and is waiting at the
    // transport boundary while its bot installation grant is reduced.
    let mut connection = pool.acquire().await.unwrap();
    let held = service
        .authorization_stamp(&mut connection, "server", &["public".to_owned()])
        .await
        .unwrap();
    drop(connection);
    assert!(service.stamp_is_current(&held).await.unwrap());

    crate::db::queries::bots::add_bot_to_server_with_grants(
        &pool, "server", "bot", "owner", "messages",
    )
    .await
    .unwrap();

    assert!(!service.stamp_is_current(&held).await.unwrap());
}

#[tokio::test]
async fn typed_search_filters_share_the_authorized_count_and_page_predicate() {
    let (pool, service) = fixture().await;
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
         ('old','server','public','owner','Alice','https://old.example needle','2026-09-01T12:00:00Z'), \
         ('match','server','public','owner','Alice','https://example.test needle','2026-09-03T12:00:00Z'), \
         ('other','server','public','member','Member','https://example.test needle','2026-09-03T13:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO attachments(id,uploader_id,message_id,filename,original_filename,content_type,file_size) \
         VALUES('attachment','owner','match','proof.txt','proof.txt','text/plain',5)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;
    let (rows, total, _) = service
        .search_messages(
            &auth,
            &actor,
            MessageSearch {
                server_id: "server",
                query: Some("needle"),
                requested_channel_id: Some("public"),
                sender: Some("alice"),
                has_attachment: true,
                has_link: true,
                before: Some("2026-09-04T00:00:00Z"),
                after: Some("2026-09-02T23:59:59Z"),
                after_inclusive: false,
                limit: 1,
                offset: 0,
                cursor_created_at: None,
                cursor_message_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "match");
}

#[tokio::test]
async fn search_authorized_channel_set_exceeds_sqlite_variable_limit() {
    let (pool, service) = fixture().await;
    for index in 0..1_005 {
        let channel = format!("many-{index}");
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES(?,'server',?)")
            .bind(&channel)
            .bind(format!("#many-{index}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) \
             VALUES(?,'server',?,'owner','Alice','needle',?)",
        )
        .bind(format!("message-{index}"))
        .bind(&channel)
        .bind(format!("2026-09-01T00:{:02}:{:02}Z", (index / 60) % 60, index % 60))
        .execute(&pool)
        .await
        .unwrap();
    }
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;
    let (rows, total, _) = service
        .search_messages(
            &auth,
            &actor,
            MessageSearch {
                server_id: "server",
                query: Some("needle"),
                requested_channel_id: None,
                sender: None,
                has_attachment: false,
                has_link: false,
                before: None,
                after: None,
                after_inclusive: false,
                limit: 50,
                offset: 0,
                cursor_created_at: None,
                cursor_message_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 1_005);
    assert_eq!(rows.len(), 50);
    assert!(rows.windows(2).all(|pair| {
        (pair[0].created_at.as_str(), pair[0].id.as_str())
            > (pair[1].created_at.as_str(), pair[1].id.as_str())
    }));
}
