use super::{ChatEngine, ChatEvent};

pub(super) async fn join_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.join_channel(session_id, &server_id, &channel).await)
}

pub(super) async fn part_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, reason): (String, String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.part_channel(session_id, &server_id, &channel, reason))
}

pub(super) async fn set_topic(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, topic): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_topic(session_id, &server_id, &channel, topic)
            .await
    })
}

pub(super) async fn fetch_history(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, before, limit): (String, String, Option<String>, Option<i64>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        if let Some(actor) = engine.get_authenticated_actor(session_id) {
            let limit = limit.unwrap_or(50).clamp(1, 200);
            match engine
                .fetch_history(&server_id, &channel, before.as_deref(), limit, &actor)
                .await
            {
                Ok((messages, has_more, stamp)) => {
                    if !engine.authorization_stamp_is_current(&actor, &stamp).await {
                        return std::ops::ControlFlow::Break(());
                    }
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send_guarded(
                            ChatEvent::History {
                                server_id,
                                channel,
                                messages,
                                has_more,
                            },
                            Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                stamp,
                            ])),
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else {
            Err("resource unavailable".into())
        }
    })
}

pub(super) async fn list_channels(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        if let Some(actor) = engine.get_authenticated_actor(session_id) {
            match engine
                .list_visible_channels_for_actor(&server_id, &actor)
                .await
            {
                Ok((channels, stamp)) => {
                    if !engine.authorization_stamp_is_current(&actor, &stamp).await {
                        return std::ops::ControlFlow::Break(());
                    }
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send_guarded(
                            ChatEvent::ChannelList {
                                server_id,
                                channels,
                            },
                            Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                stamp,
                            ])),
                        );
                    }
                    Ok(())
                }
                Err(error) => Err(error),
            }
        } else {
            Err("resource unavailable".into())
        }
    })
}

pub(super) async fn get_members(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        // Verify the user is a member of this server
        let is_member = engine
            .get_session(session_id)
            .and_then(|s| {
                s.user_id
                    .as_ref()
                    .map(|uid| engine.user_is_server_member(&server_id, uid))
            })
            .unwrap_or(false);
        if !is_member {
            Err("You are not a member of this server".into())
        } else {
            let Some(actor) = engine.get_authenticated_actor(session_id) else {
                return std::ops::ControlFlow::Break(());
            };
            match engine
                .get_visible_members(&actor, &server_id, &channel)
                .await
            {
                Ok((member_infos, stamp)) => {
                    if !engine.authorization_stamp_is_current(&actor, &stamp).await {
                        return std::ops::ControlFlow::Break(());
                    }
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send_guarded(
                            ChatEvent::Names {
                                server_id,
                                channel,
                                members: member_infos,
                            },
                            Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                stamp,
                            ])),
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    })
}
