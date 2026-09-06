use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, Query, State, StatusCode,
    error, organization_error_response,
};

#[derive(Deserialize)]
pub struct UserEmojiQuery {
    pub target_server_id: String,
}

pub async fn list_user_emoji(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<UserEmojiQuery>,
) -> impl IntoResponse {
    match state
        .engine
        .list_user_emoji_for_actor(&user.actor, &query.target_server_id)
        .await
    {
        Ok((rows, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&user.actor, &stamp)
                .await =>
        {
            let result: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "server_id": e.server_id,
                        "name": e.name,
                        "image_url": e.image_url,
                    })
                })
                .collect();
            Json(result).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to list user emoji");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct UpdateEmojiSettingsRequest {
    pub allow_external_emoji: Option<bool>,
    pub shareable_emoji: Option<bool>,
}

pub async fn update_emoji_settings(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    user: AuthUser,
    Json(body): Json<UpdateEmojiSettingsRequest>,
) -> impl IntoResponse {
    let allow_external = body.allow_external_emoji.unwrap_or(true);
    let shareable = body.shareable_emoji.unwrap_or(true);

    match state
        .engine
        .update_emoji_settings_for_actor(&user.actor, &server_id, allow_external, shareable)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => organization_error_response(error),
    }
}
