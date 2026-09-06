use super::*;

#[tokio::test]
async fn typed_search_supports_filter_only_paging_and_tracks_edits_and_deletes() {
    let (pool, service) = fixture().await;
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
         ('first','server','public','owner','Alice','original phrase','2026-09-01T12:00:00Z'), \
         ('second','server','public','owner','Alice','second phrase','2026-09-02T12:00:00Z'), \
         ('third','server','public','owner','Alice','third phrase','2026-09-03T12:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;

    let (page, total, _) = service
        .search_messages(
            &auth,
            &actor,
            MessageSearch {
                server_id: "server",
                query: None,
                requested_channel_id: Some("public"),
                sender: Some("alice"),
                has_attachment: false,
                has_link: false,
                before: None,
                after: None,
                after_inclusive: false,
                limit: 1,
                offset: 1,
                cursor_created_at: None,
                cursor_message_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 3);
    assert_eq!(
        page.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["second"]
    );

    sqlx::query("UPDATE messages SET content='revised phrase' WHERE id='first'")
        .execute(&pool)
        .await
        .unwrap();
    for (query, expected) in [("original phrase", 0), ("revised phrase", 1)] {
        let (_, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some(query),
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
        assert_eq!(total, expected, "unexpected count for {query:?}");
    }

    sqlx::query("UPDATE messages SET deleted_at='2026-09-04T00:00:00Z' WHERE id='first'")
        .execute(&pool)
        .await
        .unwrap();
    let (rows, total, _) = service
        .search_messages(
            &auth,
            &actor,
            MessageSearch {
                server_id: "server",
                query: Some("revised phrase"),
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
    assert_eq!(total, 0);
    assert!(rows.is_empty());
}

#[tokio::test]
async fn search_keyset_remains_stable_across_concurrent_insert_and_delete() {
    let (pool, service) = fixture().await;
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
         ('a','server','public','owner','Alice','needle','2026-09-01T00:00:00Z'), \
         ('b','server','public','owner','Alice','needle','2026-09-01T20:00:00Z'), \
         ('c','server','public','owner','Alice','needle','2026-09-01T22:00:00-02:00'), \
         ('d','server','public','owner','Alice','needle','2026-09-02 00:00:00')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;
    let (first, total, _) = service
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
                limit: 2,
                offset: 0,
                cursor_created_at: None,
                cursor_message_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 4);
    assert_eq!(
        first.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["d", "c"]
    );

    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) \
         VALUES('e','server','public','owner','Alice','needle','2026-09-05T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE messages SET deleted_at='2026-09-06T00:00:00Z' WHERE id='c'")
        .execute(&pool)
        .await
        .unwrap();
    let (second, total, _) = service
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
                limit: 2,
                offset: 0,
                cursor_created_at: Some(&first[1].created_at),
                cursor_message_id: Some(&first[1].id),
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 4);
    assert_eq!(
        second.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
}
