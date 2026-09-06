use super::*;

#[tokio::test]
async fn search_continuation_binds_query_and_restarts_after_authority_change() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('server','owner','owner'),('server','member','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES \
         ('everyone','server','@everyone',?,1)",
    )
    .bind((Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY).bits() as i64)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channels(id,server_id,name) VALUES \
         ('public','server','#public'),('private','server','#private')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
         ('public-old','server','public','owner','owner','needle','2026-09-01T00:00:00Z'), \
         ('private-new','server','private','owner','owner','needle','2026-09-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;
    let engine = ChatEngine::new(pool.clone(), auth, "search-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let first = engine
        .search_messages(
            &actor,
            SearchMessagesRequest {
                server_id: "server",
                query: "needle",
                channel_name: None,
                limit: 1,
                offset: 0,
                continuation: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(first.results[0].id, "private-new");
    let continuation = first.next_continuation.unwrap();
    assert!(matches!(
        engine
            .search_messages(
                &actor,
                SearchMessagesRequest {
                    server_id: "server",
                    query: "different query",
                    channel_name: None,
                    limit: 1,
                    offset: 0,
                    continuation: Some(&continuation),
                },
            )
            .await,
        Err(SearchError::InvalidContinuation)
    ));
    let second_credential = engine
        .auth
        .get()
        .unwrap()
        .issue_web_session("member")
        .await
        .unwrap()
        .1;
    assert!(matches!(
        engine
            .search_messages(
                &second_credential,
                SearchMessagesRequest {
                    server_id: "server",
                    query: "needle",
                    channel_name: None,
                    limit: 1,
                    offset: 0,
                    continuation: Some(&continuation),
                },
            )
            .await,
        Err(SearchError::InvalidContinuation)
    ));

    sqlx::query(
        "INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) \
         VALUES('deny-private','private','role','everyone',?)",
    )
    .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64)
    .execute(&pool)
    .await
    .unwrap();
    let restarted = engine
        .search_messages(
            &actor,
            SearchMessagesRequest {
                server_id: "server",
                query: "needle",
                channel_name: None,
                limit: 1,
                offset: 0,
                continuation: Some(&continuation),
            },
        )
        .await
        .unwrap();
    assert!(restarted.restarted);
    assert_eq!(restarted.offset, 0);
    assert_eq!(restarted.total_count, 1);
    assert_eq!(restarted.results[0].id, "public-old");
}

#[test]
fn typed_search_parser_preserves_unicode_phrases_and_filters() {
    let parsed = parse_search_query(
        "\"café au lait\" from:Laurelai in:#general has:attachment has:link before:2026-09-05 after:2026-09-01",
    )
    .unwrap();
    assert_eq!(parsed.text.as_deref(), Some("café au lait"));
    assert_eq!(parsed.sender.as_deref(), Some("Laurelai"));
    assert_eq!(parsed.channel.as_deref(), Some("#general"));
    assert!(parsed.has_attachment && parsed.has_link);
    assert_eq!(parsed.before.as_deref(), Some("2026-09-05T00:00:00+00:00"));
    assert_eq!(parsed.after.as_deref(), Some("2026-09-02T00:00:00+00:00"));
}

#[test]
fn typed_search_parser_accepts_each_filter_without_text() {
    let cases = [
        ("from:Alice", "sender"),
        ("in:#general", "channel"),
        ("has:attachment", "attachment"),
        ("has:link", "link"),
        ("before:2026-09-05", "before"),
        ("after:2026-09-01", "after"),
    ];
    for (query, expected) in cases {
        let parsed = parse_search_query(query)
            .unwrap_or_else(|error| panic!("rejected {expected} filter {query:?}: {error}"));
        assert!(parsed.text.is_none(), "filter parsed as text: {query:?}");
    }
}

#[test]
fn typed_search_parser_rejects_invalid_filters_and_unicode_control_input() {
    for query in [
        "has:executable",
        "before:yesterday",
        "from:",
        "from:alice from:bob",
        "in:#one in:#two",
        "before:2026-09-01 before:2026-09-02",
        "after:2026-09-01 after:2026-09-02",
        "\"unfinished",
        "hello\nworld",
        "",
    ] {
        assert!(parse_search_query(query).is_err(), "accepted {query:?}");
    }
    assert!(parse_search_query(&"x".repeat(1_025)).is_err());
}
