use super::{
    Actor, ConversationAction, MessageTarget, MessagingError, MessagingService,
    OwnedSemaphorePermit, Row, Sqlite, SqliteConnection, Transaction, map_authorization_error,
};

impl MessagingService {
    pub(super) async fn begin_write(
        &self,
    ) -> Result<(OwnedSemaphorePermit, Transaction<'static, Sqlite>), MessagingError> {
        self.write_admission
            .begin()
            .await
            .map_err(|error| match error {
                crate::engine::write_admission::WriteAdmissionError::Unavailable => {
                    MessagingError::DependencyUnavailable
                }
                crate::engine::write_admission::WriteAdmissionError::Database(error) => {
                    MessagingError::Internal(error)
                }
            })
    }

    pub(super) async fn load_and_authorize_message(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
        message_id: &str,
        allow_deleted: bool,
    ) -> Result<MessageTarget, MessagingError> {
        let row = sqlx::query(
            "SELECT m.id,m.conversation_id,m.conversation_sequence,COALESCE(m.server_id,''), \
                    COALESCE(m.channel_id,''),m.sender_id,COALESCE(c.authorization_version,0), \
                    m.deleted_at,cv.kind \
             FROM messages m JOIN conversations cv ON cv.id=m.conversation_id \
             LEFT JOIN channels c ON c.id=m.channel_id \
             WHERE m.id=? AND m.conversation_id IS NOT NULL",
        )
        .bind(message_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        if !allow_deleted && row.get::<Option<String>, _>(7).is_some() {
            return Err(MessagingError::Unavailable);
        }
        let target = MessageTarget {
            message_id: row.get(0),
            conversation_id: row.get(1),
            conversation_sequence: row.get(2),
            server_id: row.get(3),
            channel_id: row.get(4),
            sender_id: row.get(5),
            authorization_version: row.get(6),
            direct: row.get::<String, _>(8) == "direct",
            deleted: row.get::<Option<String>, _>(7).is_some(),
        };
        self.authorization
            .authorize_conversation_actor_in(
                connection,
                &self.auth,
                actor,
                &target.conversation_id,
                ConversationAction::Read,
            )
            .await
            .map_err(map_authorization_error)?;
        Ok(target)
    }
}
