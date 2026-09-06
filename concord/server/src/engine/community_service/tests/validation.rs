use super::*;

#[tokio::test]
async fn template_instantiation_remaps_ids_atomically_and_rejects_legacy_formats() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('source','Source','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('source','owner','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let config = serde_json::json!({
        "format_version": 1,
        "categories": [{"id":"old-category","name":"Info","position":0}],
        "roles": [{"id":"old-everyone","name":"@everyone","color":null,"position":0,
            "permissions": Permissions::VIEW_CHANNELS.bits(), "is_default":true}],
        "channels": [{"id":"old-channel","name":"#welcome","topic":"Welcome",
            "category_id":"old-category","position":0,"is_private":false,
            "channel_type":"text","slowmode_seconds":0,"is_nsfw":false,
            "is_announcement":true,"is_default":true,"aliases":["welcome","start"],
            "role_overrides":[{"role_id":"old-everyone","allow_bits":0,
                "deny_bits":Permissions::SEND_MESSAGES.bits()}]}]
    });
    sqlx::query("INSERT INTO server_templates(id,name,server_id,created_by,config,format_version) VALUES('template','Template','source','owner',?,1)")
        .bind(config.to_string())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_templates(id,name,server_id,created_by,config,format_version) VALUES('legacy','Legacy','source','owner','{}',0)")
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );

    let new_server = service
        .instantiate_template(&actor, "template", "Copy")
        .await
        .unwrap();
    assert_ne!(new_server.as_str(), "source");
    let copied: (String, String, String) = sqlx::query_as(
        "SELECT c.id,c.category_id,cc.id FROM channels c JOIN channel_categories cc ON cc.id=c.category_id WHERE c.server_id=?",
    )
    .bind(new_server.as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_ne!(copied.0, "old-channel");
    assert_ne!(copied.1, "old-category");
    assert_eq!(copied.1, copied.2);
    let remapped: (i64, i64, String, String, i64) = sqlx::query_as(
        "SELECT c.is_default,c.is_announcement,a.channel_id,o.target_id,o.deny_bits \
         FROM channels c JOIN channel_aliases a ON a.channel_id=c.id AND a.alias='start' \
         JOIN channel_permission_overrides o ON o.channel_id=c.id AND o.target_type='role' \
         JOIN roles r ON r.id=o.target_id AND r.server_id=c.server_id \
         WHERE c.server_id=?",
    )
    .bind(new_server.as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remapped.0, 1);
    assert_eq!(remapped.1, 1);
    assert_eq!(remapped.2, copied.0);
    assert_ne!(remapped.3, "old-everyone");
    assert_eq!(remapped.4, Permissions::SEND_MESSAGES.bits() as i64);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT use_count FROM server_templates WHERE id='template'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM servers")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(matches!(
        service
            .instantiate_template(&actor, "legacy", "Must Not Exist")
            .await,
        Err(CommunityError::InvalidInput(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM servers")
            .fetch_one(&pool)
            .await
            .unwrap(),
        before
    );
}
