use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{FromRef, FromRequestParts, Multipart, Path, Query, State};
use axum::http::header;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::auth::authority::Actor;
use crate::engine::events::HistoryMessage;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use super::app_state::AppState;
use super::auth_middleware::{AuthUser, auth_error_response};

// ── Phase 8: Bot token auth extractor ──────────────────────

/// Extractor that validates a `Authorization: Bot <token>` header.
/// Used for bot API endpoints that authenticate via bot tokens.
pub struct BotAuth {
    pub user_id: String,
    pub actor: Actor,
}

impl<S: Send + Sync> FromRequestParts<S> for BotAuth
where
    Arc<AppState>: FromRef<S>,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let app_state = Arc::<AppState>::from_ref(state);

        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

        let token = auth_header
            .strip_prefix("Bot ")
            .ok_or((StatusCode::UNAUTHORIZED, "Expected 'Bot <token>' format"))?;

        let actor = app_state
            .auth
            .authenticate_bot(token)
            .await
            .map_err(|error| {
                let response = auth_error_response(error, "Invalid bot token");
                let status = response.status();
                if status == StatusCode::SERVICE_UNAVAILABLE {
                    (status, "Authentication service unavailable")
                } else {
                    (status, "Invalid bot token")
                }
            })?;

        Ok(BotAuth {
            user_id: actor.user_id().as_str().to_owned(),
            actor,
        })
    }
}

// ── Channel endpoints (public, require server_id query param) ──

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

// ── Server endpoints (authenticated) ────────────────────

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

fn organization_error_response(error: String) -> axum::response::Response {
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

// ── Admin endpoints (system admin only) ─────────────────

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

// ── Auth status (public) ────────────────────────────────

#[derive(Serialize)]
pub struct AuthStatusResponse {
    pub authenticated: bool,
    pub providers: Vec<String>,
}

/// GET /api/auth/status — returns available providers and auth state.
pub async fn auth_status() -> impl IntoResponse {
    Json(AuthStatusResponse {
        authenticated: false, // caller can check /api/me instead
        providers: vec!["atproto".to_string()],
    })
}

// ── User profile (authenticated) ────────────────────────

#[derive(Serialize)]
pub struct UserProfile {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

/// GET /api/me — return the current user's profile.
pub async fn get_me(State(state): State<Arc<AppState>>, auth: AuthUser) -> impl IntoResponse {
    match state.engine.current_account_profile(&auth.actor).await {
        Ok(Some(profile)) => Json(UserProfile {
            id: profile.id,
            username: profile.username,
            email: profile.email,
            avatar_url: profile.avatar_url,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to fetch user profile");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── User profile lookup (public) ──────────────────────────

#[derive(Serialize)]
pub struct PublicUserProfile {
    pub username: String,
    pub avatar_url: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<String>,
}

/// GET /api/users/:nickname — look up a user's public profile by nickname.
pub async fn get_user_profile(
    State(state): State<Arc<AppState>>,
    Path(nickname): Path<String>,
) -> impl IntoResponse {
    match state.engine.public_account_profile(&nickname).await {
        Ok(Some(profile)) => Json(PublicUserProfile {
            username: profile.username,
            avatar_url: profile.avatar_url,
            provider: profile.provider,
            provider_id: profile.provider_id,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to fetch user profile");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── IRC token management (authenticated) ─────────────────

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub id: String,
    pub token: String, // plaintext, shown only once
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct IrcTokenInfo {
    pub id: String,
    pub label: Option<String>,
    pub last_used: Option<String>,
    pub created_at: String,
}

/// POST /api/tokens — generate a new IRC access token.
pub async fn create_irc_token(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(body): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    // Enforce per-user token limit (max 25 active tokens)
    const MAX_TOKENS_PER_USER: usize = 25;
    match state.engine.list_irc_tokens_for_actor(&auth.actor).await {
        Ok(existing) if existing.len() >= MAX_TOKENS_PER_USER => {
            return (
                StatusCode::BAD_REQUEST,
                "Maximum number of IRC tokens reached (25). Delete unused tokens first.",
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, "Failed to check existing IRC tokens");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
        _ => {}
    }

    let issued = match state
        .auth
        .issue_irc_token(auth.actor.user_id(), body.label.as_deref())
        .await
    {
        Ok(issued) => issued,
        Err(e) => {
            error!(error = %e, "Failed to issue IRC credential");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Token creation failed").into_response();
        }
    };

    Json(CreateTokenResponse {
        id: issued.token_id,
        token: issued.secret, // shown only once
        label: body.label,
    })
    .into_response()
}

/// GET /api/tokens — list the current user's IRC tokens (no secrets).
pub async fn list_irc_tokens(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state.engine.list_irc_tokens_for_actor(&auth.actor).await {
        Ok(rows) => {
            let tokens: Vec<IrcTokenInfo> = rows
                .into_iter()
                .map(|token| IrcTokenInfo {
                    id: token.id,
                    label: token.label,
                    last_used: token.last_used,
                    created_at: token.created_at,
                })
                .collect();
            Json(tokens).into_response()
        }
        Err(e) => {
            error!(error = %e, "Failed to list IRC tokens");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

/// DELETE /api/tokens/:id — revoke an IRC token.
pub async fn delete_irc_token(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(token_id): Path<String>,
) -> impl IntoResponse {
    match state
        .auth
        .revoke_irc_token(&token_id, auth.actor.user_id())
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Token not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to delete IRC token");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── File upload endpoints ─────────────────────────────────

#[derive(Serialize)]
pub struct UploadResponse {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub url: String,
}

/// Check whether a Content-Type is allowed for file uploads.
/// Rejects types that could execute scripts when rendered inline by a browser
/// (e.g., text/html, application/javascript, SVG) and malformed MIME strings.
fn is_allowed_upload_content_type(content_type: &str) -> bool {
    // Must look like a MIME type (contains '/') and be reasonably short
    if !content_type.contains('/') || content_type.len() > 255 {
        return false;
    }

    // Blocklist: types that browsers may execute as active content
    const BLOCKED: &[&str] = &[
        "text/html",
        "text/javascript",
        "application/javascript",
        "application/xhtml+xml",
        "image/svg+xml",
        "text/xml",
        "application/xml",
    ];

    // Compare only the base type (strip parameters like "; charset=utf-8")
    let base = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();

    !BLOCKED.contains(&base.as_str())
}

/// POST /api/uploads — upload a file (multipart form data).
#[derive(Deserialize)]
pub struct UploadQuery {
    pub purpose: Option<String>,
    pub conversation_id: Option<String>,
    pub server_id: Option<String>,
    pub channel: Option<String>,
}

pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Query(target): Query<UploadQuery>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let _upload_permit = match state.upload_admission.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return (StatusCode::TOO_MANY_REQUESTS, "Upload capacity is busy").into_response();
        }
    };
    let upload_deadline = tokio::time::Instant::now() + state.upload_total_timeout;
    let purpose = target.purpose.as_deref().unwrap_or("message");
    let mut upload_plan = match state
        .engine
        .authorize_media_upload(
            &auth.actor,
            crate::engine::media_service::UploadTarget {
                purpose,
                conversation_id: target.conversation_id.as_deref(),
                server_id: target.server_id.as_deref(),
                channel: target.channel.as_deref(),
            },
            state.max_file_size,
        )
        .await
    {
        Ok(plan) => Some(plan),
        Err(error) => return organization_error_response(error),
    };

    loop {
        let remaining = upload_deadline.saturating_duration_since(tokio::time::Instant::now());
        let field = match tokio::time::timeout(
            state.upload_idle_timeout.min(remaining),
            multipart.next_field(),
        )
        .await
        {
            Ok(Ok(Some(field))) => field,
            Ok(Ok(None)) => break,
            Ok(Err(error)) => {
                error!(error=%error, "Upload multipart stream failed");
                return (StatusCode::BAD_REQUEST, "Failed to read file data").into_response();
            }
            Err(_) => {
                return (StatusCode::REQUEST_TIMEOUT, "Upload timed out").into_response();
            }
        };
        let mut field = field;
        if field.name() == Some("file") {
            let filename = field
                .file_name()
                .unwrap_or("unnamed")
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("file")
                .to_string();
            let content_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            if !is_allowed_upload_content_type(&content_type) {
                return (StatusCode::BAD_REQUEST, "File type not allowed").into_response();
            }
            if upload_plan.as_ref().is_some_and(|plan| plan.images_only)
                && !matches!(
                    content_type.as_str(),
                    "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                )
            {
                return (
                    StatusCode::BAD_REQUEST,
                    "Managed media must be a safe image type",
                )
                    .into_response();
            }
            let mut upload = match state
                .engine
                .reserve_media_upload(
                    &auth.actor,
                    upload_plan
                        .take()
                        .expect("upload plan is consumed only when returning a response"),
                    crate::engine::media_service::UploadReservation {
                        media_root: &state.media_dir,
                        filename: &filename,
                        content_type: &content_type,
                        per_user_bytes: state.max_media_per_user,
                        total_bytes: state.max_media_total,
                    },
                )
                .await
            {
                Ok(upload) => upload,
                Err(error) => {
                    error!(error=%error, "Failed to start private upload");
                    return (StatusCode::SERVICE_UNAVAILABLE, "Media storage unavailable")
                        .into_response();
                }
            };
            loop {
                let remaining =
                    upload_deadline.saturating_duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(state.upload_idle_timeout.min(remaining), field.chunk())
                    .await
                {
                    Ok(Ok(Some(chunk))) => {
                        if let Err(error) = upload.write_chunk(&chunk).await {
                            let too_large = matches!(error, crate::media::MediaError::TooLarge);
                            upload.abort().await;
                            return (
                                if too_large {
                                    StatusCode::PAYLOAD_TOO_LARGE
                                } else {
                                    StatusCode::SERVICE_UNAVAILABLE
                                },
                                if too_large {
                                    "File too large"
                                } else {
                                    "Media storage unavailable"
                                },
                            )
                                .into_response();
                        }
                    }
                    Ok(Ok(None)) => break,
                    Ok(Err(error)) => {
                        error!(error=%error, "Upload stream failed");
                        upload.abort().await;
                        return (StatusCode::BAD_REQUEST, "Failed to read file data")
                            .into_response();
                    }
                    Err(_) => {
                        upload.abort().await;
                        return (StatusCode::REQUEST_TIMEOUT, "Upload timed out").into_response();
                    }
                }
            }
            return match upload.finish().await {
                Ok(ready) => (
                    StatusCode::CREATED,
                    Json(UploadResponse {
                        url: format!("/api/uploads/{}", ready.id),
                        id: ready.id,
                        filename,
                        content_type,
                        file_size: ready.file_size as i64,
                    }),
                )
                    .into_response(),
                Err(error) => {
                    error!(error=%error, "Failed to finalize private upload");
                    (StatusCode::SERVICE_UNAVAILABLE, "Media storage unavailable").into_response()
                }
            };
        }
    }
    (StatusCode::BAD_REQUEST, "No file field in upload").into_response()
}

/// GET /api/uploads/:id — serve an uploaded file.
pub async fn get_upload(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(attachment_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let attachment = match state
        .engine
        .authorized_media_download(&auth.actor, &attachment_id)
        .await
    {
        Ok(attachment) => attachment,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };
    let Some((start, end)) = parse_single_range(
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        attachment.file_size as u64,
    ) else {
        return (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response();
    };
    let mut file =
        match crate::media::open_rooted_media(&state.media_dir, &attachment.storage_key).await {
            Ok(file) => file,
            Err(error) => {
                error!(error=%error,attachment_id=%attachment_id,"Private media bytes missing");
                return (StatusCode::NOT_FOUND, "File not found").into_response();
            }
        };
    if !state
        .engine
        .media_download_is_still_authorized(&auth.actor, &attachment_id)
        .await
    {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    }
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "Media storage unavailable").into_response();
    }
    let safe_filename: String = attachment
        .original_filename
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != ';' && *c != '\\')
        .collect();
    let safe_filename = if safe_filename.is_empty() {
        "download".to_string()
    } else {
        safe_filename
    };
    // Only allow inline rendering for safe media types to prevent stored XSS
    // (e.g., a file with content_type: text/html containing <script> tags)
    let is_safe_inline = safe_inline_content_type(&attachment.content_type);
    let content_disposition = if is_safe_inline {
        format!("inline; filename=\"{safe_filename}\"")
    } else {
        format!("attachment; filename=\"{safe_filename}\"")
    };
    let length = end - start + 1;
    let partial = start != 0 || end + 1 != attachment.file_size as u64;
    let mut response = Body::from_stream(ReaderStream::new(file.take(length))).into_response();
    *response.status_mut() = if partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        attachment
            .content_type
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    out.insert(
        header::CONTENT_DISPOSITION,
        content_disposition
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    out.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    out.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    out.insert(
        header::ACCEPT_RANGES,
        header::HeaderValue::from_static("bytes"),
    );
    out.insert(header::CONTENT_LENGTH, length.into());
    if partial {
        out.insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", attachment.file_size)
                .parse()
                .unwrap(),
        );
    }
    response
}

fn parse_single_range(value: Option<&str>, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let Some(value) = value else {
        return Some((0, size - 1));
    };
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = start.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then_some((start, end))
}

fn safe_inline_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg"
            | "image/png"
            | "image/gif"
            | "image/webp"
            | "video/mp4"
            | "video/webm"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/webm"
    )
}

pub async fn delete_upload(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(attachment_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .delete_unattached_upload_for_actor(&auth.actor, &attachment_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(error) => organization_error_response(error),
    }
}

// ── Custom emoji endpoints ──────────────────────────────────────

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

// ── Profile endpoints ──

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

// ── Search endpoint ──

#[derive(Deserialize)]
pub struct SearchParams {
    pub server_id: String,
    pub q: String,
    pub channel: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub continuation: Option<String>,
}

/// GET /api/search — search messages
pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let q_len = params.q.len();
    if q_len == 0 || q_len > 1_024 {
        return (StatusCode::BAD_REQUEST, "Query must be 1-1024 characters").into_response();
    }

    let limit = params.limit.unwrap_or(25).min(50);
    let offset = params.offset.unwrap_or(0);

    match state
        .engine
        .search_messages(
            &auth.actor,
            crate::engine::chat_engine::SearchMessagesRequest {
                server_id: &params.server_id,
                query: &params.q,
                channel_name: params.channel.as_deref(),
                limit,
                offset,
                continuation: params.continuation.as_deref(),
            },
        )
        .await
    {
        Ok(page) => {
            if !state
                .engine
                .authorization_stamp_is_current(&auth.actor, &page.stamp)
                .await
            {
                return (StatusCode::NOT_FOUND, "Resource unavailable").into_response();
            }
            let results: Vec<serde_json::Value> = page
                .results
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "from": r.from,
                        "content": r.content,
                        "timestamp": r.timestamp,
                        "channel_id": r.channel_id,
                        "edited_at": r.edited_at,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "query": params.q,
                "results": results,
                "total_count": page.total_count,
                "offset": page.offset,
                "next_continuation": page.next_continuation,
                "restarted": page.restarted,
            }))
            .into_response()
        }
        Err(e) => {
            error!(error = %e, "Search failed");
            let status = match &e {
                crate::engine::chat_engine::SearchError::InvalidInput(_)
                | crate::engine::chat_engine::SearchError::InvalidContinuation => {
                    StatusCode::BAD_REQUEST
                }
                crate::engine::chat_engine::SearchError::ResourceUnavailable => {
                    StatusCode::NOT_FOUND
                }
                crate::engine::chat_engine::SearchError::DependencyUnavailable(_) => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            };
            (
                status,
                Json(serde_json::json!({
                    "code": e.code(),
                    "message": e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

// ── Phase 7: Community & Discovery (public endpoints) ──

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

// ── Phase 8: Webhook incoming endpoint (public, token-authed via URL) ──

#[derive(Deserialize)]
pub struct WebhookExecuteRequest {
    pub content: String,
    pub idempotency_key: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
}

/// POST /api/webhooks/{id}/{token} — execute an incoming webhook (public, no session auth).
pub async fn execute_webhook(
    State(state): State<Arc<AppState>>,
    Path((webhook_id, token)): Path<(String, String)>,
    Json(body): Json<WebhookExecuteRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .execute_incoming_webhook(
            &webhook_id,
            &token,
            &body.content,
            &body.idempotency_key,
            body.username.as_deref(),
            body.avatar_url.as_deref(),
        )
        .await
    {
        Ok(receipt) => (StatusCode::CREATED, Json(receipt)).into_response(),
        Err(e) => {
            if e.contains("Invalid webhook token") {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": e})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e})),
                )
                    .into_response()
            }
        }
    }
}

#[derive(Deserialize)]
pub struct WebhookDeliveryListParams {
    pub limit: Option<i64>,
}

pub async fn list_webhook_deliveries(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(webhook_id): Path<String>,
    Query(params): Query<WebhookDeliveryListParams>,
) -> impl IntoResponse {
    match state
        .engine
        .list_webhook_deliveries(&auth.actor, &webhook_id, params.limit.unwrap_or(50))
        .await
    {
        Ok(deliveries) => Json(serde_json::json!({"deliveries": deliveries})).into_response(),
        Err(error) => integration_http_error(&error, "Webhook unavailable"),
    }
}

pub async fn test_outgoing_webhook(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(webhook_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .enqueue_webhook_test(&auth.actor, &webhook_id)
        .await
    {
        Ok(delivery_id) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"delivery_id":delivery_id,"status":"pending"})),
        )
            .into_response(),
        Err(error) => integration_http_error(&error, "Webhook unavailable"),
    }
}

pub async fn retry_webhook_delivery(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(delivery_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .retry_webhook_delivery(&auth.actor, &delivery_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => integration_http_error(&error, "Delivery unavailable"),
    }
}

fn integration_http_error(error: &str, hidden_message: &'static str) -> axum::response::Response {
    if error.starts_with("DEPENDENCY_UNAVAILABLE:") {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Integration dependency unavailable",
        )
            .into_response()
    } else if error.starts_with("AUTHENTICATION_REQUIRED:") {
        (StatusCode::UNAUTHORIZED, "Authentication required").into_response()
    } else if error.starts_with("FORBIDDEN:") {
        (StatusCode::FORBIDDEN, hidden_message).into_response()
    } else if error.starts_with("INVALID_INPUT:") {
        (StatusCode::BAD_REQUEST, "Invalid integration request").into_response()
    } else {
        (StatusCode::NOT_FOUND, hidden_message).into_response()
    }
}

// ── Bluesky Profile Sync ──────────────────────────────────────

/// POST /api/bluesky/sync-profile — fetch and store the user's Bluesky profile.
pub async fn sync_bluesky_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    let did = match state.engine.verified_atproto_profile_did(&auth.actor).await {
        Ok(did) => did,
        Err(error) => return profile_sync_error_response(error),
    };

    let profile = match super::atproto::fetch_full_bsky_profile(
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

fn profile_sync_error_response(
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

// ── AT Protocol Record Sync Settings ──────────────────────────────────────

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

// ── Sticker endpoints ──

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

// ── Cross-server emoji endpoint ──

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

// ── Emoji settings endpoint ──

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

// ── Server limits endpoint ──

pub async fn get_server_limits(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "max_message_length": state.max_message_length,
        "max_file_size_mb": state.max_file_size / (1024 * 1024),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HistoryParams deserialization ──

    #[test]
    fn test_history_params_full() {
        let json = r#"{"server_id": "srv-1", "before": "msg-abc", "limit": 100}"#;
        let params: HistoryParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.server_id, Some("srv-1".into()));
        assert_eq!(params.before, Some("msg-abc".into()));
        assert_eq!(params.limit, Some(100));
    }

    #[test]
    fn test_history_params_minimal() {
        let json = r#"{}"#;
        let params: HistoryParams = serde_json::from_str(json).unwrap();
        assert!(params.server_id.is_none());
        assert!(params.before.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn test_history_params_only_server_id() {
        let json = r#"{"server_id": "default"}"#;
        let params: HistoryParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.server_id, Some("default".into()));
        assert!(params.before.is_none());
        assert!(params.limit.is_none());
    }

    // ── ChannelListParams deserialization ──

    #[test]
    fn test_channel_list_params() {
        let json = r#"{"server_id": "srv-1"}"#;
        let params: ChannelListParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.server_id, Some("srv-1".into()));
    }

    #[test]
    fn test_channel_list_params_empty() {
        let json = r#"{}"#;
        let params: ChannelListParams = serde_json::from_str(json).unwrap();
        assert!(params.server_id.is_none());
    }

    // ── CreateServerRequest deserialization ──

    #[test]
    fn test_create_server_request_full() {
        let json = r#"{"name": "My Server", "icon_url": "https://example.com/icon.png"}"#;
        let req: CreateServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "My Server");
        assert_eq!(req.icon_url, Some("https://example.com/icon.png".into()));
    }

    #[test]
    fn test_create_server_request_name_only() {
        let json = r#"{"name": "Test"}"#;
        let req: CreateServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "Test");
        assert!(req.icon_url.is_none());
    }

    #[test]
    fn test_create_server_request_missing_name_fails() {
        let json = r#"{"icon_url": "https://example.com/icon.png"}"#;
        assert!(serde_json::from_str::<CreateServerRequest>(json).is_err());
    }

    // ── SetAdminRequest deserialization ──

    #[test]
    fn test_set_admin_request_true() {
        let json = r#"{"is_admin": true}"#;
        let req: SetAdminRequest = serde_json::from_str(json).unwrap();
        assert!(req.is_admin);
    }

    #[test]
    fn test_set_admin_request_false() {
        let json = r#"{"is_admin": false}"#;
        let req: SetAdminRequest = serde_json::from_str(json).unwrap();
        assert!(!req.is_admin);
    }

    #[test]
    fn test_set_admin_request_missing_field_fails() {
        let json = r#"{}"#;
        assert!(serde_json::from_str::<SetAdminRequest>(json).is_err());
    }

    // ── AuthStatusResponse serialization ──

    #[test]
    fn test_auth_status_response_serialize() {
        let resp = AuthStatusResponse {
            authenticated: false,
            providers: vec!["atproto".into()],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["authenticated"], false);
        let providers = json["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0], "atproto");
    }

    // ── UserProfile serialization ──

    #[test]
    fn test_user_profile_serialize_full() {
        let profile = UserProfile {
            id: "user-1".into(),
            username: "alice".into(),
            email: Some("alice@example.com".into()),
            avatar_url: Some("https://example.com/avatar.jpg".into()),
        };
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["id"], "user-1");
        assert_eq!(json["username"], "alice");
        assert_eq!(json["email"], "alice@example.com");
        assert_eq!(json["avatar_url"], "https://example.com/avatar.jpg");
    }

    #[test]
    fn test_user_profile_serialize_minimal() {
        let profile = UserProfile {
            id: "u1".into(),
            username: "bob".into(),
            email: None,
            avatar_url: None,
        };
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["id"], "u1");
        assert_eq!(json["username"], "bob");
        assert!(json["email"].is_null());
        assert!(json["avatar_url"].is_null());
    }

    // ── PublicUserProfile serialization ──

    #[test]
    fn test_public_user_profile_serialize() {
        let profile = PublicUserProfile {
            username: "alice".into(),
            avatar_url: Some("https://example.com/pic.jpg".into()),
            provider: Some("github".into()),
            provider_id: Some("12345".into()),
        };
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["username"], "alice");
        assert_eq!(json["provider"], "github");
        assert_eq!(json["provider_id"], "12345");
    }

    #[test]
    fn test_public_user_profile_serialize_no_optionals() {
        let profile = PublicUserProfile {
            username: "bob".into(),
            avatar_url: None,
            provider: None,
            provider_id: None,
        };
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["username"], "bob");
        assert!(json["avatar_url"].is_null());
        assert!(json["provider"].is_null());
    }

    // ── CreateTokenRequest deserialization ──

    #[test]
    fn test_create_token_request_with_label() {
        let json = r#"{"label": "My IRC client"}"#;
        let req: CreateTokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.label, Some("My IRC client".into()));
    }

    #[test]
    fn test_create_token_request_no_label() {
        let json = r#"{}"#;
        let req: CreateTokenRequest = serde_json::from_str(json).unwrap();
        assert!(req.label.is_none());
    }

    // ── CreateTokenResponse serialization ──

    #[test]
    fn test_create_token_response_serialize() {
        let resp = CreateTokenResponse {
            id: "tok-1".into(),
            token: "abcdef123456".into(),
            label: Some("dev".into()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "tok-1");
        assert_eq!(json["token"], "abcdef123456");
        assert_eq!(json["label"], "dev");
    }

    // ── IrcTokenInfo serialization ──

    #[test]
    fn test_irc_token_info_serialize() {
        let info = IrcTokenInfo {
            id: "t1".into(),
            label: Some("test".into()),
            last_used: Some("2025-01-01T00:00:00Z".into()),
            created_at: "2025-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["id"], "t1");
        assert_eq!(json["label"], "test");
        assert_eq!(json["last_used"], "2025-01-01T00:00:00Z");
        assert_eq!(json["created_at"], "2025-01-01T00:00:00Z");
    }

    #[test]
    fn test_irc_token_info_serialize_no_optionals() {
        let info = IrcTokenInfo {
            id: "t2".into(),
            label: None,
            last_used: None,
            created_at: "2025-01-01".into(),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert!(json["label"].is_null());
        assert!(json["last_used"].is_null());
    }

    // ── UploadResponse serialization ──

    #[test]
    fn test_upload_response_serialize() {
        let resp = UploadResponse {
            id: "att-1".into(),
            filename: "photo.jpg".into(),
            content_type: "image/jpeg".into(),
            file_size: 1024,
            url: "/api/uploads/att-1".into(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "att-1");
        assert_eq!(json["filename"], "photo.jpg");
        assert_eq!(json["content_type"], "image/jpeg");
        assert_eq!(json["file_size"], 1024);
        assert_eq!(json["url"], "/api/uploads/att-1");
    }

    // ── EmojiResponse serialization ──

    #[test]
    fn test_emoji_response_serialize() {
        let resp = EmojiResponse {
            id: "e1".into(),
            server_id: "s1".into(),
            name: "thumbsup".into(),
            image_url: "/api/uploads/emoji.png".into(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["name"], "thumbsup");
        assert_eq!(json["server_id"], "s1");
    }

    // ── CreateEmojiRequest deserialization ──

    #[test]
    fn test_create_emoji_request() {
        let json = r#"{"name": "smile", "image_url": "https://example.com/smile.png"}"#;
        let req: CreateEmojiRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "smile");
        assert_eq!(req.image_url, "https://example.com/smile.png");
    }

    #[test]
    fn test_create_emoji_request_missing_name_fails() {
        let json = r#"{"image_url": "url"}"#;
        assert!(serde_json::from_str::<CreateEmojiRequest>(json).is_err());
    }

    #[test]
    fn test_create_emoji_request_missing_url_fails() {
        let json = r#"{"name": "smile"}"#;
        assert!(serde_json::from_str::<CreateEmojiRequest>(json).is_err());
    }

    // ── UpdateProfileRequest deserialization ──

    #[test]
    fn test_update_profile_request_full() {
        let json = r#"{"bio": "Hello!", "pronouns": "they/them", "banner_url": "https://example.com/banner.jpg"}"#;
        let req: UpdateProfileRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.bio, Some("Hello!".into()));
        assert_eq!(req.pronouns, Some("they/them".into()));
        assert_eq!(
            req.banner_url,
            Some("https://example.com/banner.jpg".into())
        );
    }

    #[test]
    fn test_update_profile_request_empty() {
        let json = r#"{}"#;
        let req: UpdateProfileRequest = serde_json::from_str(json).unwrap();
        assert!(req.bio.is_none());
        assert!(req.pronouns.is_none());
        assert!(req.banner_url.is_none());
    }

    // ── SearchParams deserialization ──

    #[test]
    fn test_search_params_full() {
        let json = r##"{"server_id": "s1", "q": "hello", "channel": "#general", "limit": 10, "offset": 5}"##;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.server_id, "s1");
        assert_eq!(params.q, "hello");
        assert_eq!(params.channel, Some("#general".into()));
        assert_eq!(params.limit, Some(10));
        assert_eq!(params.offset, Some(5));
    }

    #[test]
    fn test_search_params_minimal() {
        let json = r#"{"server_id": "s1", "q": "test"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.server_id, "s1");
        assert_eq!(params.q, "test");
        assert!(params.channel.is_none());
        assert!(params.limit.is_none());
        assert!(params.offset.is_none());
    }

    #[test]
    fn test_search_params_missing_required_fails() {
        let json = r#"{"q": "test"}"#;
        assert!(serde_json::from_str::<SearchParams>(json).is_err());
    }

    // ── DiscoverParams deserialization ──

    #[test]
    fn test_discover_params_with_category() {
        let json = r#"{"category": "gaming"}"#;
        let params: DiscoverParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.category, Some("gaming".into()));
    }

    #[test]
    fn test_discover_params_empty() {
        let json = r#"{}"#;
        let params: DiscoverParams = serde_json::from_str(json).unwrap();
        assert!(params.category.is_none());
    }

    // ── WebhookExecuteRequest deserialization ──

    #[test]
    fn test_webhook_execute_request_full() {
        let json = r#"{"content": "Hello from webhook", "idempotency_key": "request-1", "username": "Bot", "avatar_url": "https://example.com/bot.png"}"#;
        let req: WebhookExecuteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "Hello from webhook");
        assert_eq!(req.idempotency_key, "request-1");
        assert_eq!(req.username, Some("Bot".into()));
        assert_eq!(req.avatar_url, Some("https://example.com/bot.png".into()));
    }

    #[test]
    fn test_webhook_execute_request_content_only() {
        let json = r#"{"content": "test message", "idempotency_key": "request-2"}"#;
        let req: WebhookExecuteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "test message");
        assert!(req.username.is_none());
        assert!(req.avatar_url.is_none());
    }

    #[test]
    fn test_webhook_execute_request_missing_content_fails() {
        let json = r#"{"idempotency_key": "request-3", "username": "Bot"}"#;
        assert!(serde_json::from_str::<WebhookExecuteRequest>(json).is_err());
    }

    #[test]
    fn test_webhook_execute_request_missing_idempotency_key_fails() {
        let json = r#"{"content": "test message"}"#;
        assert!(serde_json::from_str::<WebhookExecuteRequest>(json).is_err());
    }

    // ── HistoryResponse serialization ──

    #[test]
    fn test_history_response_serialize() {
        let resp = HistoryResponse {
            channel: "#general".into(),
            messages: vec![],
            has_more: false,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["channel"], "#general");
        assert_eq!(json["messages"].as_array().unwrap().len(), 0);
        assert_eq!(json["has_more"], false);
    }

    #[test]
    fn test_history_response_serialize_has_more() {
        let resp = HistoryResponse {
            channel: "#dev".into(),
            messages: vec![],
            has_more: true,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["has_more"], true);
    }

    // ── Content-Type upload validation ──

    #[test]
    fn test_allowed_upload_content_types() {
        assert!(is_allowed_upload_content_type("image/jpeg"));
        assert!(is_allowed_upload_content_type("image/png"));
        assert!(is_allowed_upload_content_type("image/gif"));
        assert!(is_allowed_upload_content_type("image/webp"));
        assert!(is_allowed_upload_content_type("video/mp4"));
        assert!(is_allowed_upload_content_type("audio/mpeg"));
        assert!(is_allowed_upload_content_type("application/pdf"));
        assert!(is_allowed_upload_content_type("application/octet-stream"));
        assert!(is_allowed_upload_content_type("text/plain"));
        assert!(is_allowed_upload_content_type("text/css"));
    }

    #[test]
    fn test_blocked_upload_content_types() {
        assert!(!is_allowed_upload_content_type("text/html"));
        assert!(!is_allowed_upload_content_type("text/javascript"));
        assert!(!is_allowed_upload_content_type("application/javascript"));
        assert!(!is_allowed_upload_content_type("application/xhtml+xml"));
        assert!(!is_allowed_upload_content_type("image/svg+xml"));
        assert!(!is_allowed_upload_content_type("text/xml"));
        assert!(!is_allowed_upload_content_type("application/xml"));
    }

    #[test]
    fn private_media_range_parser_rejects_ambiguous_and_out_of_bounds_ranges() {
        assert_eq!(parse_single_range(None, 10), Some((0, 9)));
        assert_eq!(parse_single_range(Some("bytes=2-5"), 10), Some((2, 5)));
        assert_eq!(parse_single_range(Some("bytes=-3"), 10), Some((7, 9)));
        assert_eq!(parse_single_range(Some("bytes=7-"), 10), Some((7, 9)));
        assert_eq!(parse_single_range(Some("bytes=10-11"), 10), None);
        assert_eq!(parse_single_range(Some("bytes=1-2,4-5"), 10), None);
    }

    #[test]
    fn active_media_types_are_always_downloads() {
        assert!(safe_inline_content_type("image/png"));
        assert!(!safe_inline_content_type("image/svg+xml"));
        assert!(!safe_inline_content_type("text/html"));
        assert!(!safe_inline_content_type("application/pdf"));
    }

    #[test]
    fn test_blocked_content_type_with_params() {
        // Should still block even with charset parameters
        assert!(!is_allowed_upload_content_type("text/html; charset=utf-8"));
        assert!(!is_allowed_upload_content_type(
            "application/javascript; charset=utf-8"
        ));
    }

    #[test]
    fn test_blocked_content_type_case_insensitive() {
        assert!(!is_allowed_upload_content_type("Text/HTML"));
        assert!(!is_allowed_upload_content_type("APPLICATION/JAVASCRIPT"));
        assert!(!is_allowed_upload_content_type("Image/SVG+XML"));
    }

    #[test]
    fn test_malformed_content_type_rejected() {
        assert!(!is_allowed_upload_content_type("notamimetype"));
        assert!(!is_allowed_upload_content_type(""));
        // Excessively long content type
        let long_type = format!("image/{}", "x".repeat(300));
        assert!(!is_allowed_upload_content_type(&long_type));
    }
}
