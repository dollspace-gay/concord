use super::{
    EventIdentity, MessageTarget, MessagingError, Row, SqliteConnection, insert_event,
    set_entity_version,
};

pub(super) async fn propagate_announcement_edit(
    connection: &mut SqliteConnection,
    generation: &str,
    source_message_id: &str,
    content: &str,
    content_format: &str,
    source_version: i64,
    actor_id: &str,
) -> Result<(), MessagingError> {
    let targets = sqlx::query(
        "SELECT ap.id,m.id,m.conversation_id,m.conversation_sequence,m.server_id,m.channel_id, \
                c.authorization_version \
         FROM announcement_publications ap \
         JOIN messages m ON m.id=ap.target_message_id \
         JOIN channels c ON c.id=m.channel_id \
         WHERE ap.source_message_id=? AND ap.state='published' AND m.deleted_at IS NULL",
    )
    .bind(source_message_id)
    .fetch_all(&mut *connection)
    .await?;
    for row in targets {
        let target_message_id: String = row.get(1);
        let target_version: i64 = sqlx::query_scalar(
            "UPDATE messages SET content=?,content_format=?,edited_at=datetime('now'), \
             entity_version=entity_version+1 WHERE id=? RETURNING entity_version",
        )
        .bind(content)
        .bind(content_format)
        .bind(&target_message_id)
        .fetch_one(&mut *connection)
        .await?;
        set_entity_version(connection, "message", &target_message_id, target_version).await?;
        let target = MessageTarget {
            message_id: target_message_id.clone(),
            conversation_id: row.get(2),
            conversation_sequence: row.get(3),
            server_id: row.get(4),
            channel_id: row.get(5),
            sender_id: actor_id.to_string(),
            authorization_version: row.get(6),
            direct: false,
            deleted: false,
        };
        insert_event(
            connection,
            generation,
            &target,
            EventIdentity {
                kind: "message_edited",
                entity_type: "message",
                entity_id: &target_message_id,
                version: target_version,
            },
            actor_id,
            serde_json::json!({
                "message_id": target_message_id,
                "announcement_source_message_id": source_message_id,
            }),
        )
        .await?;
        sqlx::query(
            "UPDATE announcement_publications SET source_version=?,updated_at=datetime('now') \
             WHERE id=?",
        )
        .bind(source_version)
        .bind(row.get::<String, _>(0))
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

pub(super) async fn propagate_announcement_delete(
    connection: &mut SqliteConnection,
    generation: &str,
    source_message_id: &str,
    source_version: i64,
    actor_id: &str,
) -> Result<(), MessagingError> {
    let targets = sqlx::query(
        "SELECT ap.id,m.id,m.conversation_id,m.conversation_sequence,m.server_id,m.channel_id, \
                c.authorization_version \
         FROM announcement_publications ap \
         JOIN messages m ON m.id=ap.target_message_id \
         JOIN channels c ON c.id=m.channel_id \
         WHERE ap.source_message_id=? AND ap.state='published'",
    )
    .bind(source_message_id)
    .fetch_all(&mut *connection)
    .await?;
    for row in targets {
        let target_message_id: String = row.get(1);
        let target_version: i64 = sqlx::query_scalar(
            "UPDATE messages SET deleted_at=COALESCE(deleted_at,datetime('now')), \
             entity_version=entity_version+1 WHERE id=? RETURNING entity_version",
        )
        .bind(&target_message_id)
        .fetch_one(&mut *connection)
        .await?;
        set_entity_version(connection, "message", &target_message_id, target_version).await?;
        let target = MessageTarget {
            message_id: target_message_id.clone(),
            conversation_id: row.get(2),
            conversation_sequence: row.get(3),
            server_id: row.get(4),
            channel_id: row.get(5),
            sender_id: actor_id.to_string(),
            authorization_version: row.get(6),
            direct: false,
            deleted: true,
        };
        insert_event(
            connection,
            generation,
            &target,
            EventIdentity {
                kind: "message_deleted",
                entity_type: "message",
                entity_id: &target_message_id,
                version: target_version,
            },
            actor_id,
            serde_json::json!({
                "message_id": target_message_id,
                "announcement_source_message_id": source_message_id,
            }),
        )
        .await?;
        sqlx::query(
            "UPDATE announcement_publications SET state='deleted',source_version=?, \
             updated_at=datetime('now') WHERE id=?",
        )
        .bind(source_version)
        .bind(row.get::<String, _>(0))
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}
