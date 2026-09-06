use super::{MessageMention, MessagingError, SqliteConnection};

pub(super) async fn insert_mentions(
    connection: &mut SqliteConnection,
    message_id: &str,
    mentions: &[MessageMention],
) -> Result<(), MessagingError> {
    for (ordinal, mention) in mentions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO message_mentions( \
                message_id,ordinal,mention_kind,target_id,start_byte,end_byte \
             ) VALUES(?,?,?,?,?,?)",
        )
        .bind(message_id)
        .bind(ordinal as i64)
        .bind(mention.kind.as_str())
        .bind(&mention.target_id)
        .bind(mention.start_byte as i64)
        .bind(mention.end_byte as i64)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}
