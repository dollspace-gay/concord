use super::{
    Actor, ActorScopeRequirement, AuthService, AuthorizationError, AuthorizationService,
    AuthorizationStamp, ChannelAction, ChannelRow, CredentialKind, Permissions, SqliteConnection,
    compute_effective_permissions,
};

impl AuthorizationService {
    pub(crate) async fn authorize_bot_installation_scope(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        scope: &str,
    ) -> Result<(), AuthorizationError> {
        if actor.kind() != CredentialKind::BotToken {
            return Err(AuthorizationError::Unavailable);
        }
        let mut transaction = self.pool.begin().await?;
        self.require_actor_scope_in(
            &mut transaction,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope,
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn require_actor_scope_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        requirement: ActorScopeRequirement<'_>,
    ) -> Result<(), AuthorizationError> {
        auth.validate_actor_in(connection, actor)
            .await
            .map_err(AuthorizationError::Authentication)?;
        let transport = match actor.kind() {
            CredentialKind::WebSession => "web",
            CredentialKind::IrcToken => "irc",
            CredentialKind::BotToken => "bot",
        };
        if !actor.scopes().contains(transport) {
            return Err(AuthorizationError::Unavailable);
        }
        if actor.kind() != CredentialKind::BotToken {
            return Ok(());
        }
        let granted: Option<String> = sqlx::query_scalar(
            "SELECT granted_scopes FROM bot_installations WHERE bot_user_id=? AND server_id=? \
             AND state='active' AND revoked_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .bind(requirement.server_id)
        .fetch_optional(&mut *connection)
        .await?;
        let granted = granted.ok_or(AuthorizationError::Unavailable)?;
        let installation_scopes = crate::auth::authority::CredentialScopes::parse(&granted);
        let exact = requirement
            .channel_id
            .map(|id| format!("webhook:channel:{id}"));
        let credential_allows = actor.scopes().contains(requirement.scope)
            || actor.scopes().contains("*")
            || exact.as_deref().is_some_and(|scope| {
                requirement.allow_exact_channel && actor.scopes().contains(scope)
            });
        let installation_allows = installation_scopes.contains(requirement.scope)
            || installation_scopes.contains("*")
            || exact.as_deref().is_some_and(|scope| {
                requirement.allow_exact_channel && installation_scopes.contains(scope)
            });
        if credential_allows && installation_allows {
            Ok(())
        } else {
            Err(AuthorizationError::Unavailable)
        }
    }

    pub async fn authorize_actor(
        &self,
        auth: &AuthService,
        actor: &Actor,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.authorize_actor_in(&mut transaction, auth, actor, channel_id, action)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn authorize_actor_stamped(
        &self,
        auth: &AuthService,
        actor: &Actor,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<AuthorizationStamp, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.authorize_actor_in(&mut transaction, auth, actor, channel_id, action)
            .await?;
        let channel: ChannelRow = self.load_channel(&mut transaction, channel_id).await?;
        let stamp = self
            .authorization_stamp(&mut transaction, &channel.server_id, &[channel.id])
            .await?;
        transaction.commit().await?;
        Ok(stamp)
    }

    pub(crate) async fn authorize_actor_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        resource: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let channel: ChannelRow = self.load_channel(connection, resource).await?;
        let scope = if action == ChannelAction::Manage {
            "channels"
        } else {
            "messages"
        };
        self.require_actor_scope_in(
            connection,
            auth,
            actor,
            ActorScopeRequirement {
                server_id: &channel.server_id,
                scope,
                channel_id: Some(resource),
                allow_exact_channel: action == ChannelAction::Send,
            },
        )
        .await?;
        self.authorize_channel_in(connection, actor.user_id().as_str(), resource, action)
            .await
    }

    pub(crate) async fn require_server_actor_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        required: Permissions,
    ) -> Result<(), AuthorizationError> {
        let permissions = self
            .server_actor_permissions_in(connection, auth, actor, server_id)
            .await?;
        if permissions.contains(required) {
            Ok(())
        } else {
            Err(AuthorizationError::Unavailable)
        }
    }

    pub(crate) async fn require_channel_actor_permission_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
        required: Permissions,
    ) -> Result<(), AuthorizationError> {
        let scope = if required.intersects(Permissions::MANAGE_CHANNELS) {
            "channels"
        } else {
            "messages"
        };
        self.require_actor_scope_in(
            connection,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope,
                channel_id: Some(channel_id),
                allow_exact_channel: required == Permissions::SEND_MESSAGES,
            },
        )
        .await?;
        let authority = self
            .server_authority(connection, actor.user_id().as_str(), server_id)
            .await?;
        let channel = self.load_channel(connection, channel_id).await?;
        if channel.server_id != server_id {
            return Err(AuthorizationError::Unavailable);
        }
        let permissions = self
            .channel_permissions(connection, actor.user_id().as_str(), &channel, &authority)
            .await?;
        if permissions.contains(required) {
            Ok(())
        } else {
            Err(AuthorizationError::Unavailable)
        }
    }

    pub(crate) async fn server_actor_permissions_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
    ) -> Result<Permissions, AuthorizationError> {
        self.require_actor_scope_in(
            connection,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope: "server",
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        let authority = self
            .server_authority(connection, actor.user_id().as_str(), server_id)
            .await?;
        let permissions = if authority.privileged {
            Permissions::all()
        } else {
            compute_effective_permissions(
                authority.base_permissions,
                &authority.role_permissions,
                &[],
                &authority.default_role_id,
                actor.user_id().as_str(),
                authority.owner_id == actor.user_id().as_str(),
            )
        };
        Ok(permissions)
    }
}
