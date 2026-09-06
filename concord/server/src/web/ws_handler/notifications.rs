use super::{ChatEngine, ChatEvent};

pub(super) struct UpdateNotificationSettings {
    pub(super) server_id: String,
    pub(super) channel_id: Option<String>,
    pub(super) level: String,
    pub(super) suppress_everyone: Option<bool>,
    pub(super) suppress_roles: Option<bool>,
    pub(super) muted: Option<bool>,
    pub(super) mute_until: Option<String>,
}

pub(super) async fn update_notification_settings(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    payload: UpdateNotificationSettings,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    let UpdateNotificationSettings {
        server_id,
        channel_id,
        level,
        suppress_everyone,
        suppress_roles,
        muted,
        mute_until,
    } = payload;
    std::ops::ControlFlow::Continue({
        let params = crate::engine::chat_engine::UpdateNotificationSettingsParams {
            server_id: &server_id,
            channel_id: channel_id.as_deref(),
            level: &level,
            suppress_everyone: suppress_everyone.unwrap_or(false),
            suppress_roles: suppress_roles.unwrap_or(false),
            muted: muted.unwrap_or(false),
            mute_until: mute_until.as_deref(),
        };
        engine
            .update_notification_settings(session_id, &params)
            .await
    })
}

pub(super) async fn get_notification_settings(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .get_notification_settings(session_id, &server_id)
            .await
        {
            Ok(settings) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::NotificationSettings {
                        server_id,
                        settings,
                    });
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
}
