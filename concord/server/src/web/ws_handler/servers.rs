use super::{ChatEngine, ChatEvent};

pub(super) async fn list_servers(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        if let Some(session) = engine.get_session(session_id) {
            let servers = if let Some(ref uid) = session.user_id {
                engine.list_servers_for_user(uid).await
            } else {
                vec![] // unauthenticated sessions see no servers
            };
            let _ = session.send(ChatEvent::ServerList { servers });
        }
        Ok(())
    })
}

pub(super) async fn create_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (name, icon_url): (String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_authenticated_actor(session_id) {
            Some(actor) => match engine.create_server_for_actor(&actor, name, icon_url).await {
                Ok(_server_id) => {
                    if let Some(session) = engine.get_session(session_id)
                        && let Some(ref uid) = session.user_id
                    {
                        let servers = engine.list_servers_for_user(uid).await;
                        let _ = session.send(ChatEvent::ServerList { servers });
                    }
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
            None => Err("UNAUTHENTICATED: authentication required".into()),
        }
    })
}

pub(super) async fn join_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_authenticated_actor(session_id) {
            Some(actor) => match engine.join_server_for_actor(&actor, &server_id).await {
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

pub(super) async fn leave_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_authenticated_actor(session_id) {
            Some(actor) => match engine.leave_server_for_actor(&actor, &server_id).await {
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
