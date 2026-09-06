use super::ChatEngine;

pub(super) struct CreateEvent {
    pub(super) server_id: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) channel_id: Option<String>,
    pub(super) start_time: String,
    pub(super) end_time: Option<String>,
    pub(super) image_url: Option<String>,
}

pub(super) async fn create_event(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    payload: CreateEvent,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    let CreateEvent {
        server_id,
        name,
        description,
        channel_id,
        start_time,
        end_time,
        image_url,
    } = payload;
    std::ops::ControlFlow::Continue({
        let user_id = engine.get_session_user_id(session_id).unwrap_or_default();
        let event_id = uuid::Uuid::new_v4().to_string();
        engine
            .create_event(
                session_id,
                &crate::engine::chat_engine::CreateServerEventRequest {
                    id: &event_id,
                    server_id: &server_id,
                    name: &name,
                    description: description.as_deref(),
                    channel_id: channel_id.as_deref(),
                    start_time: &start_time,
                    end_time: end_time.as_deref(),
                    image_url: image_url.as_deref(),
                    created_by: &user_id,
                },
            )
            .await
    })
}

pub(super) async fn list_events(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_events(session_id, &server_id).await)
}

pub(super) async fn update_event_status(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, event_id, status): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .update_event_status(session_id, &server_id, &event_id, &status)
            .await
    })
}

pub(super) async fn delete_event(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, event_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.delete_event(session_id, &server_id, &event_id).await)
}

pub(super) async fn set_rsvp(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, event_id, status): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_rsvp(session_id, &server_id, &event_id, &status)
            .await
    })
}

pub(super) async fn remove_rsvp(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, event_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.remove_rsvp(session_id, &server_id, &event_id).await)
}

pub(super) async fn list_rsvps(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (event_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_rsvps(session_id, &event_id).await)
}
