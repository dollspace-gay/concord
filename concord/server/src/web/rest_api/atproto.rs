use super::{
    AppState, Arc, AuthUser, IntoResponse, Json, Path, State, StatusCode, error, info, warn,
};

/// POST /api/bluesky/sync-profile — fetch and store the user's Bluesky profile.
pub async fn sync_bluesky_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    let did = match state.engine.verified_atproto_profile_did(&auth.actor).await {
        Ok(did) => did,
        Err(error) => return profile_sync_error_response(error),
    };

    let profile = match crate::web::atproto::fetch_full_bsky_profile(
        &state.egress.general,
        state.egress.profile_sync_endpoint(),
        &did,
    )
    .await
    {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Could not fetch Bluesky profile"})),
            )
                .into_response();
        }
    };

    let input = crate::engine::profile_sync::BlueskyProfileSyncInput {
        did: &profile.did,
        handle: &profile.handle,
        display_name: profile.display_name.as_deref(),
        description: profile.description.as_deref(),
        avatar: profile.avatar.as_deref(),
        banner: profile.banner.as_deref(),
        followers_count: profile.followers_count,
        follows_count: profile.follows_count,
    };
    if let Err(error) = state
        .engine
        .apply_atproto_profile_sync(&auth.actor, &did, &input)
        .await
    {
        return profile_sync_error_response(error);
    }

    Json(serde_json::json!({
        "did": profile.did,
        "handle": profile.handle,
        "display_name": profile.display_name,
        "description": profile.description,
        "avatar": profile.avatar,
        "banner": profile.banner,
        "followers_count": profile.followers_count,
        "follows_count": profile.follows_count,
        "posts_count": profile.posts_count,
    }))
    .into_response()
}

pub(super) fn profile_sync_error_response(
    error: crate::engine::profile_sync::ProfileSyncError,
) -> axum::response::Response {
    use crate::engine::profile_sync::ProfileSyncError;
    match error {
        ProfileSyncError::Authentication => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"Authentication required"})),
        )
            .into_response(),
        ProfileSyncError::IdentityUnavailable | ProfileSyncError::IdentityChanged => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response(),
        ProfileSyncError::DependencyUnavailable | ProfileSyncError::Database(_) => {
            error!(error = %error, "Bluesky profile sync dependency failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"Profile sync is temporarily unavailable"})),
            )
                .into_response()
        }
    }
}

/// GET /api/users/{id}/bluesky — get Bluesky identity info for a user.
pub async fn get_bluesky_identity(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(user_id): Path<String>,
) -> impl IntoResponse {
    let (identity, stamp) = match state
        .engine
        .atproto_identity_for_actor(&auth.actor, &user_id)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            error!(%error, "Failed to fetch Bluesky identity");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Database error"})),
            )
                .into_response();
        }
    };
    if let Some(stamp) = stamp.as_ref()
        && !state
            .engine
            .authorization_stamp_is_current(&auth.actor, stamp)
            .await
    {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"No Bluesky account found for user"})),
        )
            .into_response();
    }
    match identity {
        Some(identity) => Json(identity).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No Bluesky account found for user"})),
        )
            .into_response(),
    }
}

/// POST /api/messages/{id}/share-bluesky — share a message to Bluesky.
pub async fn share_to_bluesky(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(message_id): Path<String>,
) -> impl IntoResponse {
    info!(user_id = %auth.user_id, message_id = %message_id, "share_to_bluesky request received");
    match state
        .engine
        .request_atproto_publication(&auth.actor, &message_id)
        .await
    {
        Ok(publication) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "success": true,
                "publication_id": publication.id,
                "status": publication.status,
                "post_uri": publication.remote_uri,
                "cid": publication.remote_cid,
            })),
        )
            .into_response(),
        Err(crate::engine::chat_engine::AtprotoPublicationError::Unavailable) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"Message is not eligible for publication"})),
        )
            .into_response(),
        Err(crate::engine::chat_engine::AtprotoPublicationError::Authentication) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"Authentication is no longer valid"})),
        )
            .into_response(),
        Err(crate::engine::chat_engine::AtprotoPublicationError::DependencyUnavailable) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"Publication service unavailable"})),
        )
            .into_response(),
        Err(crate::engine::chat_engine::AtprotoPublicationError::Database(error)) => {
            error!(error=%error, "Failed to schedule Bluesky publication");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"Publication service unavailable"})),
            )
                .into_response()
        }
    }
}

pub async fn list_atproto_publications(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state.engine.list_atproto_publications(&auth.actor).await {
        Ok(publications) => Json(publications).into_response(),
        Err(error) => {
            warn!(%error, "Failed to list AT publications");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"Publication inventory unavailable"})),
            )
                .into_response()
        }
    }
}

pub async fn retry_atproto_publication(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(publication_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .retry_atproto_publication(&auth.actor, &publication_id)
        .await
    {
        Ok(publication) => (StatusCode::ACCEPTED, Json(publication)).into_response(),
        Err(crate::engine::chat_engine::AtprotoPublicationError::Authentication) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"AT Protocol reauthentication required"})),
        )
            .into_response(),
        Err(crate::engine::chat_engine::AtprotoPublicationError::Unavailable) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"Publication is not currently eligible for retry"})),
        )
            .into_response(),
        Err(error) => {
            warn!(%error, "Failed to retry AT publication");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"Publication retry unavailable"})),
            )
                .into_response()
        }
    }
}

/// GET /api/settings/atproto-sync — get the user's AT Protocol record sync setting.
pub async fn get_atproto_sync_setting(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state
        .engine
        .atproto_sync_enabled_for_actor(&auth.actor)
        .await
    {
        Ok(enabled) => Json(serde_json::json!({ "atproto_sync_enabled": enabled })).into_response(),
        Err(e) => {
            error!(error = %e, "Failed to get atproto sync setting");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Database error"})),
            )
                .into_response()
        }
    }
}

/// PATCH /api/settings/atproto-sync — toggle AT Protocol record sync.
pub async fn update_atproto_sync_setting(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let enabled = match body.get("enabled").and_then(|v| v.as_bool()) {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'enabled' boolean field"})),
            )
                .into_response();
        }
    };

    if let Some(channel_id) = body.get("channel_id").and_then(|value| value.as_str()) {
        return match state
            .engine
            .set_atproto_publication_grant(&auth.actor, channel_id, enabled)
            .await
        {
            Ok(()) => Json(serde_json::json!({
                "channel_id": channel_id,
                "publication_enabled": enabled,
            }))
            .into_response(),
            Err(error) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": error})),
            )
                .into_response(),
        };
    }

    match state
        .engine
        .set_atproto_sync_enabled_for_actor(&auth.actor, enabled)
        .await
    {
        Ok(()) => Json(serde_json::json!({ "atproto_sync_enabled": enabled })).into_response(),
        Err(e) => {
            error!(error = %e, "Failed to update atproto sync setting");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Database error"})),
            )
                .into_response()
        }
    }
}

pub async fn configure_atproto_channel(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(enabled) = body.get("enabled").and_then(|value| value.as_bool()) else {
        return (StatusCode::BAD_REQUEST, "Missing 'enabled' boolean field").into_response();
    };
    match state
        .engine
        .configure_atproto_channel(&auth.actor, &channel_id, enabled)
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "channel_id": channel_id,
            "atproto_publication_enabled": enabled,
        }))
        .into_response(),
        Err(error) => (StatusCode::NOT_FOUND, error).into_response(),
    }
}

pub async fn get_atproto_channel_policy(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .atproto_channel_publication_policy(&auth.actor, &channel_id)
        .await
    {
        Ok(policy) => Json(policy).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Channel unavailable").into_response(),
    }
}
