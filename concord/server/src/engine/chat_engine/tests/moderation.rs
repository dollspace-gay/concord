use super::*;

#[tokio::test]
async fn kick_commits_membership_and_audit_together_then_evicts_subscriptions() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    engine
        .kick_member(
            moderator_session,
            "server",
            "target",
            Some("documented reason"),
        )
        .await
        .unwrap();
    let state: (i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM server_members \
             WHERE server_id='server' AND user_id='target'), \
            (SELECT count(*) FROM audit_log \
             WHERE server_id='server' AND actor_id='moderator' \
               AND action_type='member_kick' AND target_id='target')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 1));
    assert!(
        !engine
            .servers
            .get("server")
            .unwrap()
            .member_user_ids
            .contains("target")
    );
    assert!(
        !engine
            .channels
            .get("channel")
            .unwrap()
            .members
            .contains(&target_session)
    );
}

#[tokio::test]
async fn kick_rolls_back_membership_when_audit_insert_fails() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    sqlx::query(
        "CREATE TRIGGER reject_kick_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='member_kick' BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        engine
            .kick_member(moderator_session, "server", "target", None)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM server_members \
             WHERE server_id='server' AND user_id='target'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert!(
        engine
            .servers
            .get("server")
            .unwrap()
            .member_user_ids
            .contains("target")
    );
    assert!(
        engine
            .channels
            .get("channel")
            .unwrap()
            .members
            .contains(&target_session)
    );
}

#[tokio::test]
async fn bulk_delete_commits_tombstones_versions_events_outbox_and_audit() {
    let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
    insert_moderation_message(&pool, "message-1").await;
    insert_moderation_message(&pool, "message-2").await;

    engine
        .bulk_delete_messages(
            moderator_session,
            "server",
            "#general",
            vec!["message-1".into(), "message-2".into()],
        )
        .await
        .unwrap();

    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM messages WHERE id IN ('message-1','message-2') \
             AND deleted_at IS NOT NULL AND entity_version=2), \
            (SELECT count(*) FROM entity_versions WHERE entity_type='message' \
             AND entity_id IN ('message-1','message-2') AND version=2), \
            (SELECT count(*) FROM event_log e JOIN delivery_outbox o USING(event_sequence) \
             WHERE e.event_kind='message_deleted' \
             AND e.entity_id IN ('message-1','message-2')), \
            (SELECT count(*) FROM audit_log WHERE action_type='message_bulk_delete' \
             AND actor_id='moderator' AND target_id='channel')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (2, 2, 2, 1));
}

#[tokio::test]
async fn bulk_delete_rolls_back_all_tombstones_when_audit_fails() {
    let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
    insert_moderation_message(&pool, "message-1").await;
    insert_moderation_message(&pool, "message-2").await;
    sqlx::query(
        "CREATE TRIGGER reject_bulk_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='message_bulk_delete' \
         BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        engine
            .bulk_delete_messages(
                moderator_session,
                "server",
                "#general",
                vec!["message-1".into(), "message-2".into()],
            )
            .await
            .is_err()
    );
    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM messages WHERE id IN ('message-1','message-2') \
             AND deleted_at IS NULL AND entity_version=1), \
            (SELECT count(*) FROM entity_versions WHERE entity_type='message' \
             AND entity_id IN ('message-1','message-2')), \
            (SELECT count(*) FROM event_log WHERE event_kind='message_deleted' \
             AND entity_id IN ('message-1','message-2')), \
            (SELECT count(*) FROM audit_log WHERE action_type='message_bulk_delete')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (2, 0, 0, 0));
}

#[tokio::test]
async fn ban_commits_membership_ban_and_audit_then_evicts_subscriptions() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    engine
        .ban_member(
            moderator_session,
            "server",
            "target",
            Some("documented reason"),
            0,
        )
        .await
        .unwrap();
    let state: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM server_members \
             WHERE server_id='server' AND user_id='target'), \
            (SELECT count(*) FROM bans \
             WHERE server_id='server' AND user_id='target' AND banned_by='moderator'), \
            (SELECT count(*) FROM audit_log \
             WHERE action_type='member_ban' AND actor_id='moderator' AND target_id='target')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 1, 1));
    assert!(
        !engine
            .channels
            .get("channel")
            .unwrap()
            .members
            .contains(&target_session)
    );
}

#[tokio::test]
async fn ban_rolls_back_membership_and_ban_when_audit_fails() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    sqlx::query(
        "CREATE TRIGGER reject_ban_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='member_ban' BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        engine
            .ban_member(moderator_session, "server", "target", None, 0)
            .await
            .is_err()
    );
    let state: (i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM server_members \
             WHERE server_id='server' AND user_id='target'), \
            (SELECT count(*) FROM bans WHERE server_id='server' AND user_id='target')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (1, 0));
    assert!(
        engine
            .channels
            .get("channel")
            .unwrap()
            .members
            .contains(&target_session)
    );
}

#[tokio::test]
async fn timeout_rolls_back_member_state_when_audit_fails() {
    let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
    sqlx::query(
        "CREATE TRIGGER reject_timeout_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='member_timeout' BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let until = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    assert!(
        engine
            .timeout_member(moderator_session, "server", "target", Some(&until), None)
            .await
            .is_err()
    );
    let state: (Option<String>, i64) = sqlx::query_as(
        "SELECT timeout_until, \
            (SELECT count(*) FROM audit_log WHERE action_type='member_timeout') \
         FROM server_members WHERE server_id='server' AND user_id='target'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (None, 0));
}

#[tokio::test]
async fn automod_crud_commits_each_rule_change_with_audit() {
    let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
    engine
        .create_automod_rule(
            moderator_session,
            &CreateAutomodRuleRequest {
                server_id: "server",
                name: "keywords",
                rule_type: "keyword",
                config: r#"{"words":["blocked"]}"#,
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();
    let rule_id: String =
        sqlx::query_scalar("SELECT id FROM automod_rules WHERE server_id='server'")
            .fetch_one(&pool)
            .await
            .unwrap();
    engine
        .update_automod_rule(
            moderator_session,
            &UpdateAutomodRuleRequest {
                rule_id: &rule_id,
                server_id: "server",
                name: "mentions",
                enabled: false,
                config: r#"{"words":["blocked","second"]}"#,
                action_type: "flag",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();
    engine
        .delete_automod_rule(moderator_session, "server", &rule_id)
        .await
        .unwrap();

    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM automod_rules WHERE server_id='server'), \
            (SELECT count(*) FROM audit_log WHERE action_type='automod_rule_create'), \
            (SELECT count(*) FROM audit_log WHERE action_type='automod_rule_update'), \
            (SELECT count(*) FROM audit_log WHERE action_type='automod_rule_delete')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 1, 1, 1));
}

#[tokio::test]
async fn automod_create_rolls_back_rule_when_audit_fails() {
    let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
    sqlx::query(
        "CREATE TRIGGER reject_automod_create_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='automod_rule_create' \
         BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        engine
            .create_automod_rule(
                moderator_session,
                &CreateAutomodRuleRequest {
                    server_id: "server",
                    name: "keywords",
                    rule_type: "keyword",
                    config: r#"{"words":["blocked"]}"#,
                    action_type: "delete",
                    timeout_duration_seconds: None,
                },
            )
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM automod_rules WHERE server_id='server'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
