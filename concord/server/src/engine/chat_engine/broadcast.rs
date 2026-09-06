use super::{ChatEngine, ChatEvent, ConnectionId, warn};

impl ChatEngine {
    /// Broadcast an event to all members of a channel, optionally excluding one session.
    pub(super) fn broadcast_to_channel(
        &self,
        channel_id: &str,
        event: &ChatEvent,
        exclude: Option<ConnectionId>,
    ) {
        let Some(channel) = self.channels.get(channel_id) else {
            return;
        };

        for member_id in &channel.members {
            if Some(*member_id) == exclude {
                continue;
            }
            let channel_guard_id = match event {
                ChatEvent::ThreadCreate { thread, .. } | ChatEvent::ThreadUpdate { thread, .. } => {
                    thread.id.clone()
                }
                _ => channel_id.to_owned(),
            };
            if let Some(session) = self.sessions.get(member_id)
                && !session.send_guarded(
                    event.clone(),
                    Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                        channel_guard_id,
                    ])),
                )
            {
                warn!(%member_id, "failed to send event to session (channel closed)");
            }
        }
    }
    pub(super) fn broadcast_to_channel_guarded(
        &self,
        channel_id: &str,
        conversation_id: &str,
        event: &ChatEvent,
        exclude: Option<ConnectionId>,
    ) {
        let Some(channel) = self.channels.get(channel_id) else {
            return;
        };
        for member_id in &channel.members {
            if Some(*member_id) == exclude {
                continue;
            }
            if let Some(session) = self.sessions.get(member_id)
                && !session.send_guarded(
                    event.clone(),
                    Some(crate::engine::user_session::DeliveryGuard::Conversations(
                        vec![conversation_id.to_owned()],
                    )),
                )
            {
                warn!(%member_id, "guarded channel delivery overflowed");
            }
        }
    }
}
