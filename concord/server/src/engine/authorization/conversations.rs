use super::{
    Actor, ActorScopeRequirement, AuthService, AuthorizationError, AuthorizationService,
    ChannelAction, ConversationAction, CredentialKind, Row, SqliteConnection,
};

impl AuthorizationService {
    pub async fn authorize_conversation_actor_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        conversation_id: &str,
        action: ConversationAction,
    ) -> Result<(), AuthorizationError> {
        let conversation = sqlx::query("SELECT kind,channel_id FROM conversations WHERE id=?")
            .bind(conversation_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(AuthorizationError::Unavailable)?;
        if conversation.get::<String, _>(0) == "channel" {
            let channel_id: String = conversation
                .get::<Option<String>, _>(1)
                .ok_or(AuthorizationError::Unavailable)?;
            let channel_action = match action {
                ConversationAction::View => ChannelAction::View,
                ConversationAction::Read => ChannelAction::ReadHistory,
                ConversationAction::Send => ChannelAction::Send,
                ConversationAction::ManageMessages => ChannelAction::ManageMessages,
            };
            let channel = self.load_channel(connection, &channel_id).await?;
            self.require_actor_scope_in(
                connection,
                auth,
                actor,
                ActorScopeRequirement {
                    server_id: &channel.server_id,
                    scope: "messages",
                    channel_id: Some(&channel_id),
                    allow_exact_channel: action == ConversationAction::Send,
                },
            )
            .await?;
            return self
                .authorize_channel_in(
                    connection,
                    actor.user_id().as_str(),
                    &channel_id,
                    channel_action,
                )
                .await;
        }

        auth.validate_actor_in(connection, actor)
            .await
            .map_err(AuthorizationError::Authentication)?;
        let transport = match actor.kind() {
            CredentialKind::WebSession => "web",
            CredentialKind::IrcToken => "irc",
            CredentialKind::BotToken => return Err(AuthorizationError::Unavailable),
        };
        if !actor.scopes().contains(transport) {
            return Err(AuthorizationError::Unavailable);
        }

        let user_id = actor.user_id().as_str();
        let participant: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM conversation_participants cp JOIN users u ON u.id=cp.user_id WHERE cp.conversation_id=? AND cp.user_id=? AND cp.left_at IS NULL AND u.disabled_at IS NULL)",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_one(&mut *connection)
        .await?;
        if !participant || action == ConversationAction::ManageMessages {
            return Err(AuthorizationError::Unavailable);
        }
        if action != ConversationAction::Send {
            return Ok(());
        }
        let recipient: String = sqlx::query_scalar(
            "SELECT cp.user_id FROM conversation_participants cp JOIN users u ON u.id=cp.user_id \
             WHERE cp.conversation_id=? AND cp.user_id<>? AND cp.left_at IS NULL AND u.disabled_at IS NULL \
             ORDER BY cp.user_id LIMIT 1",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(AuthorizationError::Unavailable)?;
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM user_blocks WHERE (blocker_user_id=? AND blocked_user_id=?) OR (blocker_user_id=? AND blocked_user_id=?))",
        )
        .bind(user_id)
        .bind(&recipient)
        .bind(&recipient)
        .bind(user_id)
        .fetch_one(&mut *connection)
        .await?;
        if blocked {
            return Err(AuthorizationError::Unavailable);
        }
        let preference: Option<String> =
            sqlx::query_scalar("SELECT allow_from FROM direct_message_preferences WHERE user_id=?")
                .bind(&recipient)
                .fetch_optional(&mut *connection)
                .await?;
        match preference.as_deref().unwrap_or("shared_server") {
            "everyone" => Ok(()),
            "shared_server" => {
                let shared: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM server_members sender JOIN server_members recipient ON recipient.server_id=sender.server_id WHERE sender.user_id=? AND recipient.user_id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=sender.server_id AND b.user_id IN (?,?)))",
                )
                .bind(user_id)
                .bind(&recipient)
                .bind(user_id)
                .bind(&recipient)
                .fetch_one(connection)
                .await?;
                shared.then_some(()).ok_or(AuthorizationError::Unavailable)
            }
            _ => Err(AuthorizationError::Unavailable),
        }
    }
}
