use super::{
    ChatEngine, ChatEvent, ConnectionId, CreateServerEventRequest, EventInfo, RsvpInfo,
    referenced_channel_id, referenced_server_id,
};

impl ChatEngine {
    /// Create a scheduled server event. Requires MANAGE_SERVER permission.
    pub async fn create_event(
        &self,
        session_id: ConnectionId,
        params: &CreateServerEventRequest<'_>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let server_resource_id = referenced_server_id(params.server_id)?;
        let channel_resource_id = params.channel_id.map(referenced_channel_id).transpose()?;
        let created_at = self
            .community_service()?
            .create_event(
                &actor,
                &crate::engine::community_service::CreateEvent {
                    id: params.id,
                    server_id: &server_resource_id,
                    name: params.name,
                    description: params.description,
                    channel_id: channel_resource_id.as_ref(),
                    start_time: params.start_time,
                    end_time: params.end_time,
                    image_url: params.image_url,
                    created_by: params.created_by,
                },
            )
            .await
            .map_err(String::from)?;

        let event_info = EventInfo {
            id: params.id.to_string(),
            server_id: params.server_id.to_string(),
            name: params.name.to_string(),
            description: params.description.map(String::from),
            channel_id: params.channel_id.map(String::from),
            start_time: params.start_time.to_string(),
            end_time: params.end_time.map(String::from),
            image_url: params.image_url.map(String::from),
            created_by: params.created_by.to_string(),
            status: "scheduled".to_string(),
            interested_count: 0,
            created_at,
        };

        let event = ChatEvent::EventUpdate {
            server_id: params.server_id.to_string(),
            event: event_info,
        };
        self.broadcast_to_server(params.server_id, &event);

        Ok(())
    }
    /// List events for a server. Requires VIEW_CHANNELS permission.
    pub async fn list_events(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (rows, stamp) = self
            .community_service()?
            .list_events(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let mut events = Vec::with_capacity(rows.len());
        for visible in rows {
            let row = visible.event;
            events.push(EventInfo {
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                description: row.description,
                channel_id: row.channel_id,
                start_time: row.start_time,
                end_time: row.end_time,
                image_url: row.image_url,
                created_by: row.created_by,
                status: row.status,
                interested_count: visible.rsvp_count,
                created_at: row.created_at,
            });
        }

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::EventList {
                    server_id: server_id.to_string(),
                    events,
                },
                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                    stamp,
                ])),
            );
        }

        Ok(())
    }
    /// Update an event's status. Requires MANAGE_SERVER permission.
    pub async fn update_event_status(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), String> {
        if !["scheduled", "active", "completed", "cancelled"].contains(&status) {
            return Err("Invalid status. Must be: scheduled, active, completed, cancelled".into());
        }
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let row = self
            .community_service()?
            .update_event_status(&actor, &referenced_server_id(server_id)?, event_id, status)
            .await?;
        let rsvp_count = crate::db::queries::events::get_rsvp_count(
            self.db.as_ref().ok_or("No database configured")?,
            event_id,
        )
        .await
        .unwrap_or(0);
        let event_info = EventInfo {
            id: row.id,
            server_id: row.server_id,
            name: row.name,
            description: row.description,
            channel_id: row.channel_id,
            start_time: row.start_time,
            end_time: row.end_time,
            image_url: row.image_url,
            created_by: row.created_by,
            status: row.status,
            interested_count: rsvp_count,
            created_at: row.created_at,
        };
        self.broadcast_to_server(
            server_id,
            &ChatEvent::EventUpdate {
                server_id: server_id.to_string(),
                event: event_info,
            },
        );
        Ok(())
    }
    /// Delete a scheduled event. Requires MANAGE_SERVER permission.
    pub async fn delete_event(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        self.community_service()?
            .delete_event(&actor, &referenced_server_id(server_id)?, event_id)
            .await?;
        self.broadcast_to_server(
            server_id,
            &ChatEvent::EventDelete {
                server_id: server_id.to_string(),
                event_id: event_id.to_string(),
            },
        );
        Ok(())
    }
    /// Set an RSVP for an event. Requires visibility of the linked channel.
    pub async fn set_rsvp(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), String> {
        if !["interested", "going", "not_going"].contains(&status) {
            return Err("Invalid RSVP status. Must be: interested, going, not_going".into());
        }
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (channel_id, rows) = self
            .community_service()?
            .set_rsvp(
                &actor,
                &referenced_server_id(server_id)?,
                event_id,
                (status != "not_going").then_some(status),
            )
            .await?;
        let rsvps = rows
            .into_iter()
            .map(|row| RsvpInfo {
                user_id: row.user_id,
                status: row.status,
            })
            .collect();
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::EventRsvpList {
                    event_id: event_id.to_string(),
                    rsvps,
                },
                Some(match channel_id {
                    Some(channel_id) => crate::engine::user_session::DeliveryGuard::Channels(vec![
                        channel_id.into_inner(),
                    ]),
                    None => crate::engine::user_session::DeliveryGuard::ServerMembership(vec![
                        server_id.to_string(),
                    ]),
                }),
            );
        }
        Ok(())
    }
    /// Remove an RSVP for an event. Requires visibility of the linked channel.
    pub async fn remove_rsvp(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        self.community_service()?
            .set_rsvp(&actor, &referenced_server_id(server_id)?, event_id, None)
            .await?;
        Ok(())
    }
    /// List RSVPs for an event. Sends EventRsvpList to the requesting session.
    pub async fn list_rsvps(&self, session_id: ConnectionId, event_id: &str) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (server_id, channel_id, rows) = self
            .community_service()?
            .list_rsvps(&actor, event_id)
            .await?;
        let rsvps = rows
            .into_iter()
            .map(|row| RsvpInfo {
                user_id: row.user_id,
                status: row.status,
            })
            .collect();
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::EventRsvpList {
                    event_id: event_id.to_string(),
                    rsvps,
                },
                Some(match channel_id {
                    Some(channel_id) => crate::engine::user_session::DeliveryGuard::Channels(vec![
                        channel_id.into_inner(),
                    ]),
                    None => crate::engine::user_session::DeliveryGuard::ServerMembership(vec![
                        server_id.into_inner(),
                    ]),
                }),
            );
        }
        Ok(())
    }
}
