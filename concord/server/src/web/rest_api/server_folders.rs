use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Serialize, State, StatusCode,
    organization_error_response,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerFolderRequest {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub server_ids: Vec<String>,
    pub collapsed: Option<bool>,
}

/// Account-scoped folder layout. Collapse state is returned for wire compatibility
/// but remains a presentation preference supplied by the client.
pub async fn get_server_folders(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> impl IntoResponse {
    match state
        .engine
        .list_server_folders_for_actor(&user.actor)
        .await
    {
        Ok(folders) => Json(
            folders
                .into_iter()
                .map(|folder| ServerFolderRequest {
                    id: folder.id,
                    name: folder.name,
                    color: folder.color,
                    server_ids: folder.server_ids,
                    collapsed: Some(false),
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(error) => organization_error_response(error),
    }
}

pub async fn replace_server_folders(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(folders): Json<Vec<ServerFolderRequest>>,
) -> impl IntoResponse {
    let folders = folders
        .into_iter()
        .map(|folder| crate::engine::account::ServerFolder {
            id: folder.id,
            name: folder.name,
            color: folder.color,
            server_ids: folder.server_ids,
        })
        .collect::<Vec<_>>();
    match state
        .engine
        .replace_server_folders_for_actor(&user.actor, &folders)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) if error.starts_with("INVALID_INPUT:") => {
            (StatusCode::BAD_REQUEST, error).into_response()
        }
        Err(error) => organization_error_response(error),
    }
}

pub async fn get_server_limits(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "max_message_length": state.max_message_length,
        "max_file_size_mb": state.max_file_size / (1024 * 1024),
    }))
}
