use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, State, StatusCode,
    organization_error_response,
};

/// GET /api/admin/servers — list all servers (system admin).
pub async fn admin_list_servers(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state.engine.list_all_servers_for_admin(&auth.actor).await {
        Ok(servers) => Json(servers).into_response(),
        Err(error) => organization_error_response(error),
    }
}

/// DELETE /api/admin/servers/:id — delete any server (system admin).
pub async fn admin_delete_server(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state
        .engine
        .admin_delete_server(&auth.actor, &server_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => organization_error_response(error),
    }
}

#[derive(Deserialize)]
pub struct SetAdminRequest {
    pub is_admin: bool,
}

/// PUT /api/admin/users/:id/admin — toggle system admin flag.
pub async fn admin_set_admin(
    State(state): State<Arc<AppState>>,
    Path(target_user_id): Path<String>,
    auth: AuthUser,
    Json(body): Json<SetAdminRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .set_system_admin_for_actor(&auth.actor, &target_user_id, body.is_admin)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => organization_error_response(error),
    }
}
