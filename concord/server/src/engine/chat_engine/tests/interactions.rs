use super::*;

#[test]
fn rich_interaction_response_rejects_unsafe_media_and_duplicate_controls() {
    let unsafe_embed = crate::engine::events::RichEmbedInfo {
        title: Some("Unsafe".into()),
        description: None,
        url: None,
        color: None,
        fields: None,
        footer: None,
        image_url: Some("javascript:alert(1)".into()),
        thumbnail_url: None,
        author: None,
        timestamp: None,
    };
    assert!(validate_rich_interaction_response(Some(&[unsafe_embed]), None).is_err());

    let button = crate::engine::events::MessageComponent::Button {
        custom_id: "same".into(),
        label: "Confirm".into(),
        style: "primary".into(),
        emoji: None,
        disabled: false,
    };
    let rows = [crate::engine::events::MessageComponent::ActionRow {
        components: vec![button.clone(), button],
    }];
    assert!(validate_rich_interaction_response(None, Some(&rows)).is_err());
}

#[test]
fn rich_interaction_response_accepts_bounded_https_embed_and_controls() {
    let embed = crate::engine::events::RichEmbedInfo {
        title: Some("Result".into()),
        description: Some("Completed".into()),
        url: Some("https://example.test/result".into()),
        color: Some("#5865f2".into()),
        fields: None,
        footer: None,
        image_url: Some("https://example.test/image.png".into()),
        thumbnail_url: None,
        author: None,
        timestamp: None,
    };
    let rows = [crate::engine::events::MessageComponent::ActionRow {
        components: vec![crate::engine::events::MessageComponent::Button {
            custom_id: "confirm".into(),
            label: "Confirm".into(),
            style: "success".into(),
            emoji: None,
            disabled: false,
        }],
    }];
    validate_rich_interaction_response(Some(&[embed]), Some(&rows)).unwrap();
}

#[tokio::test]
async fn persisted_button_invocation_is_authorized_and_routed_to_owning_bot() {
    let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
    sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot','bot',1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','bot','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) \
         VALUES('install','bot','server','moderator','commands','active')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let conversation_id: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
             conversation_id,conversation_sequence,components_json) \
         VALUES('response','server','channel','bot','bot','Choose',?,1,?)",
    )
    .bind(conversation_id)
    .bind(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO interactions(id,interaction_type,user_id,server_id,channel_id,data_json, \
             application_user_id,expires_at,response_state,response_message_id) \
         VALUES('source','slash_command','target','server','channel','{}','bot', \
             datetime('now','+5 minutes'),'responded','response')",
    )
    .execute(&pool)
    .await
    .unwrap();

    engine
        .invoke_message_component(target_session, "response", "confirm", &[])
        .await
        .unwrap();
    let invoked: (String, String, String, String) = sqlx::query_as(
        "SELECT interaction_type,user_id,application_user_id,data_json FROM interactions \
         WHERE id!='source'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(invoked.0, "button");
    assert_eq!(invoked.1, "target");
    assert_eq!(invoked.2, "bot");
    assert!(invoked.3.contains("confirm"));

    sqlx::query(
        "INSERT INTO interactions(id,interaction_type,user_id,server_id,channel_id,data_json, \
             application_user_id,expires_at,response_state,ephemeral_response_json,response_expires_at) \
         VALUES('ephemeral-source','slash_command','target','server','channel','{}','bot', \
             datetime('now','+5 minutes'),'responded',?,datetime('now','+5 minutes'))",
    )
    .bind(r#"{"content":"Choose","components":[{"type":"action_row","components":[{"type":"button","custom_id":"private-confirm","label":"Confirm"}]}],"ephemeral":true}"#)
    .execute(&pool)
    .await
    .unwrap();
    engine
        .invoke_message_component(
            target_session,
            "ephemeral:ephemeral-source",
            "private-confirm",
            &[],
        )
        .await
        .unwrap();
    let private_invocation: (String, String) = sqlx::query_as(
        "SELECT interaction_type,user_id FROM interactions \
         WHERE interaction_type='button' AND data_json LIKE '%private-confirm%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(private_invocation, ("button".into(), "target".into()));
    assert!(
        engine
            .invoke_message_component(
                moderator_session,
                "ephemeral:ephemeral-source",
                "private-confirm",
                &[],
            )
            .await
            .is_err()
    );

    sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        engine
            .invoke_message_component(target_session, "response", "confirm", &[])
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM interactions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        4
    );
}

#[tokio::test]
async fn component_invocation_revalidates_source_access_and_installation_after_admission_wait() {
    async fn wait_until_queued(engine: &ChatEngine, available_before: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let available = engine
                    .write_admission
                    .as_ref()
                    .unwrap()
                    .pending_available_permits_for_test();
                if available < available_before {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("component invocation should queue for write admission");
    }

    async fn prepare_component_source(pool: &SqlitePool) {
        sqlx::query("INSERT OR IGNORE INTO users(id,username,is_bot) VALUES('bot','bot',1)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT OR IGNORE INTO server_members(server_id,user_id,role) \
             VALUES('server','bot','member')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT OR REPLACE INTO bot_installations \
             (id,bot_user_id,server_id,installed_by,granted_scopes,state,revoked_at) \
             VALUES('install','bot','server','moderator','commands','active',NULL)",
        )
        .execute(pool)
        .await
        .unwrap();
        let conversation_id: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT OR REPLACE INTO messages \
             (id,server_id,channel_id,sender_id,sender_nick,content,conversation_id, \
              conversation_sequence,components_json,deleted_at) \
             VALUES('response','server','channel','bot','bot','Choose',?,1,?,NULL)",
        )
        .bind(conversation_id)
        .bind(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT OR REPLACE INTO interactions \
             (id,interaction_type,user_id,server_id,channel_id,data_json,application_user_id, \
              expires_at,response_state,response_message_id) \
             VALUES('source','slash_command','target','server','channel','{}','bot', \
              datetime('now','+5 minutes'),'responded','response')",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    let (engine, pool, _moderator_session, target_session) = moderation_engine_fixture().await;
    prepare_component_source(&pool).await;
    let engine = std::sync::Arc::new(engine);

    for mutation in ["source_delete", "access_revoke", "uninstall"] {
        prepare_component_source(&pool).await;
        let admission = engine.write_admission.as_ref().unwrap();
        let held = admission.hold_active_capacity_for_test().await;
        let available_before = admission.pending_available_permits_for_test();
        let invocation = {
            let engine = engine.clone();
            tokio::spawn(async move {
                engine
                    .invoke_message_component(target_session, "response", "confirm", &[])
                    .await
            })
        };
        wait_until_queued(&engine, available_before).await;

        match mutation {
            "source_delete" => {
                sqlx::query("UPDATE messages SET deleted_at=datetime('now') WHERE id='response'")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            "access_revoke" => {
                sqlx::query(
                    "DELETE FROM server_members WHERE server_id='server' AND user_id='target'",
                )
                .execute(&pool)
                .await
                .unwrap();
            }
            "uninstall" => {
                sqlx::query(
                    "UPDATE bot_installations SET state='revoked',revoked_at=datetime('now') \
                     WHERE id='install'",
                )
                .execute(&pool)
                .await
                .unwrap();
            }
            _ => unreachable!(),
        }
        drop(held);
        let result = invocation.await.unwrap();
        assert!(
            result.is_err(),
            "{mutation} must invalidate the queued invocation"
        );
        let created: i64 =
            sqlx::query_scalar("SELECT count(*) FROM interactions WHERE interaction_type='button'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(created, 0, "{mutation} must not persist an interaction");

        if mutation == "access_revoke" {
            sqlx::query(
                "INSERT INTO server_members(server_id,user_id,role) \
                 VALUES('server','target','member')",
            )
            .execute(&pool)
            .await
            .unwrap();
        }
    }
}
