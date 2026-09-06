use super::*;

#[tokio::test]
async fn history_allow_cannot_bypass_view_deny() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits,deny_bits) VALUES('deny','public','role','everyone',?,?)")
        .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64)
        .bind(Permissions::VIEW_CHANNELS.bits() as i64)
        .execute(&pool).await.unwrap();
    assert!(matches!(
        service
            .authorize_channel("member", "public", ChannelAction::ReadHistory)
            .await,
        Err(AuthorizationError::Unavailable)
    ));
}

#[tokio::test]
async fn public_thread_cannot_exceed_parent_visibility() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','public_thread','public')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('parent-deny','public','role','everyone',?)")
        .bind(Permissions::VIEW_CHANNELS.bits() as i64).execute(&pool).await.unwrap();
    assert!(matches!(
        service
            .authorize_channel("member", "thread", ChannelAction::View)
            .await,
        Err(AuthorizationError::Unavailable)
    ));
}

#[tokio::test]
async fn thread_cannot_exceed_parent_repair_guard() {
    let (pool, service) = fixture().await;
    sqlx::query("UPDATE channels SET visibility_repair_required=1 WHERE id='public'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','public_thread','public')")
        .execute(&pool).await.unwrap();
    assert!(matches!(
        service
            .authorize_channel("member", "thread", ChannelAction::View)
            .await,
        Err(AuthorizationError::Unavailable)
    ));
}

#[tokio::test]
async fn date_only_after_excludes_the_named_utc_day_and_includes_next_midnight() {
    let (pool, service) = fixture().await;
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
         ('near-midnight','server','public','owner','Alice','near','2026-09-01T23:59:59.999Z'), \
         ('midnight','server','public','owner','Alice','midnight','2026-09-02T00:00:00Z')",
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
                query: None,
                requested_channel_id: None,
                sender: None,
                has_attachment: false,
                has_link: false,
                before: None,
                after: Some("2026-09-02T00:00:00Z"),
                after_inclusive: true,
                limit: 50,
                offset: 0,
                cursor_created_at: None,
                cursor_message_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].id, "midnight");
}
