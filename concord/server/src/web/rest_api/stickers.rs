use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, State, StatusCode, error,
    organization_error_response,
};

pub async fn list_server_stickers(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .list_server_stickers_for_actor(&auth.actor, &server_id)
        .await
    {
        Ok((rows, stamp))
            if state
                .engine
                .authorization_stamp_is_current(&auth.actor, &stamp)
                .await =>
        {
            let result: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "server_id": s.server_id,
                        "name": s.name,
                        "image_url": s.image_url,
                        "description": s.description,
                    })
                })
                .collect();
            Json(result).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to list stickers");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct CreateStickerRequest {
    pub name: String,
    pub image_url: String,
    pub description: Option<String>,
}

pub async fn create_server_sticker(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    user: AuthUser,
    Json(body): Json<CreateStickerRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .create_sticker_for_actor(
            &user.actor,
            &server_id,
            &body.name,
            &body.image_url,
            body.description.as_deref(),
        )
        .await
    {
        Ok(created) => Json(serde_json::json!({
            "id": created.id,
            "server_id": server_id,
            "name": created.name,
            "image_url": created.image_url,
            "description": created.description,
        }))
        .into_response(),
        Err(error) if error.starts_with("CONFLICT:") => {
            (StatusCode::CONFLICT, error).into_response()
        }
        Err(error) => organization_error_response(error),
    }
}

pub async fn delete_server_sticker(
    State(state): State<Arc<AppState>>,
    Path((server_id, sticker_id)): Path<(String, String)>,
    user: AuthUser,
) -> impl IntoResponse {
    match state
        .engine
        .delete_sticker_for_actor(&user.actor, &server_id, &sticker_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Sticker not found").into_response(),
        Err(error) => organization_error_response(error),
    }
}
