use super::{ChatEngine, ChatEvent};

pub(super) async fn delete_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_authenticated_actor(session_id) {
            Some(actor) => match engine.delete_owned_server(&server_id, &actor).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id)
                        && let Some(ref uid) = session.user_id
                    {
                        let servers = engine.list_servers_for_user(uid).await;
                        let _ = session.send(ChatEvent::ServerList { servers });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            None => Err("UNAUTHENTICATED: authentication required".into()),
        }
    })
}

pub(super) async fn update_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name, icon_url): (String, Option<String>, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_authenticated_actor(session_id) {
            Some(actor) => {
                match engine
                    .update_server_settings_for_actor(
                        &actor,
                        &server_id,
                        name.as_deref(),
                        icon_url.as_deref(),
                    )
                    .await
                {
                    Ok(()) => {
                        // Send updated server list to the requester
                        if let Some(session) = engine.get_session(session_id)
                            && let Some(ref uid) = session.user_id
                        {
                            let servers = engine.list_servers_for_user(uid).await;
                            let _ = session.send(ChatEvent::ServerList { servers });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            None => Err("UNAUTHENTICATED: authentication required".into()),
        }
    })
}

pub(super) async fn update_member_role(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id, role): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(match engine.get_authenticated_actor(session_id) {
        Some(actor) => {
            engine
                .update_member_role_for_actor(&actor, &server_id, &user_id, &role)
                .await
        }
        None => Err("UNAUTHENTICATED: authentication required".into()),
    })
}
