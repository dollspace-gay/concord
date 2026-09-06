use super::{
    Actor, AnnouncementPublication, ChannelAction, EventIdentity, MessageTarget, MessagingError,
    MessagingService, PublishAnnouncementCommand, Row, Uuid, database_generation, insert_event,
    map_authorization_error, set_entity_version,
};

impl MessagingService {
    /// Explicitly publish one message from a public announcement channel to
    /// every currently authorized follow destination. The lineage uniqueness
    /// constraint makes retries idempotent per follow and source message.
    pub async fn publish_announcement(
        &self,
        actor: &Actor,
        command: PublishAnnouncementCommand<'_>,
    ) -> Result<Vec<AnnouncementPublication>, MessagingError> {
        let (_permit, mut transaction) = self.begin_write().await?;
        let source = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, false)
            .await?;
        if source.direct {
            return Err(MessagingError::Unavailable);
        }
        self.authorization
            .authorize_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &source.channel_id,
                ChannelAction::ManageMessages,
            )
            .await
            .map_err(map_authorization_error)?;
        let source_row = sqlx::query(
            "SELECT m.content,m.content_format,m.entity_version,m.sender_id,m.sender_nick, \
                    c.is_announcement,c.is_private \
             FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.id=?",
        )
        .bind(command.message_id)
        .fetch_one(&mut *transaction)
        .await?;
        if source_row.get::<i64, _>(5) == 0 || source_row.get::<i64, _>(6) != 0 {
            return Err(MessagingError::Unavailable);
        }
        let follows = sqlx::query(
            "SELECT cf.id,cf.target_channel_id,cf.created_by,cv.id,c.server_id,c.authorization_version \
             FROM channel_follows cf \
             JOIN channels c ON c.id=cf.target_channel_id \
             JOIN conversations cv ON cv.channel_id=c.id AND cv.kind='channel' \
             WHERE cf.source_channel_id=? ORDER BY cf.id LIMIT 101",
        )
        .bind(&source.channel_id)
        .fetch_all(&mut *transaction)
        .await?;
        if follows.len() > 100 {
            return Err(MessagingError::Conflict(
                "announcement fanout exceeds 100 destinations".into(),
            ));
        }
        let generation = database_generation(&mut transaction).await?;
        let mut publications = Vec::new();
        let mut wakeup = None;
        for follow in follows {
            let follow_id: String = follow.get(0);
            let target_channel_id: String = follow.get(1);
            let grant_owner: String = follow.get(2);
            if self
                .authorization
                .authorize_channel_in(
                    &mut transaction,
                    &grant_owner,
                    &target_channel_id,
                    ChannelAction::Send,
                )
                .await
                .is_err()
                || self
                    .authorization
                    .authorize_channel_in(
                        &mut transaction,
                        &grant_owner,
                        &target_channel_id,
                        ChannelAction::Manage,
                    )
                    .await
                    .is_err()
            {
                continue;
            }
            if let Some(existing) = sqlx::query(
                "SELECT id,target_message_id FROM announcement_publications \
                 WHERE follow_id=? AND source_message_id=? AND state='published'",
            )
            .bind(&follow_id)
            .bind(command.message_id)
            .fetch_optional(&mut *transaction)
            .await?
            {
                if let Some(target_message_id) = existing.get::<Option<String>, _>(1) {
                    publications.push(AnnouncementPublication {
                        publication_id: existing.get(0),
                        target_message_id,
                        target_channel_id,
                    });
                }
                continue;
            }
            let target_conversation_id: String = follow.get(3);
            let target_server_id: String = follow.get(4);
            let target_authorization_version: i64 = follow.get(5);
            let target_sequence: i64 = sqlx::query_scalar(
                "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
                 WHERE id=? RETURNING next_message_sequence",
            )
            .bind(&target_conversation_id)
            .fetch_one(&mut *transaction)
            .await?;
            let target_message_id = Uuid::new_v4().to_string();
            let publication_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO messages( \
                    id,server_id,channel_id,sender_id,sender_nick,content,conversation_id, \
                    conversation_sequence,content_format,entity_version \
                 ) VALUES(?,?,?,?,?,?,?,?,?,1)",
            )
            .bind(&target_message_id)
            .bind(&target_server_id)
            .bind(&target_channel_id)
            .bind(source_row.get::<&str, _>(3))
            .bind(source_row.get::<&str, _>(4))
            .bind(source_row.get::<&str, _>(0))
            .bind(&target_conversation_id)
            .bind(target_sequence)
            .bind(source_row.get::<&str, _>(1))
            .execute(&mut *transaction)
            .await?;
            set_entity_version(&mut transaction, "message", &target_message_id, 1).await?;
            let target = MessageTarget {
                message_id: target_message_id.clone(),
                conversation_id: target_conversation_id,
                conversation_sequence: target_sequence,
                server_id: target_server_id,
                channel_id: target_channel_id.clone(),
                sender_id: source_row.get(3),
                authorization_version: target_authorization_version,
                direct: false,
                deleted: false,
            };
            let event_sequence = insert_event(
                &mut transaction,
                &generation,
                &target,
                EventIdentity {
                    kind: "message_created",
                    entity_type: "message",
                    entity_id: &target_message_id,
                    version: 1,
                },
                actor.user_id().as_str(),
                serde_json::json!({
                    "conversation_id": target.conversation_id,
                    "message_id": target_message_id,
                    "conversation_sequence": target_sequence.to_string(),
                    "announcement_source_message_id": command.message_id,
                }),
            )
            .await?;
            wakeup = Some(event_sequence as u64);
            sqlx::query(
                "INSERT INTO announcement_publications( \
                    id,follow_id,source_message_id,target_message_id,source_version,state \
                 ) VALUES(?,?,?,?,?,'published')",
            )
            .bind(&publication_id)
            .bind(&follow_id)
            .bind(command.message_id)
            .bind(&target_message_id)
            .bind(source_row.get::<i64, _>(2))
            .execute(&mut *transaction)
            .await?;
            publications.push(AnnouncementPublication {
                publication_id,
                target_message_id,
                target_channel_id,
            });
        }
        if publications.is_empty() {
            return Err(MessagingError::Conflict(
                "announcement has no authorized destinations".into(),
            ));
        }
        transaction.commit().await?;
        if let Some(sequence) = wakeup {
            let _ = self.wakeups.send(sequence);
        }
        Ok(publications)
    }
}
