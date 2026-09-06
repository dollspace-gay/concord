use super::{
    Actor, AuthorizationStamp, ChannelAction, ChannelFollowRow, ChannelId, CommunityError,
    CommunityService, CreatedFollow, Permissions, ServerId, Uuid,
};

impl CommunityService {
    pub async fn set_announcement(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        value: bool,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let updated = sqlx::query("UPDATE channels SET is_announcement=? WHERE id=? AND server_id=? AND channel_type='text' AND parent_channel_id IS NULL")
            .bind(i64::from(value)).bind(channel_id).bind(server_id).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn follow_channel(
        &self,
        actor: &Actor,
        source: &ChannelId,
        target: &ChannelId,
    ) -> Result<CreatedFollow, CommunityError> {
        let source = source.as_str();
        let target = target.as_str();
        if source == target {
            return Err(CommunityError::InvalidInput(
                "announcement follows cannot form a cycle",
            ));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT id,server_id,is_announcement FROM channels WHERE id IN (?,?)")
                .bind(source)
                .bind(target)
                .fetch_all(&mut *tx)
                .await?;
        let _source_server = rows
            .iter()
            .find(|row| row.0 == source && row.2 != 0)
            .map(|row| row.1.as_str())
            .ok_or(CommunityError::Forbidden)?;
        let target_server = rows
            .iter()
            .find(|row| row.0 == target)
            .map(|row| row.1.as_str())
            .ok_or(CommunityError::Forbidden)?;
        self.authorization
            .authorize_actor_in(
                &mut tx,
                &self.auth,
                actor,
                source,
                ChannelAction::ManageMessages,
            )
            .await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                target_server,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        self.authorization
            .authorize_actor_in(&mut tx, &self.auth, actor, target, ChannelAction::Manage)
            .await?;
        let cycle: bool = sqlx::query_scalar(
            "WITH RECURSIVE reachable(id) AS (SELECT target_channel_id FROM channel_follows WHERE source_channel_id=? UNION SELECT f.target_channel_id FROM channel_follows f JOIN reachable r ON f.source_channel_id=r.id) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?)",
        ).bind(target).bind(source).fetch_one(&mut *tx).await?;
        if cycle {
            return Err(CommunityError::InvalidInput(
                "announcement follows cannot form a cycle",
            ));
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO channel_follows(id,source_channel_id,target_channel_id,created_by) VALUES(?,?,?,?)")
            .bind(&id).bind(source).bind(target).bind(actor.user_id().as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(CreatedFollow {
            id,
            created_by: actor.user_id().as_str().to_owned(),
        })
    }

    pub async fn unfollow_channel(
        &self,
        actor: &Actor,
        follow_id: &str,
    ) -> Result<ServerId, CommunityError> {
        let (_permit, mut tx) = self.writes.begin().await?;
        let target_server: Option<String> = sqlx::query_scalar(
            "SELECT c.server_id FROM channel_follows f JOIN channels c ON c.id=f.target_channel_id WHERE f.id=?",
        ).bind(follow_id).fetch_optional(&mut *tx).await?;
        let target_server = target_server.ok_or(CommunityError::Forbidden)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                &target_server,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        sqlx::query("DELETE FROM channel_follows WHERE id=?")
            .bind(follow_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(ServerId::from_stored(target_server)?)
    }

    pub async fn list_channel_follows(
        &self,
        actor: &Actor,
        channel_id: &ChannelId,
    ) -> Result<(Vec<ChannelFollowRow>, AuthorizationStamp), CommunityError> {
        let channel_id = channel_id.as_str();
        let mut tx = self.pool.begin().await?;
        let server_id: String = sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
            .bind(channel_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CommunityError::Forbidden)?;
        self.authorization
            .require_channel_actor_permission_in(
                &mut tx,
                &self.auth,
                actor,
                &server_id,
                channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let rows = sqlx::query_as::<_, ChannelFollowRow>(
            "SELECT * FROM channel_follows WHERE source_channel_id=? OR target_channel_id=? \
             ORDER BY created_at,id",
        )
        .bind(channel_id)
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, &server_id, &[channel_id.to_owned()])
            .await?;
        tx.commit().await?;
        Ok((rows, stamp))
    }
}
