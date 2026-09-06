use super::{
    Actor, AuthorizationStamp, ChannelAction, ChannelId, CommunityError, CommunityService,
    CreateEvent, Permissions, ServerEventRow, ServerId, VisibleEvent,
};

impl CommunityService {
    pub async fn create_event(
        &self,
        actor: &Actor,
        params: &CreateEvent<'_>,
    ) -> Result<String, CommunityError> {
        let server_id = params.server_id.as_str();
        let channel_id = params.channel_id.map(ChannelId::as_str);
        if params.name.trim().is_empty()
            || params.name.chars().count() > 100
            || params.name.chars().any(char::is_control)
            || params.description.is_some_and(|value| value.len() > 2_000)
            || params.image_url.is_some_and(|value| value.len() > 2_000)
            || params.created_by != actor.user_id().as_str()
        {
            return Err(CommunityError::InvalidInput("invalid event"));
        }
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
        let valid_times: bool = sqlx::query_scalar(
            "SELECT unixepoch(?) IS NOT NULL AND (? IS NULL OR (unixepoch(?) IS NOT NULL AND unixepoch(?)>unixepoch(?)))",
        ).bind(params.start_time).bind(params.end_time).bind(params.end_time)
            .bind(params.end_time).bind(params.start_time).fetch_one(&mut *tx).await?;
        if !valid_times {
            return Err(CommunityError::InvalidInput("invalid event time"));
        }
        if let Some(channel_id) = channel_id {
            let scoped: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)",
            )
            .bind(channel_id)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await?;
            if !scoped {
                return Err(CommunityError::Forbidden);
            }
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await?;
        }
        let created_at: String = sqlx::query_scalar("INSERT INTO server_events(id,server_id,name,description,channel_id,start_time,end_time,image_url,created_by) VALUES(?,?,?,?,?,?,?,?,?) RETURNING created_at")
            .bind(params.id).bind(server_id).bind(params.name.trim()).bind(params.description)
            .bind(channel_id).bind(params.start_time).bind(params.end_time).bind(params.image_url)
            .bind(actor.user_id().as_str()).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(created_at)
    }

    pub async fn list_events(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(Vec<VisibleEvent>, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await?;
        let rows = sqlx::query_as::<_, ServerEventRow>(
            "SELECT * FROM server_events WHERE server_id=? AND integrity_state='active' \
             ORDER BY start_time,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut events = Vec::with_capacity(rows.len());
        let mut channel_ids = Vec::new();
        for event in rows {
            if let Some(channel_id) = event.channel_id.as_deref() {
                match self
                    .authorization
                    .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                    .await
                {
                    Ok(()) => channel_ids.push(channel_id.to_owned()),
                    Err(crate::engine::authorization::AuthorizationError::Unavailable) => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            let rsvp_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM event_rsvps WHERE event_id=? AND status IN ('interested','going')",
            )
            .bind(&event.id)
            .fetch_one(&mut *tx)
            .await?;
            events.push(VisibleEvent { event, rsvp_count });
        }
        channel_ids.sort();
        channel_ids.dedup();
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &channel_ids)
            .await?;
        tx.commit().await?;
        Ok((events, stamp))
    }
}
