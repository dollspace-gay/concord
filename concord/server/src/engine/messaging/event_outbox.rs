use super::{MessageTarget, MessagingError, SqliteConnection, Uuid};

pub(super) struct EventIdentity<'a> {
    pub(super) kind: &'a str,
    pub(super) entity_type: &'a str,
    pub(super) entity_id: &'a str,
    pub(super) version: i64,
}

pub(super) async fn insert_event(
    connection: &mut SqliteConnection,
    generation: &str,
    target: &MessageTarget,
    event: EventIdentity<'_>,
    actor_id: &str,
    descriptor: serde_json::Value,
) -> Result<i64, MessagingError> {
    let event_sequence: i64 = sqlx::query_scalar(
        "INSERT INTO event_log( \
            database_generation,conversation_id,event_kind,entity_type,entity_id, \
            entity_version,authorization_version,actor_id,descriptor_json \
         ) VALUES(?,?,?,?,?,?,?,?,?) RETURNING event_sequence",
    )
    .bind(generation)
    .bind(&target.conversation_id)
    .bind(event.kind)
    .bind(event.entity_type)
    .bind(event.entity_id)
    .bind(event.version)
    .bind(target.authorization_version)
    .bind(actor_id)
    .bind(descriptor.to_string())
    .fetch_one(&mut *connection)
    .await?;
    sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
        .bind(event_sequence)
        .execute(&mut *connection)
        .await?;
    enqueue_outgoing_webhooks(
        connection,
        event_sequence,
        target,
        &event,
        actor_id,
        &descriptor,
    )
    .await?;
    Ok(event_sequence)
}

pub(super) async fn enqueue_outgoing_webhooks(
    connection: &mut SqliteConnection,
    event_sequence: i64,
    target: &MessageTarget,
    event: &EventIdentity<'_>,
    actor_id: &str,
    descriptor: &serde_json::Value,
) -> Result<(), MessagingError> {
    let event_type = match event.kind {
        "message_created" => "message_create",
        "message_edited" => "message_update",
        "message_deleted" => "message_delete",
        other => other,
    };
    let mut outbound_data = descriptor.clone();
    if matches!(event_type, "message_create" | "message_update") {
        let row: Option<(String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT content,content_format,sender_id,sender_nick,edited_at FROM messages \
             WHERE id=? AND channel_id=? AND deleted_at IS NULL",
        )
        .bind(event.entity_id)
        .bind(&target.channel_id)
        .fetch_optional(&mut *connection)
        .await?;
        if let (Some(fields), Some(data)) = (row, outbound_data.as_object_mut()) {
            data.insert("content".into(), fields.0.into());
            data.insert("content_format".into(), fields.1.into());
            data.insert("sender_id".into(), fields.2.into());
            data.insert("sender_nick".into(), fields.3.into());
            data.insert("edited_at".into(), fields.4.into());
        }
    }
    let webhooks: Vec<(String, i64)> = sqlx::query_as(
        "SELECT w.id,w.grant_version FROM webhooks w \
         JOIN webhook_events e ON e.webhook_id=w.id \
         JOIN channels c ON c.id=w.channel_id AND c.server_id=w.server_id \
         WHERE w.server_id=? AND w.channel_id=? AND c.is_private=0 \
           AND w.webhook_type='outgoing' \
           AND w.credential_state='active' AND w.revoked_at IS NULL AND e.event_type=?",
    )
    .bind(&target.server_id)
    .bind(&target.channel_id)
    .bind(event_type)
    .fetch_all(&mut *connection)
    .await?;
    for (webhook_id, grant_version) in webhooks {
        let job_id = Uuid::new_v4().to_string();
        let delivery_id = Uuid::new_v4().to_string();
        let deduplication_key = format!("webhook:{webhook_id}:event:{event_sequence}");
        let destination_grant = format!("webhook:{webhook_id}:{grant_version}");
        let payload = serde_json::json!({
            "delivery_id": delivery_id,
            "event_type": event_type,
            "event_version": event.version,
            "entity_type": event.entity_type,
            "entity_id": event.entity_id,
            "actor_id": actor_id,
            "server_id": target.server_id,
            "channel_id": target.channel_id,
            "conversation_id": target.conversation_id,
            "data": &outbound_data,
        });
        sqlx::query(
            "INSERT INTO external_jobs( \
                id,deduplication_key,operation_type,resource_id,resource_version, \
                destination_grant,payload_json \
             ) VALUES(?,?,'webhook_delivery',?,?,?,?)",
        )
        .bind(&job_id)
        .bind(&deduplication_key)
        .bind(&delivery_id)
        .bind(event.version)
        .bind(&destination_grant)
        .bind(payload.to_string())
        .execute(&mut *connection)
        .await?;
        sqlx::query(
            "INSERT INTO webhook_deliveries( \
                id,webhook_id,event_sequence,external_job_id,delivery_id,event_type, \
                event_version,payload_json \
             ) VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&webhook_id)
        .bind(event_sequence)
        .bind(&job_id)
        .bind(&delivery_id)
        .bind(event_type)
        .bind(event.version)
        .bind(payload.to_string())
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

pub(super) async fn set_entity_version(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    version: i64,
) -> Result<(), MessagingError> {
    sqlx::query(
        "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
         VALUES(?,?,?,datetime('now')) \
         ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
             version=excluded.version,updated_at=excluded.updated_at",
    )
    .bind(entity_type)
    .bind(entity_id)
    .bind(version)
    .execute(connection)
    .await?;
    Ok(())
}

pub(super) async fn advance_entity_version(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> Result<i64, MessagingError> {
    Ok(sqlx::query_scalar(
        "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
         VALUES(?,?,1,datetime('now')) \
         ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
             version=entity_versions.version+1,updated_at=excluded.updated_at \
         RETURNING version",
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_one(connection)
    .await?)
}
