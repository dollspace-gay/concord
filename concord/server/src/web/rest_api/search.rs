use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Query, State, StatusCode, error,
};

#[derive(Deserialize)]
pub struct SearchParams {
    pub server_id: String,
    pub q: String,
    pub channel: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub continuation: Option<String>,
}

/// GET /api/search — search messages
pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let q_len = params.q.len();
    if q_len == 0 || q_len > 1_024 {
        return (StatusCode::BAD_REQUEST, "Query must be 1-1024 characters").into_response();
    }

    let limit = params.limit.unwrap_or(25).min(50);
    let offset = params.offset.unwrap_or(0);

    match state
        .engine
        .search_messages(
            &auth.actor,
            crate::engine::chat_engine::SearchMessagesRequest {
                server_id: &params.server_id,
                query: &params.q,
                channel_name: params.channel.as_deref(),
                limit,
                offset,
                continuation: params.continuation.as_deref(),
            },
        )
        .await
    {
        Ok(page) => {
            if !state
                .engine
                .authorization_stamp_is_current(&auth.actor, &page.stamp)
                .await
            {
                return (StatusCode::NOT_FOUND, "Resource unavailable").into_response();
            }
            let results: Vec<serde_json::Value> = page
                .results
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "from": r.from,
                        "content": r.content,
                        "timestamp": r.timestamp,
                        "channel_id": r.channel_id,
                        "edited_at": r.edited_at,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "query": params.q,
                "results": results,
                "total_count": page.total_count,
                "offset": page.offset,
                "next_continuation": page.next_continuation,
                "restarted": page.restarted,
            }))
            .into_response()
        }
        Err(e) => {
            error!(error = %e, "Search failed");
            let status = match &e {
                crate::engine::chat_engine::SearchError::InvalidInput(_)
                | crate::engine::chat_engine::SearchError::InvalidContinuation => {
                    StatusCode::BAD_REQUEST
                }
                crate::engine::chat_engine::SearchError::ResourceUnavailable => {
                    StatusCode::NOT_FOUND
                }
                crate::engine::chat_engine::SearchError::DependencyUnavailable(_) => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            };
            (
                status,
                Json(serde_json::json!({
                    "code": e.code(),
                    "message": e.to_string(),
                })),
            )
                .into_response()
        }
    }
}
