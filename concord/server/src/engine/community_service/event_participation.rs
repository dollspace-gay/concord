use super::{
    Actor, ChannelAction, ChannelId, CommunityError, CommunityService, EventRsvpRow, Permissions,
    ServerEventRow, ServerId,
};

impl CommunityService {
    pub async fn update_event_status(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        event_id: &str,
        status: &str,
    ) -> Result<ServerEventRow, CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(CommunityError::from)?;
        let row = sqlx::query_as::<_, ServerEventRow>(
            "UPDATE server_events SET status=?,updated_at=datetime('now') \
             WHERE id=? AND server_id=? AND integrity_state='active' \
               AND (status=? \
                    OR (status='scheduled' AND ? IN ('active','cancelled')) \
                    OR (status='active' AND ? IN ('completed','cancelled'))) \
             RETURNING *",
        )
        .bind(status)
        .bind(event_id)
        .bind(server_id)
        .bind(status)
        .bind(status)
        .bind(status)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?
        .ok_or(CommunityError::Forbidden)?;
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(row)
    }

    pub async fn delete_event(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        event_id: &str,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(CommunityError::from)?;
        let deleted = sqlx::query(
            "DELETE FROM server_events WHERE id=? AND server_id=? AND integrity_state='active'",
        )
        .bind(event_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        if deleted.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(())
    }

    pub async fn set_rsvp(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        event_id: &str,
        status: Option<&str>,
    ) -> Result<(Option<ChannelId>, Vec<EventRsvpRow>), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        let channel_id: Option<Option<String>> = sqlx::query_scalar(
            "SELECT channel_id FROM server_events WHERE id=? AND server_id=? AND integrity_state='active'",
        )
        .bind(event_id)
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let channel_id = channel_id.ok_or(CommunityError::Forbidden)?;
        if let Some(channel_id) = channel_id.as_deref() {
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await
                .map_err(CommunityError::from)?;
        } else {
            self.authorization
                .require_server_actor_in(
                    &mut tx,
                    &self.auth,
                    actor,
                    server_id,
                    Permissions::VIEW_CHANNELS,
                )
                .await
                .map_err(CommunityError::from)?;
        }
        match status {
            Some(status) => {
                sqlx::query(
                    "INSERT INTO event_rsvps(event_id,user_id,status) VALUES(?,?,?) \
                     ON CONFLICT(event_id,user_id) DO UPDATE SET status=excluded.status",
                )
                .bind(event_id)
                .bind(actor.user_id().as_str())
                .bind(status)
                .execute(&mut *tx)
                .await
                .map_err(CommunityError::from)?;
            }
            None => {
                sqlx::query("DELETE FROM event_rsvps WHERE event_id=? AND user_id=?")
                    .bind(event_id)
                    .bind(actor.user_id().as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(CommunityError::from)?;
            }
        }
        let rows = sqlx::query_as::<_, EventRsvpRow>(
            "SELECT * FROM event_rsvps WHERE event_id=? ORDER BY created_at,user_id",
        )
        .bind(event_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        tx.commit().await.map_err(CommunityError::from)?;
        let channel_id = channel_id.map(ChannelId::from_stored).transpose()?;
        Ok((channel_id, rows))
    }

    pub async fn list_rsvps(
        &self,
        actor: &Actor,
        event_id: &str,
    ) -> Result<(ServerId, Option<ChannelId>, Vec<EventRsvpRow>), CommunityError> {
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        let scope: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT server_id,channel_id FROM server_events WHERE id=? AND integrity_state='active'",
        )
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let (server_id, channel_id) = scope.ok_or(CommunityError::Forbidden)?;
        if let Some(channel_id) = channel_id.as_deref() {
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await
                .map_err(CommunityError::from)?;
        } else {
            self.authorization
                .require_server_actor_in(
                    &mut tx,
                    &self.auth,
                    actor,
                    &server_id,
                    Permissions::VIEW_CHANNELS,
                )
                .await
                .map_err(CommunityError::from)?;
        }
        let rows = sqlx::query_as::<_, EventRsvpRow>(
            "SELECT * FROM event_rsvps WHERE event_id=? ORDER BY created_at,user_id",
        )
        .bind(event_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        tx.commit().await.map_err(CommunityError::from)?;
        Ok((
            ServerId::from_stored(server_id)?,
            channel_id.map(ChannelId::from_stored).transpose()?,
            rows,
        ))
    }
}
