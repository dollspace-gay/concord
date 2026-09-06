use super::{
    Actor, AuthorizationStamp, ChannelAction, ChannelId, CommunityError, CommunityService,
    CreatedInvite, InviteRow, Permissions, ServerId, Uuid,
};

impl CommunityService {
    pub async fn create_invite(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        max_uses: Option<i32>,
        expires_at: Option<&str>,
        channel_id: Option<&ChannelId>,
    ) -> Result<CreatedInvite, CommunityError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.map(ChannelId::as_str);
        if max_uses.is_some_and(|value| value <= 0) {
            return Err(CommunityError::InvalidInput("invalid invite use limit"));
        }
        if expires_at.is_some_and(|value| value.len() > 64 || value.chars().any(char::is_control)) {
            return Err(CommunityError::InvalidInput("invalid invite expiry"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::CREATE_INVITES,
            )
            .await?;
        if let Some(expires_at) = expires_at {
            let future: bool =
                sqlx::query_scalar("SELECT unixepoch(?) IS NOT NULL AND unixepoch(?)>unixepoch()")
                    .bind(expires_at)
                    .bind(expires_at)
                    .fetch_one(&mut *tx)
                    .await?;
            if !future {
                return Err(CommunityError::InvalidInput("invalid invite expiry"));
            }
        }
        if let Some(channel_id) = channel_id {
            let scoped:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=? AND parent_channel_id IS NULL)")
                .bind(channel_id).bind(server_id).fetch_one(&mut *tx).await?;
            if !scoped {
                return Err(CommunityError::Forbidden);
            }
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await?;
        }
        let id = Uuid::new_v4().to_string();
        use rand::RngExt;
        let code: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(24)
            .map(char::from)
            .collect();
        let created_at:String=sqlx::query_scalar("INSERT INTO invites(id,server_id,code,created_by,max_uses,expires_at,channel_id) VALUES(?,?,?,?,?,?,?) RETURNING created_at")
            .bind(&id).bind(server_id).bind(&code).bind(actor.user_id().as_str()).bind(max_uses).bind(expires_at).bind(channel_id)
            .fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(CreatedInvite {
            id,
            code,
            created_at,
        })
    }

    pub async fn delete_invite(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        invite_id: &str,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let deleted = sqlx::query("DELETE FROM invites WHERE id=? AND server_id=?")
            .bind(invite_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_invites(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(Vec<InviteRow>, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let rows = sqlx::query_as::<_, InviteRow>(
            "SELECT * FROM invites WHERE server_id=? ORDER BY created_at DESC",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((rows, stamp))
    }
}
