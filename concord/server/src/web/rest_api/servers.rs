use super::{
    AppState, Arc, AuthUser, Deserialize, HistoryParams, HistoryResponse, IntoResponse, Json, Path,
    Query, State, StatusCode, error,
};

/// GET /api/servers — list the current user's servers.
pub async fn list_servers(State(state): State<Arc<AppState>>, auth: AuthUser) -> impl IntoResponse {
    match state.engine.list_servers_for_actor(&auth.actor).await {
        Ok(servers) => Json(servers).into_response(),
        Err(error) => organization_error_response(error),
    }
}

#[derive(Deserialize)]
pub struct CreateServerRequest {
    pub name: String,
    pub icon_url: Option<String>,
}

pub(super) fn organization_error_response(error: String) -> axum::response::Response {
    let status = if error.starts_with("UNAUTHENTICATED:") {
        StatusCode::UNAUTHORIZED
    } else if error.starts_with("FORBIDDEN:") {
        StatusCode::NOT_FOUND
    } else if error.starts_with("DEPENDENCY_UNAVAILABLE:") {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, error).into_response()
}

/// POST /api/servers — create a new server.
pub async fn create_server(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(body): Json<CreateServerRequest>,
) -> impl IntoResponse {
    // Validate field lengths
    if body.name.is_empty() || body.name.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            "Server name must be between 1 and 100 characters",
        )
            .into_response();
    }
    if let Some(ref icon_url) = body.icon_url
        && icon_url.len() > 2000
    {
        return (
            StatusCode::BAD_REQUEST,
            "Icon URL must be 2000 characters or less",
        )
            .into_response();
    }

    match state
        .engine
        .create_server_for_actor(&auth.actor, body.name, body.icon_url)
        .await
    {
        Ok(server_id) => {
            let server = state
                .engine
                .list_all_servers()
                .into_iter()
                .find(|s| s.id == server_id);
            (StatusCode::CREATED, Json(server)).into_response()
        }
        Err(e) => organization_error_response(e),
    }
}

/// GET /api/servers/:id — get server info.
pub async fn get_server(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    match state.engine.server_for_actor(&auth.actor, &server_id).await {
        Ok((server, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&auth.actor, &stamp)
                .await =>
        {
            Json(server).into_response()
        }
        Ok(_) | Err(_) => (StatusCode::NOT_FOUND, "Server not found").into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateServerMediaRequest {
    pub icon_url: String,
}

pub async fn update_server_media(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
    Json(body): Json<UpdateServerMediaRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .update_server_icon_for_actor(&auth.actor, &server_id, &body.icon_url)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) if error.starts_with("CONFLICT:") => {
            (StatusCode::CONFLICT, error).into_response()
        }
        Err(error) => organization_error_response(error),
    }
}

pub async fn update_server_member_media(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
    Json(body): Json<UpdateServerMediaRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .update_member_avatar_for_actor(&auth.actor, &server_id, &body.icon_url)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) if error.starts_with("CONFLICT:") => {
            (StatusCode::CONFLICT, error).into_response()
        }
        Err(error) => organization_error_response(error),
    }
}

/// DELETE /api/servers/:id — delete a server (owner only).
pub async fn delete_server(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state
        .engine
        .delete_owned_server(&server_id, &auth.actor)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => organization_error_response(e),
    }
}

/// GET /api/servers/:id/channels — list channels in a server.
pub async fn list_server_channels(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .list_visible_channels_for_actor(&server_id, &auth.actor)
        .await
    {
        Ok((channels, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&auth.actor, &stamp)
                .await =>
        {
            Json(channels).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
    }
}

/// GET /api/servers/:id/channels/:name/messages — channel history within a server.
pub async fn get_server_channel_history(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path((server_id, channel_name)): Path<(String, String)>,
    Query(params): Query<HistoryParams>,
) -> impl IntoResponse {
    let channel = if channel_name.starts_with('#') {
        channel_name
    } else {
        format!("#{}", channel_name)
    };

    let limit = params.limit.unwrap_or(50).min(200);

    match state
        .engine
        .fetch_history(
            &server_id,
            &channel,
            params.before.as_deref(),
            limit,
            &auth.actor,
        )
        .await
    {
        Ok((messages, has_more, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&auth.actor, &stamp)
                .await =>
        {
            Json(HistoryResponse {
                channel,
                messages,
                has_more,
            })
            .into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(e) if e == "resource unavailable" || e.starts_with("No such channel:") => {
            (StatusCode::NOT_FOUND, "Resource unavailable").into_response()
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch history");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to fetch history").into_response()
        }
    }
}

/// GET /api/servers/:id/members — list server members.
pub async fn list_server_members(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .server_members_for_actor(&auth.actor, &server_id)
        .await
    {
        Ok((rows, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&auth.actor, &stamp)
                .await =>
        {
            let members: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "user_id": m.user_id,
                        "role": m.role,
                        "joined_at": m.joined_at,
                    })
                })
                .collect();
            Json(members).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to list server members");
            (StatusCode::NOT_FOUND, "Resource unavailable").into_response()
        }
    }
}
