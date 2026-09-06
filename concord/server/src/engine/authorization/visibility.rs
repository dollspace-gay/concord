use super::{
    Actor, ActorScopeRequirement, AuthService, AuthorizationError, AuthorizationService,
    AuthorizationStamp, ChannelAction, ChannelRow, Permissions, ServerMemberRow,
    compute_effective_permissions,
};

impl AuthorizationService {
    pub async fn authorize_channel(
        &self,
        user_id: &str,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.authorize_channel_in(&mut transaction, user_id, channel_id, action)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn effective_permissions(
        &self,
        user_id: &str,
        server_id: &str,
        channel_id: Option<&str>,
    ) -> Result<Permissions, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let permissions = if let Some(channel_id) = channel_id {
            let channel = self.load_channel(&mut transaction, channel_id).await?;
            if channel.server_id != server_id {
                return Err(AuthorizationError::Unavailable);
            }
            self.channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?
        } else if authority.privileged {
            Permissions::all()
        } else {
            compute_effective_permissions(
                authority.base_permissions,
                &authority.role_permissions,
                &[],
                &authority.default_role_id,
                user_id,
                authority.owner_id == user_id,
            )
        };
        transaction.commit().await?;
        Ok(permissions)
    }

    pub async fn visible_channels(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Vec<ChannelRow>, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let channels = sqlx::query_as::<_, ChannelRow>(
            "SELECT * FROM channels WHERE server_id=? ORDER BY position,name",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut visible = Vec::new();
        for channel in channels {
            let permissions = self
                .channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?;
            if permissions.contains(Permissions::VIEW_CHANNELS)
                && self
                    .visibility_granted(&mut transaction, user_id, &channel, &authority)
                    .await?
            {
                visible.push(channel);
            }
        }
        transaction.commit().await?;
        Ok(visible)
    }

    pub async fn visible_channels_for_actor(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(Vec<ChannelRow>, AuthorizationStamp), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.require_actor_scope_in(
            &mut transaction,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope: "messages",
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        let user_id = actor.user_id().as_str();
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let channels = sqlx::query_as::<_, ChannelRow>(
            "SELECT * FROM channels WHERE server_id=? ORDER BY position,name",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut visible = Vec::new();
        for channel in channels {
            let permissions = self
                .channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?;
            if permissions.contains(Permissions::VIEW_CHANNELS)
                && self
                    .visibility_granted(&mut transaction, user_id, &channel, &authority)
                    .await?
            {
                visible.push(channel);
            }
        }
        let ids = visible
            .iter()
            .map(|channel| channel.id.clone())
            .collect::<Vec<_>>();
        let stamp = self
            .authorization_stamp(&mut transaction, server_id, &ids)
            .await?;
        transaction.commit().await?;
        Ok((visible, stamp))
    }

    pub async fn server_members_for_actor(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(Vec<ServerMemberRow>, AuthorizationStamp), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.require_actor_scope_in(
            &mut transaction,
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
        self.server_authority(&mut transaction, actor.user_id().as_str(), server_id)
            .await?;
        let rows = sqlx::query_as::<_, ServerMemberRow>(
            "SELECT * FROM server_members WHERE server_id=? ORDER BY joined_at,user_id",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        let stamp = self
            .authorization_stamp(&mut transaction, server_id, &[])
            .await?;
        transaction.commit().await?;
        Ok((rows, stamp))
    }
}
