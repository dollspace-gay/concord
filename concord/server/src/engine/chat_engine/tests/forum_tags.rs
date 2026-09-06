use super::*;

#[tokio::test]
async fn forum_tags_enforce_parent_ownership_and_moderated_authority() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    insert_moderation_message(&pool, "thread-parent").await;
    engine
        .create_thread(
            target_session,
            "server",
            "#general",
            "topic",
            "thread-parent",
            false,
        )
        .await
        .unwrap();
    let thread_id: String =
        sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT thread_creator_user_id FROM channels WHERE id=?"
        )
        .bind(&thread_id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .as_deref(),
        Some("target")
    );
    engine
        .create_forum_tag(
            moderator_session,
            "server",
            "#general",
            "ordinary",
            None,
            false,
        )
        .await
        .unwrap();
    engine
        .create_forum_tag(
            moderator_session,
            "server",
            "#general",
            "moderated",
            None,
            true,
        )
        .await
        .unwrap();
    let ordinary: String = sqlx::query_scalar(
        "SELECT id FROM forum_tags WHERE channel_id='channel' AND name='ordinary'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let moderated: String = sqlx::query_scalar(
        "SELECT id FROM forum_tags WHERE channel_id='channel' AND name='moderated'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    engine
        .set_thread_tags(target_session, "server", &thread_id, vec![ordinary.clone()])
        .await
        .unwrap();
    assert!(
        engine
            .set_thread_tags(
                target_session,
                "server",
                &thread_id,
                vec![moderated.clone()],
            )
            .await
            .is_err()
    );
    let selected: Vec<String> =
        sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id=?")
            .bind(&thread_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(selected, vec![ordinary]);
    engine
        .set_thread_tags(
            moderator_session,
            "server",
            &thread_id,
            vec![moderated.clone()],
        )
        .await
        .unwrap();
    assert!(
        engine
            .set_thread_tags(target_session, "server", &thread_id, Vec::new())
            .await
            .is_err()
    );
    engine
        .delete_forum_tag(moderator_session, "server", "#general", &moderated)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM thread_tags WHERE thread_id=?")
            .bind(&thread_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let durable: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT thread_tags_version, \
            (SELECT count(*) FROM event_log WHERE entity_type='thread_tags' AND entity_id=?), \
            (SELECT count(*) FROM delivery_outbox o JOIN event_log e USING(event_sequence) \
             WHERE e.entity_type='thread_tags' AND e.entity_id=?), \
            (SELECT count(*) FROM audit_log WHERE action_type='thread_tags_update' \
             AND target_id=?) \
         FROM channels WHERE id=?",
    )
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(durable, (4, 3, 3, 3));
    let projection = engine.channels.get(&thread_id).unwrap();
    assert_eq!(projection.thread_tags_version, 4);
    assert!(projection.thread_tag_ids.is_empty());
}

#[tokio::test]
async fn thread_tag_audit_failure_rolls_back_selection_version_and_event() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    insert_moderation_message(&pool, "thread-parent").await;
    engine
        .create_thread(
            target_session,
            "server",
            "#general",
            "topic",
            "thread-parent",
            false,
        )
        .await
        .unwrap();
    let thread_id: String =
        sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    engine
        .create_forum_tag(
            moderator_session,
            "server",
            "#general",
            "ordinary",
            None,
            false,
        )
        .await
        .unwrap();
    let tag_id: String = sqlx::query_scalar("SELECT id FROM forum_tags WHERE channel_id='channel'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_thread_tag_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='thread_tags_update' \
         BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        engine
            .set_thread_tags(target_session, "server", &thread_id, vec![tag_id])
            .await
            .is_err()
    );
    let state: (i64, i64, i64) = sqlx::query_as(
        "SELECT thread_tags_version, \
            (SELECT count(*) FROM thread_tags WHERE thread_id=?), \
            (SELECT count(*) FROM event_log WHERE entity_type='thread_tags' AND entity_id=?) \
         FROM channels WHERE id=?",
    )
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (1, 0, 0));
    assert_eq!(
        engine.channels.get(&thread_id).unwrap().thread_tags_version,
        1
    );
}

#[tokio::test]
async fn legacy_unknown_thread_creator_has_no_guessed_tag_authority() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    insert_moderation_message(&pool, "thread-parent").await;
    engine
        .create_thread(
            target_session,
            "server",
            "#general",
            "topic",
            "thread-parent",
            false,
        )
        .await
        .unwrap();
    let thread_id: String =
        sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE channels SET thread_creator_user_id=NULL WHERE id=?")
        .bind(&thread_id)
        .execute(&pool)
        .await
        .unwrap();
    engine
        .create_forum_tag(
            moderator_session,
            "server",
            "#general",
            "ordinary",
            None,
            false,
        )
        .await
        .unwrap();
    let tag_id: String = sqlx::query_scalar("SELECT id FROM forum_tags WHERE channel_id='channel'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        engine
            .set_thread_tags(target_session, "server", &thread_id, vec![tag_id.clone()])
            .await
            .is_err()
    );
    engine
        .set_thread_tags(moderator_session, "server", &thread_id, vec![tag_id])
        .await
        .unwrap();
}
