use super::{
    AppState, Arc, Deserialize, IntoResponse, Json, Path, Query, State, StatusCode, error,
};

/// GET /api/invite/{code} — public invite preview
pub async fn get_invite_preview(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    match state.engine.public_invite_preview(&code).await {
        Ok(Some(preview)) => Json(serde_json::json!({
            "code": preview.code,
            "server_id": preview.server_id,
            "server_name": preview.server_name,
            "server_icon_url": preview.server_icon_url,
            "is_vanity": preview.is_vanity,
        }))
        .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(crate::engine::community_service::PublicInvitePreviewError::ExpiredOrExhausted) => (
            StatusCode::GONE,
            Json(serde_json::json!({"error": "Invite is expired or exhausted"})),
        )
            .into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Invite lookup unavailable").into_response(),
    }
}

#[derive(Deserialize)]
pub struct DiscoverParams {
    pub category: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// GET /api/discover — public server discovery with pagination
pub async fn discover_servers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DiscoverParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);
    match state
        .engine
        .discover_public_servers(params.category.as_deref(), limit, offset)
        .await
    {
        Ok(servers) => {
            let results: Vec<serde_json::Value> = servers
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "name": s.name,
                        "icon_url": s.icon_url,
                        "description": s.description,
                        "category": s.category,
                    })
                })
                .collect();
            Json(serde_json::json!({ "servers": results, "limit": limit, "offset": offset }))
                .into_response()
        }
        Err(e) => {
            error!(error = %e, "Failed to list discoverable servers");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}
