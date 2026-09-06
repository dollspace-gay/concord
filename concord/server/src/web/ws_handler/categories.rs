use super::{ChatEngine, ChatEvent};

pub(super) async fn list_categories(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.list_categories(&server_id).await {
            Ok(categories) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::CategoryList {
                        server_id,
                        categories,
                    });
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn create_category(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_CHANNELS,
            )
            .await
        {
            Ok(_) => match engine.create_category(session_id, &server_id, &name).await {
                Ok(category) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::CategoryUpdate {
                            server_id,
                            category,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn update_category(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, category_id, name): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_CHANNELS,
            )
            .await
        {
            Ok(_) => match engine
                .update_category(session_id, &server_id, &category_id, &name)
                .await
            {
                Ok(category) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::CategoryUpdate {
                            server_id,
                            category,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn delete_category(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, category_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_CHANNELS,
            )
            .await
        {
            Ok(_) => match engine
                .delete_category(session_id, &server_id, &category_id)
                .await
            {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::CategoryDelete {
                            server_id,
                            category_id,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn reorder_channels(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channels): (String, Vec<crate::engine::events::ChannelPositionInfo>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_CHANNELS,
            )
            .await
        {
            Ok(_) => match engine
                .reorder_channels(session_id, &server_id, &channels)
                .await
            {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::ChannelReorder {
                            server_id,
                            channels,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    })
}
