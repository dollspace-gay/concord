use super::*;

#[tokio::test]
async fn visibility_mutations_advance_channel_and_server_versions() {
    let (pool, _) = fixture().await;
    sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','private_thread','public')")
        .execute(&pool).await.unwrap();

    let versions = async || {
        (
            sqlx::query_scalar::<_, i64>(
                "SELECT authorization_version FROM channels WHERE id='thread'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            sqlx::query_scalar::<_, i64>(
                "SELECT authorization_version FROM servers WHERE id='server'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
        )
    };

    let mut previous = versions().await;
    for statement in [
        "INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) VALUES('thread','user','member')",
        "UPDATE channel_visibility_grants SET target_id='owner' WHERE channel_id='thread'",
        "DELETE FROM channel_visibility_grants WHERE channel_id='thread'",
        "INSERT INTO thread_members(thread_id,user_id) VALUES('thread','member')",
        "UPDATE thread_members SET user_id='owner' WHERE thread_id='thread'",
        "DELETE FROM thread_members WHERE thread_id='thread'",
        "UPDATE channels SET visibility_repair_required=1 WHERE id='thread'",
        "UPDATE channels SET is_private=1 WHERE id='thread'",
        "UPDATE channels SET channel_type='public_thread' WHERE id='thread'",
        "UPDATE channels SET parent_channel_id=NULL WHERE id='thread'",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
        let current = versions().await;
        assert!(
            current.0 > previous.0,
            "channel version unchanged: {statement}"
        );
        assert!(
            current.1 > previous.1,
            "server version unchanged: {statement}"
        );
        previous = current;
    }

    let server_before: i64 =
        sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id='server'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE servers SET owner_id='member' WHERE id='server'")
        .execute(&pool)
        .await
        .unwrap();
    let server_after: i64 =
        sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id='server'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(server_after > server_before);
}
