use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, State, StatusCode,
    organization_error_response,
};

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
}

/// GET /api/users/:id/profile — get a user's full profile
pub async fn get_user_full_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(user_id): Path<String>,
) -> impl IntoResponse {
    match state.engine.get_user_profile(&auth.actor, &user_id).await {
        Ok((profile, stamp))
            if match stamp.as_ref() {
                Some(stamp) => {
                    state
                        .engine
                        .authorization_stamp_is_current(&auth.actor, stamp)
                        .await
                }
                None => state.auth.validate_actor(&auth.actor).await.is_ok(),
            } =>
        {
            Json(profile).into_response()
        }
        Ok(_) | Err(_) => (StatusCode::NOT_FOUND, "Resource unavailable").into_response(),
    }
}

/// PATCH /api/profile — update own profile
pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .update_profile_for_actor(
            &auth.actor,
            crate::engine::media_service::ProfileUpdate {
                bio: body.bio.as_deref(),
                pronouns: body.pronouns.as_deref(),
                avatar_url: body.avatar_url.as_deref(),
                banner_url: body.banner_url.as_deref(),
            },
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) if error.starts_with("CONFLICT:") => {
            (StatusCode::CONFLICT, error).into_response()
        }
        Err(error) => organization_error_response(error),
    }
}
