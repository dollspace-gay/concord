use super::{ChatEngine, ChatEvent};

pub(super) struct SearchMessages {
    pub(super) request_id: Option<String>,
    pub(super) server_id: String,
    pub(super) query: String,
    pub(super) channel: Option<String>,
    pub(super) limit: Option<i64>,
    pub(super) offset: Option<i64>,
    pub(super) continuation: Option<String>,
}

pub(super) async fn search_messages(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    payload: SearchMessages,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    let SearchMessages {
        request_id,
        server_id,
        query,
        channel,
        limit,
        offset,
        continuation,
    } = payload;
    std::ops::ControlFlow::Continue({
        let limit = limit.unwrap_or(25).min(50);
        let offset = offset.unwrap_or(0);
        let Some(actor) = engine.get_authenticated_actor(session_id) else {
            return std::ops::ControlFlow::Break(());
        };
        match engine
            .search_messages(
                &actor,
                crate::engine::chat_engine::SearchMessagesRequest {
                    server_id: &server_id,
                    query: &query,
                    channel_name: channel.as_deref(),
                    limit,
                    offset,
                    continuation: continuation.as_deref(),
                },
            )
            .await
        {
            Ok(page) => {
                if !engine
                    .authorization_stamp_is_current(&actor, &page.stamp)
                    .await
                {
                    return std::ops::ControlFlow::Break(());
                }
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send_guarded(
                        ChatEvent::SearchResults {
                            request_id,
                            server_id,
                            query,
                            results: page.results,
                            total_count: page.total_count,
                            offset: page.offset,
                            next_continuation: page.next_continuation,
                            restarted: page.restarted,
                        },
                        Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                            page.stamp,
                        ])),
                    );
                }
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    })
}
