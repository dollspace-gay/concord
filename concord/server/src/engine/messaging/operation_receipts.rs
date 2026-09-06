use super::{CommandReceipt, MessageTarget, MessagingError, Row, SqliteConnection};

pub(super) async fn operation_generation(
    connection: &mut SqliteConnection,
    requested: Option<&str>,
) -> Result<String, MessagingError> {
    let row = sqlx::query(
        "SELECT s.current_generation,g.expires_at \
         FROM operation_generation_state s \
         JOIN operation_generations g ON g.generation=s.current_generation \
         WHERE s.singleton=1",
    )
    .fetch_one(&mut *connection)
    .await?;
    let current: String = row.get(0);
    let expires_at: i64 = row.get(1);
    let now: i64 = sqlx::query_scalar("SELECT unixepoch()")
        .fetch_one(&mut *connection)
        .await?;
    if expires_at <= now {
        if requested.is_some() {
            return Err(MessagingError::OperationGenerationExpired);
        }
        let next: String = sqlx::query_scalar("SELECT lower(hex(randomblob(16)))")
            .fetch_one(&mut *connection)
            .await?;
        sqlx::query(
            "INSERT INTO operation_generations(generation,issued_at,expires_at) \
             VALUES(?,?,?)",
        )
        .bind(&next)
        .bind(now)
        .bind(now + 604_800)
        .execute(&mut *connection)
        .await?;
        sqlx::query("UPDATE operation_generation_state SET current_generation=? WHERE singleton=1")
            .bind(&next)
            .execute(&mut *connection)
            .await?;
        return Ok(next);
    }
    if requested.is_some_and(|generation| generation != current) {
        return Err(MessagingError::OperationGenerationExpired);
    }
    Ok(current)
}

pub(super) async fn load_receipt(
    connection: &mut SqliteConnection,
    principal_id: &str,
    client_message_id: &str,
    fingerprint: &str,
    request_id: &str,
) -> Result<Option<CommandReceipt>, MessagingError> {
    let row = sqlx::query(
        "SELECT payload_fingerprint,response_json,event_sequence FROM command_receipts \
         WHERE principal_id=? AND client_message_id=?",
    )
    .bind(principal_id)
    .bind(client_message_id)
    .fetch_optional(connection)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<String, _>(0) != fingerprint {
        return Err(MessagingError::IdempotencyConflict);
    }
    let mut receipt: CommandReceipt = serde_json::from_str(row.get::<&str, _>(1))
        .map_err(|_| MessagingError::DependencyUnavailable)?;
    receipt.request_id = request_id.to_owned();
    receipt.replayed = true;
    receipt.event_sequence_internal = row.get::<i64, _>(2) as u64;
    Ok(Some(receipt))
}

pub(super) fn mutation_receipt(
    request_id: &str,
    client_message_id: &str,
    message_id: &str,
    conversation_sequence: i64,
    event_sequence: i64,
    entity_version: i64,
    persisted_at: &str,
) -> CommandReceipt {
    CommandReceipt {
        request_id: request_id.to_owned(),
        client_message_id: client_message_id.to_owned(),
        message_id: message_id.to_owned(),
        sequence: conversation_sequence.to_string(),
        entity_version: entity_version as u64,
        persisted_at: persisted_at.to_owned(),
        replayed: false,
        event_sequence_internal: event_sequence as u64,
    }
}

pub(super) async fn insert_receipt(
    connection: &mut SqliteConnection,
    principal_id: &str,
    generation: &str,
    operation_kind: &str,
    fingerprint: &str,
    target: &MessageTarget,
    receipt: &CommandReceipt,
) -> Result<(), MessagingError> {
    sqlx::query(
        "INSERT INTO command_receipts( \
            principal_id,operation_generation,client_message_id,request_id,operation_kind, \
            payload_fingerprint,conversation_id,canonical_message_id,conversation_sequence, \
            event_sequence,entity_version,persisted_at,response_json \
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(principal_id)
    .bind(generation)
    .bind(&receipt.client_message_id)
    .bind(&receipt.request_id)
    .bind(operation_kind)
    .bind(fingerprint)
    .bind(&target.conversation_id)
    .bind(&target.message_id)
    .bind(target.conversation_sequence)
    .bind(receipt.event_sequence_internal as i64)
    .bind(receipt.entity_version as i64)
    .bind(&receipt.persisted_at)
    .bind(serde_json::to_string(receipt).expect("receipt serialization is infallible"))
    .execute(connection)
    .await?;
    Ok(())
}
