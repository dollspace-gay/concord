use super::{
    AppState, Arc, AuthUser, Deserialize, HistoryMessage, IntoResponse, Json, Path, Query,
    Serialize, State, StatusCode, error,
};

#[derive(Deserialize)]
pub struct HistoryParams {
    pub server_id: Option<String>,
    pub before: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct HistoryResponse {
    pub channel: String,
    pub messages: Vec<HistoryMessage>,
    pub has_more: bool,
}

#[derive(Deserialize)]
pub struct ChannelListParams {
    pub server_id: Option<String>,
}

pub async fn get_channel_history(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(channel_name): Path<String>,
    Query(params): Query<HistoryParams>,
) -> impl IntoResponse {
    let Some(server_id) = params.server_id else {
        return (
            StatusCode::BAD_REQUEST,
            "server_id query parameter is required",
        )
            .into_response();
    };

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
            &_auth.actor,
        )
        .await
    {
        Ok((messages, has_more, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&_auth.actor, &stamp)
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

pub async fn get_channels(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Query(params): Query<ChannelListParams>,
) -> impl IntoResponse {
    let Some(server_id) = params.server_id else {
        return (
            StatusCode::BAD_REQUEST,
            "server_id query parameter is required",
        )
            .into_response();
    };
    match state
        .engine
        .list_visible_channels_for_actor(&server_id, &_auth.actor)
        .await
    {
        Ok((channels, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&_auth.actor, &stamp)
                .await =>
        {
            Json(channels).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
    }
}
