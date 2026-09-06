use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, Serialize, State, StatusCode,
    error, organization_error_response,
};

#[derive(Serialize)]
pub struct EmojiResponse {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub image_url: String,
}

pub async fn list_server_emoji(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .list_server_emoji_for_actor(&auth.actor, &server_id)
        .await
    {
        Ok((rows, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&auth.actor, &stamp)
                .await =>
        {
            let list: Vec<EmojiResponse> = rows
                .into_iter()
                .map(|r| EmojiResponse {
                    id: r.id,
                    server_id: r.server_id,
                    name: r.name,
                    image_url: r.image_url,
                })
                .collect();
            Json(list).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to list emoji");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct CreateEmojiRequest {
    pub name: String,
    pub image_url: String,
}

pub async fn create_server_emoji(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    user: AuthUser,
    Json(body): Json<CreateEmojiRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .create_emoji_for_actor(&user.actor, &server_id, &body.name, &body.image_url)
        .await
    {
        Ok(created) => Json(EmojiResponse {
            id: created.id,
            server_id,
            name: created.name,
            image_url: created.image_url,
        })
        .into_response(),
        Err(error) if error.starts_with("CONFLICT:") => {
            (StatusCode::CONFLICT, error).into_response()
        }
        Err(error) => organization_error_response(error),
    }
}

pub async fn delete_server_emoji(
    State(state): State<Arc<AppState>>,
    Path((server_id, emoji_id)): Path<(String, String)>,
    user: AuthUser,
) -> impl IntoResponse {
    match state
        .engine
        .delete_emoji_for_actor(&user.actor, &server_id, &emoji_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Emoji not found").into_response(),
        Err(error) => organization_error_response(error),
    }
}
