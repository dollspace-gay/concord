use super::{
    Actor, ConversationId, DurableEventProjection, ReplayError, ReplayService, Row,
    authorize_conversation, load_message_projection, load_reaction_projection,
    load_read_projection, map_auth_error, resolve_current_event_state,
};

impl ReplayService {
    /// Resolve one durable descriptor to current state under current authority.
    /// `None` means the event is no longer visible to this principal.
    pub async fn project_event(
        &self,
        actor: &Actor,
        event_sequence: i64,
    ) -> Result<Option<(ConversationId, DurableEventProjection)>, ReplayError> {
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let Some(row) = sqlx::query(
            "SELECT conversation_id,event_kind,entity_type,entity_id,entity_version,descriptor_json \
             FROM event_log WHERE event_sequence=?",
        )
        .bind(event_sequence)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            return Ok(None);
        };
        let Some(conversation_id) = row.get::<Option<String>, _>(0) else {
            return Ok(None);
        };
        match authorize_conversation(
            &self.authorization,
            &self.auth,
            &mut transaction,
            actor,
            &conversation_id,
        )
        .await
        {
            Ok(()) => {}
            Err(ReplayError::Unavailable) => return Ok(None),
            Err(error) => return Err(error),
        }
        let event_kind: String = row.get(1);
        let entity_type: String = row.get(2);
        let entity_id: String = row.get(3);
        let recorded_version: i64 = row.get(4);
        let mut descriptor: serde_json::Value =
            serde_json::from_str(row.get::<&str, _>(5)).map_err(|_| ReplayError::InvalidInput)?;
        if entity_type == "read_state"
            && descriptor
                .get("user_id")
                .and_then(serde_json::Value::as_str)
                != Some(actor.user_id().as_str())
        {
            return Ok(None);
        }
        let message = if entity_type == "message" {
            load_message_projection(&mut transaction, &entity_id).await?
        } else {
            None
        };
        let reaction = if entity_type == "reaction" {
            load_reaction_projection(&mut transaction, &entity_id, &descriptor).await?
        } else {
            None
        };
        let read_state = if entity_type == "read_state" {
            load_read_projection(
                &mut transaction,
                actor.user_id().as_str(),
                &conversation_id,
                &entity_id,
            )
            .await?
        } else {
            None
        };
        let entity_version = resolve_current_event_state(
            &mut transaction,
            &entity_type,
            &entity_id,
            recorded_version,
            &mut descriptor,
        )
        .await?;
        transaction.commit().await?;
        let conversation_id =
            ConversationId::from_stored(conversation_id).map_err(|_| ReplayError::InvalidInput)?;
        Ok(Some((
            conversation_id.clone(),
            DurableEventProjection {
                kind: event_kind,
                conversation_id,
                entity_type,
                entity_id,
                entity_version: entity_version as u64,
                message,
                reaction,
                read_state,
                descriptor,
            },
        )))
    }
}
