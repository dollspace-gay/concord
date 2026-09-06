use super::{ChatEngine, ChatEvent, Permissions};

pub(super) async fn list_channel_permission_overrides(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(
        match engine
            .list_channel_permission_overrides(session_id, &server_id, &channel_id)
            .await
        {
            Ok(overrides) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send_guarded(
                        ChatEvent::ChannelPermissionOverrideList {
                            server_id: server_id.clone(),
                            channel_id,
                            overrides,
                        },
                        Some(
                            crate::engine::user_session::DeliveryGuard::ServerPermissions(vec![(
                                server_id,
                                Permissions::MANAGE_CHANNELS,
                            )]),
                        ),
                    );
                }
                Ok(())
            }
            Err(error) => Err(error),
        },
    )
}

pub(super) async fn set_channel_permission_override(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel_id, target_type, target_id, allow_bits, deny_bits): (
        String,
        String,
        String,
        String,
        i64,
        i64,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match (
            crate::engine::ids::ServerId::from_stored(server_id.clone()),
            crate::engine::ids::ChannelId::from_stored(channel_id.clone()),
        ) {
            (Ok(server_resource_id), Ok(channel_resource_id)) => match engine
                .set_channel_permission_override(
                    session_id,
                    crate::engine::organization::ChannelOverrideUpdate {
                        server_id: &server_resource_id,
                        channel_id: &channel_resource_id,
                        target_type: &target_type,
                        target_id: &target_id,
                        allow_bits,
                        deny_bits,
                    },
                )
                .await
            {
                Ok(()) => {
                    engine
                        .broadcast_channel_permission_overrides(session_id, &server_id, &channel_id)
                        .await
                }
                Err(error) => Err(error),
            },
            _ => Err("INVALID_INPUT: invalid resource id".to_owned()),
        }
    })
}

pub(super) async fn delete_channel_permission_override(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel_id, target_type, target_id): (String, String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(
        match engine
            .delete_channel_permission_override(
                session_id,
                &server_id,
                &channel_id,
                &target_type,
                &target_id,
            )
            .await
        {
            Ok(()) => {
                engine
                    .broadcast_channel_permission_overrides(session_id, &server_id, &channel_id)
                    .await
            }
            Err(error) => Err(error),
        },
    )
}
