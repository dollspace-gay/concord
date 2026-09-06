#[cfg(feature = "storage-fault-injection")]
use super::StorageFaultBarrierStage;
use super::{
    Actor, CommandReceipt, MessagingError, MessagingService, SendMessageCommand, validate_command,
    validate_interaction_response_command,
};

impl MessagingService {
    pub async fn send_channel_message(
        &self,
        actor: &Actor,
        command: SendMessageCommand<'_>,
    ) -> Result<CommandReceipt, MessagingError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::MessageCommit);
        validate_command(&command, self.max_message_length)?;
        let (permit, mut transaction) = self.begin_write().await?;
        let result = self
            .send_channel_message_in(&mut transaction, actor, &command, command.content)
            .await;
        let result = match result {
            Ok(receipt) => {
                #[cfg(feature = "storage-fault-injection")]
                self.wait_storage_fault_barrier(StorageFaultBarrierStage::BeforeCommit)
                    .await;
                transaction
                    .commit()
                    .await
                    .map_err(MessagingError::Internal)?;
                #[cfg(feature = "storage-fault-injection")]
                self.wait_storage_fault_barrier(StorageFaultBarrierStage::AfterCommit)
                    .await;
                Ok(receipt)
            }
            Err(error @ MessagingError::AutoModRejected(_)) => {
                transaction
                    .commit()
                    .await
                    .map_err(MessagingError::Internal)?;
                Err(error)
            }
            Err(error) => Err(error),
        };
        drop(permit);
        if let Ok(receipt) = &result
            && !receipt.replayed
        {
            let _ = self.wakeups.send(receipt.event_sequence_internal);
        }
        if result.is_ok() {
            metric.succeed();
        }
        result
    }

    /// Commit a public interaction response as a canonical message and consume
    /// the interaction in the same write transaction.
    pub async fn respond_to_interaction_public(
        &self,
        actor: &Actor,
        interaction_id: &str,
        command: SendMessageCommand<'_>,
        rich_embeds_json: Option<&str>,
        components_json: Option<&str>,
    ) -> Result<CommandReceipt, MessagingError> {
        validate_interaction_response_command(
            &command,
            self.max_message_length,
            rich_embeds_json.is_some() || components_json.is_some(),
        )?;
        let (permit, mut transaction) = self.begin_write().await?;
        let receipt = match self
            .send_channel_message_in(&mut transaction, actor, &command, command.content)
            .await
        {
            Ok(receipt) => receipt,
            Err(error @ MessagingError::AutoModRejected(_)) => {
                transaction
                    .commit()
                    .await
                    .map_err(MessagingError::Internal)?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let response_channel: Option<String> =
            sqlx::query_scalar("SELECT channel_id FROM messages WHERE id=?")
                .bind(&receipt.message_id)
                .fetch_optional(&mut *transaction)
                .await?;
        let expected_channel: Option<String> =
            sqlx::query_scalar("SELECT channel_id FROM interactions WHERE id=?")
                .bind(interaction_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if response_channel.is_none() || response_channel != expected_channel {
            return Err(MessagingError::Unavailable);
        }
        sqlx::query("UPDATE messages SET rich_embeds_json=?,components_json=? WHERE id=?")
            .bind(rich_embeds_json)
            .bind(components_json)
            .bind(&receipt.message_id)
            .execute(&mut *transaction)
            .await?;
        use crate::db::queries::slash_commands::InteractionResponseResult;
        match crate::db::queries::slash_commands::accept_interaction_response(
            &mut transaction,
            interaction_id,
            actor.user_id().as_str(),
            Some(&receipt.message_id),
            None,
            None,
        )
        .await?
        {
            InteractionResponseResult::Accepted => {}
            InteractionResponseResult::AlreadyResponded => {
                return Err(MessagingError::Conflict(
                    "interaction already responded".into(),
                ));
            }
            InteractionResponseResult::Expired => {
                return Err(MessagingError::Conflict("interaction expired".into()));
            }
            InteractionResponseResult::WrongApplication | InteractionResponseResult::NotFound => {
                return Err(MessagingError::Unavailable);
            }
        }
        transaction
            .commit()
            .await
            .map_err(MessagingError::Internal)?;
        drop(permit);
        if !receipt.replayed {
            let _ = self.wakeups.send(receipt.event_sequence_internal);
        }
        Ok(receipt)
    }
}
