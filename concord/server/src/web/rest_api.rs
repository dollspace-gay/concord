use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{FromRef, FromRequestParts, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;

use crate::auth::token::{generate_irc_token, hash_irc_token, verify_irc_token};
use crate::db::queries::{
    atproto as atproto_queries, attachments, bots, community, emoji, invites, messages, profiles,
    roles, servers, stickers, users,
};
use crate::engine::events::HistoryMessage;
use crate::engine::permissions::{Permissions, compute_effective_permissions};
use sqlx;

use super::app_state::AppState;
use super::auth_middleware::AuthUser;

/// Check if a user has a specific permission in a server.
/// Returns Ok(()) if permitted, or an error response.
async fn check_server_permission(
    pool: &sqlx::SqlitePool,
    server_id: &str,
    user_id: &str,
    required: Permissions,
) -> Result<(), (StatusCode, &'static str)> {
    let server = servers::get_server(pool, server_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::NOT_FOUND, "Server not found"))?;

    if server.owner_id == user_id {
        return Ok(()); // Owner has all permissions
    }

    let default_role = roles::get_default_role(pool, server_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
    let base = default_role
        .map(|r| Permissions::from_bits_truncate(r.permissions as u64))
        .unwrap_or(Permissions::empty());

    let user_roles = roles::get_user_roles(pool, server_id, user_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
    let role_perms: Vec<(String, Permissions)> = user_roles
        .iter()
        .map(|r| {
            (
                r.id.clone(),
                Permissions::from_bits_truncate(r.permissions as u64),
            )
        })
        .collect();

    let effective = compute_effective_permissions(base, &role_perms, &[], "", user_id, false);

    if effective.contains(required) {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "Insufficient permissions"))
    }
}

// ── Phase 8: Bot token auth extractor ──────────────────────

/// Extractor that validates a `Authorization: Bot <token>` header.
/// Used for bot API endpoints that authenticate via bot tokens.
pub struct BotAuth {
    pub user_id: String,
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

        // Bot tokens have format "bot_<user_id>.<random>" — extract user_id
        // to scope the hash search to only that user's tokens.
        let user_id_hint = token
            .strip_prefix("bot_")
            .and_then(|rest| rest.split('.').next())
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid bot token format"))?;

        let user_tokens = bots::list_bot_tokens(&app_state.db, user_id_hint)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        if user_tokens.is_empty() {
            // Constant-time: always perform one argon2 verify even when no tokens exist
            // to prevent timing side-channel that could enumerate valid bot user_ids.
            let _ = verify_irc_token(
                "dummy",
                "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$invaliddummyhashvalue",
            );
            return Err((StatusCode::UNAUTHORIZED, "Invalid bot token"));
        }

        let row = user_tokens
            .into_iter()
            .find(|t| verify_irc_token(token, &t.token_hash))
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid bot token"))?;

        // Update last_used timestamp in background
        let pool = app_state.db.clone();
        let tid = row.id.clone();
        tokio::spawn(async move {
            let _ = bots::update_token_last_used(&pool, &tid).await;
        });

        Ok(BotAuth {
            user_id: row.user_id,
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
            Some(&_auth.user_id),
        )
        .await
    {
        Ok((messages, has_more)) => Json(HistoryResponse {
            channel,
            messages,
            has_more,
        })
        .into_response(),
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
    Json(state.engine.list_channels(&server_id)).into_response()
}

// ── Server endpoints (authenticated) ────────────────────

/// GET /api/servers — list the current user's servers.
pub async fn list_servers(State(state): State<Arc<AppState>>, auth: AuthUser) -> impl IntoResponse {
    Json(state.engine.list_servers_for_user(&auth.user_id).await)
}

#[derive(Deserialize)]
pub struct CreateServerRequest {
    pub name: String,
    pub icon_url: Option<String>,
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
        .create_server(body.name, auth.user_id, body.icon_url)
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
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// GET /api/servers/:id — get server info.
pub async fn get_server(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    let is_member = servers::is_server_member(&state.db, &server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

    match state
        .engine
        .list_all_servers()
        .into_iter()
        .find(|s| s.id == server_id)
    {
        Some(server) => Json(server).into_response(),
        None => (StatusCode::NOT_FOUND, "Server not found").into_response(),
    }
}

/// DELETE /api/servers/:id — delete a server (owner only).
pub async fn delete_server(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    auth: AuthUser,
) -> impl IntoResponse {
    if !state.engine.is_server_owner(&server_id, &auth.user_id) {
        return (StatusCode::FORBIDDEN, "Only the server owner can delete it").into_response();
    }
    match state.engine.delete_server(&server_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// GET /api/servers/:id/channels — list channels in a server.
pub async fn list_server_channels(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    let is_member = servers::is_server_member(&state.db, &server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

    Json(state.engine.list_channels(&server_id)).into_response()
}

/// GET /api/servers/:id/channels/:name/messages — channel history within a server.
pub async fn get_server_channel_history(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path((server_id, channel_name)): Path<(String, String)>,
    Query(params): Query<HistoryParams>,
) -> impl IntoResponse {
    let is_member = servers::is_server_member(&state.db, &server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

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
            Some(&auth.user_id),
        )
        .await
    {
        Ok((messages, has_more)) => Json(HistoryResponse {
            channel,
            messages,
            has_more,
        })
        .into_response(),
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
    let is_member = servers::is_server_member(&state.db, &server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

    match servers::get_server_members(&state.db, &server_id).await {
        Ok(rows) => {
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
        Err(e) => {
            error!(error = %e, "Failed to list server members");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── Admin endpoints (system admin only) ─────────────────

/// GET /api/admin/servers — list all servers (system admin).
pub async fn admin_list_servers(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match servers::is_system_admin(&state.db, &auth.user_id).await {
        Ok(true) => Json(state.engine.list_all_servers()).into_response(),
        Ok(false) => (StatusCode::FORBIDDEN, "Not a system admin").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to check admin status");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

/// DELETE /api/admin/servers/:id — delete any server (system admin).
pub async fn admin_delete_server(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    auth: AuthUser,
) -> impl IntoResponse {
    match servers::is_system_admin(&state.db, &auth.user_id).await {
        Ok(true) => match state.engine.delete_server(&server_id).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        },
        Ok(false) => (StatusCode::FORBIDDEN, "Not a system admin").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to check admin status");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
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
    match servers::is_system_admin(&state.db, &auth.user_id).await {
        Ok(true) => {
            match servers::set_system_admin(&state.db, &target_user_id, body.is_admin).await {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => {
                    error!(error = %e, "Failed to set admin flag");
                    (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
                }
            }
        }
        Ok(false) => (StatusCode::FORBIDDEN, "Not a system admin").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to check admin status");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
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
    match users::get_user(&state.db, &auth.user_id).await {
        Ok(Some((id, username, email, avatar_url))) => Json(UserProfile {
            id,
            username,
            email,
            avatar_url,
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
    match users::get_user_by_nickname(&state.db, &nickname).await {
        Ok(Some((_id, username, _email, avatar_url, provider, provider_id))) => {
            Json(PublicUserProfile {
                username,
                avatar_url,
                provider,
                provider_id,
            })
            .into_response()
        }
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
    match users::list_irc_tokens(&state.db, &auth.user_id).await {
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

    let token = generate_irc_token();
    let hash = match hash_irc_token(&token) {
        Ok(h) => h,
        Err(e) => {
            error!(error = %e, "Failed to hash IRC token");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Token creation failed").into_response();
        }
    };

    let token_id = Uuid::new_v4().to_string();

    if let Err(e) = users::create_irc_token(
        &state.db,
        &token_id,
        &auth.user_id,
        &hash,
        body.label.as_deref(),
    )
    .await
    {
        error!(error = %e, "Failed to store IRC token");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Token creation failed").into_response();
    }

    Json(CreateTokenResponse {
        id: token_id,
        token, // shown only once
        label: body.label,
    })
    .into_response()
}

/// GET /api/tokens — list the current user's IRC tokens (no secrets).
pub async fn list_irc_tokens(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match users::list_irc_tokens(&state.db, &auth.user_id).await {
        Ok(rows) => {
            let tokens: Vec<IrcTokenInfo> = rows
                .into_iter()
                .map(|(id, label, last_used, created_at)| IrcTokenInfo {
                    id,
                    label,
                    last_used,
                    created_at,
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
    match users::delete_irc_token(&state.db, &token_id, &auth.user_id).await {
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
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut file_data: Option<(String, String, Vec<u8>)> = None; // (filename, content_type, bytes)

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or("unnamed").to_string();
            let content_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();

            match field.bytes().await {
                Ok(bytes) => {
                    if bytes.len() as u64 > state.max_file_size {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            format!(
                                "File too large. Max size is {} MB",
                                state.max_file_size / (1024 * 1024)
                            ),
                        )
                            .into_response();
                    }
                    file_data = Some((filename, content_type, bytes.to_vec()));
                }
                Err(e) => {
                    error!(error = %e, "Failed to read upload data");
                    return (StatusCode::BAD_REQUEST, "Failed to read file data").into_response();
                }
            }
            break;
        }
    }

    let Some((original_filename, content_type, bytes)) = file_data else {
        return (StatusCode::BAD_REQUEST, "No file field in upload").into_response();
    };

    // Validate Content-Type: must look like a MIME type and not be on the blocklist.
    if !is_allowed_upload_content_type(&content_type) {
        return (StatusCode::BAD_REQUEST, "File type not allowed").into_response();
    }

    let file_size = bytes.len() as i64;
    let attachment_id = Uuid::new_v4().to_string();

    // Sanitize filename: keep only the last path component, replace unsafe chars
    let safe_filename = original_filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file");

    // Try uploading to the user's PDS (AT Protocol blob storage) first
    let public_url = &state.auth_config.public_url;
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);
    let redirect_uri = format!("{}/api/auth/atproto/callback", public_url);
    match super::pds_client::upload_blob_to_pds(
        &state.db,
        &auth.user_id,
        bytes.clone(),
        &content_type,
        &state.atproto.signing_key,
        &client_id,
        &redirect_uri,
    )
    .await
    {
        Ok(blob_ref) => {
            info!(
                user_id = %auth.user_id,
                cid = %blob_ref.cid,
                blob_url = %blob_ref.url,
                "Uploaded blob to PDS"
            );
            if let Err(e) = attachments::insert_attachment_with_blob(
                &state.db,
                &attachments::InsertBlobAttachmentParams {
                    id: &attachment_id,
                    uploader_id: &auth.user_id,
                    filename: &attachment_id,
                    original_filename: safe_filename,
                    content_type: &content_type,
                    file_size,
                    blob_cid: &blob_ref.cid,
                    blob_url: &blob_ref.url,
                },
            )
            .await
            {
                error!(error = %e, "Failed to insert blob attachment record");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }

            let url = format!("/api/uploads/{}", attachment_id);
            (
                StatusCode::CREATED,
                Json(UploadResponse {
                    id: attachment_id,
                    filename: safe_filename.to_string(),
                    content_type,
                    file_size,
                    url,
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(error = %e, "PDS blob upload failed — AT Protocol credentials may be missing or expired");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "File upload requires AT Protocol (Bluesky) authentication with valid PDS credentials",
            )
                .into_response()
        }
    }
}

/// GET /api/uploads/:id — serve an uploaded file.
pub async fn get_upload(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(attachment_id): Path<String>,
) -> impl IntoResponse {
    // Look up attachment metadata
    let attachment = match attachments::get_attachment(&state.db, &attachment_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to look up attachment");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    // Proxy blob from user's PDS through our server.
    // This avoids CORS issues (PDS doesn't send CORS headers) and ensures
    // the correct Content-Type is served for audio/video playback.
    let Some(blob_url) = &attachment.blob_url else {
        return (StatusCode::NOT_FOUND, "Attachment has no PDS blob URL").into_response();
    };

    // SSRF protection: verify blob URL doesn't point to private/internal IPs
    if !crate::engine::embeds::is_safe_url(blob_url).await {
        return (
            StatusCode::FORBIDDEN,
            "Blob URL points to a restricted address",
        )
            .into_response();
    }

    info!(attachment_id = %attachment_id, blob_url = %blob_url, "Proxying PDS blob");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_default();
    match client.get(blob_url.as_str()).send().await {
        Ok(resp) if resp.status().is_success() => {
            // Sanitize filename: strip control chars, newlines, semicolons, quotes
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
            let is_safe_inline = attachment.content_type.starts_with("image/")
                || attachment.content_type.starts_with("video/")
                || attachment.content_type.starts_with("audio/")
                || attachment.content_type == "application/pdf";
            let content_disposition = if is_safe_inline {
                format!("inline; filename=\"{safe_filename}\"")
            } else {
                format!("attachment; filename=\"{safe_filename}\"")
            };
            let body = Body::from_stream(resp.bytes_stream());
            (
                [
                    (header::CONTENT_TYPE, attachment.content_type),
                    (header::CONTENT_DISPOSITION, content_disposition),
                    (
                        header::CACHE_CONTROL,
                        "public, max-age=31536000, immutable".to_string(),
                    ),
                ],
                body,
            )
                .into_response()
        }
        Ok(resp) => {
            error!(
                attachment_id = %attachment_id,
                status = %resp.status(),
                "PDS blob fetch returned error"
            );
            (StatusCode::BAD_GATEWAY, "Failed to fetch blob from storage").into_response()
        }
        Err(e) => {
            error!(error = %e, "Failed to proxy PDS blob");
            (StatusCode::BAD_GATEWAY, "Failed to fetch blob from storage").into_response()
        }
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
    let is_member = servers::is_server_member(&state.db, &server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

    match emoji::list_emoji(&state.db, &server_id).await {
        Ok(rows) => {
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
    if let Err(resp) = check_server_permission(
        &state.db,
        &server_id,
        &user.user_id,
        Permissions::MANAGE_SERVER,
    )
    .await
    {
        return resp.into_response();
    }

    // Validate emoji name: alphanumeric + underscores, 2-32 chars
    let name = body.name.trim().to_lowercase();
    if name.len() < 2 || name.len() > 32 || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return (
            StatusCode::BAD_REQUEST,
            "Emoji name must be 2-32 alphanumeric/underscore characters",
        )
            .into_response();
    }

    let id = Uuid::new_v4().to_string();
    match emoji::insert_emoji(
        &state.db,
        &id,
        &server_id,
        &name,
        &body.image_url,
        &user.user_id,
    )
    .await
    {
        Ok(()) => Json(EmojiResponse {
            id,
            server_id,
            name,
            image_url: body.image_url,
        })
        .into_response(),
        Err(e) => {
            if e.to_string().contains("UNIQUE") {
                (
                    StatusCode::CONFLICT,
                    "An emoji with that name already exists",
                )
                    .into_response()
            } else {
                error!(error = %e, "Failed to create emoji");
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
            }
        }
    }
}

pub async fn delete_server_emoji(
    State(state): State<Arc<AppState>>,
    Path((server_id, emoji_id)): Path<(String, String)>,
    user: AuthUser,
) -> impl IntoResponse {
    if let Err(resp) = check_server_permission(
        &state.db,
        &server_id,
        &user.user_id,
        Permissions::MANAGE_SERVER,
    )
    .await
    {
        return resp.into_response();
    }

    match emoji::delete_emoji(&state.db, &emoji_id, &server_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Emoji not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to delete emoji");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── Profile endpoints ──

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub banner_url: Option<String>,
}

/// GET /api/users/:id/profile — get a user's full profile
pub async fn get_user_full_profile(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
) -> impl IntoResponse {
    let user = match users::get_user(&state.db, &user_id).await {
        Ok(Some((id, username, _email, avatar_url))) => (id, username, avatar_url),
        Ok(None) => return (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to get user");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let profile = profiles::get_profile(&state.db, &user.0)
        .await
        .unwrap_or(None);

    let created_at = sqlx::query_scalar::<_, String>("SELECT created_at FROM users WHERE id = ?")
        .bind(&user.0)
        .fetch_one(&state.db)
        .await
        .unwrap_or_else(|_| "unknown".into());

    Json(serde_json::json!({
        "user_id": user.0,
        "username": user.1,
        "avatar_url": user.2,
        "bio": profile.as_ref().and_then(|p| p.bio.as_ref()),
        "pronouns": profile.as_ref().and_then(|p| p.pronouns.as_ref()),
        "banner_url": profile.as_ref().and_then(|p| p.banner_url.as_ref()),
        "created_at": created_at,
    }))
    .into_response()
}

/// PATCH /api/profile — update own profile
pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> impl IntoResponse {
    // Validate field lengths
    if let Some(ref bio) = body.bio
        && bio.len() > 2000
    {
        return (
            StatusCode::BAD_REQUEST,
            "Bio must be 2000 characters or less",
        )
            .into_response();
    }
    if let Some(ref pronouns) = body.pronouns
        && pronouns.len() > 100
    {
        return (
            StatusCode::BAD_REQUEST,
            "Pronouns must be 100 characters or less",
        )
            .into_response();
    }
    if let Some(ref banner_url) = body.banner_url
        && banner_url.len() > 2000
    {
        return (
            StatusCode::BAD_REQUEST,
            "Banner URL must be 2000 characters or less",
        )
            .into_response();
    }

    match profiles::upsert_profile(
        &state.db,
        &auth.user_id,
        body.bio.as_deref(),
        body.pronouns.as_deref(),
        body.banner_url.as_deref(),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            error!(error = %e, "Failed to update profile");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
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
}

/// GET /api/search — search messages
pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let is_member = servers::is_server_member(&state.db, &params.server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

    let q_len = params.q.len();
    if q_len == 0 || q_len > 200 {
        return (StatusCode::BAD_REQUEST, "Query must be 1-200 characters").into_response();
    }

    let limit = params.limit.unwrap_or(25).min(50);
    let offset = params.offset.unwrap_or(0);

    // Resolve channel name to ID if needed
    let channel_id = if let Some(ref ch_name) = params.channel {
        match crate::db::queries::channels::get_channel_by_name(
            &state.db,
            &params.server_id,
            ch_name,
        )
        .await
        {
            Ok(Some(row)) => Some(row.id),
            _ => None,
        }
    } else {
        None
    };

    match crate::db::queries::search::search_messages(
        &state.db,
        &params.server_id,
        &params.q,
        channel_id.as_deref(),
        limit,
        offset,
    )
    .await
    {
        Ok((rows, total)) => {
            let results: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "from": r.sender_nick,
                        "content": r.content,
                        "timestamp": r.created_at,
                        "channel_id": r.channel_id,
                        "edited_at": r.edited_at,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "query": params.q,
                "results": results,
                "total_count": total,
                "offset": offset,
            }))
            .into_response()
        }
        Err(e) => {
            error!(error = %e, "Search failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Search error").into_response()
        }
    }
}

// ── Phase 7: Community & Discovery (public endpoints) ──

/// GET /api/invite/{code} — public invite preview
pub async fn get_invite_preview(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    let pool = &state.db;
    match invites::get_invite_by_code(pool, &code).await {
        Ok(Some(invite)) => {
            // Check not expired
            if let Some(ref exp) = invite.expires_at
                && exp < &chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
            {
                return (
                    StatusCode::GONE,
                    Json(serde_json::json!({"error": "Invite expired"})),
                )
                    .into_response();
            }
            // Check max uses
            if let Some(max) = invite.max_uses
                && invite.use_count >= max
            {
                return (
                    StatusCode::GONE,
                    Json(serde_json::json!({"error": "Invite has reached max uses"})),
                )
                    .into_response();
            }
            // Get server info
            match servers::get_server(pool, &invite.server_id).await {
                Ok(Some(server)) => Json(serde_json::json!({
                    "code": invite.code,
                    "server_id": server.id,
                    "server_name": server.name,
                    "server_icon_url": server.icon_url,
                }))
                .into_response(),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
        _ => {
            // Fallback: try vanity code
            match servers::get_server_by_vanity(pool, &code).await {
                Ok(Some(server)) => Json(serde_json::json!({
                    "code": code,
                    "server_id": server.id,
                    "server_name": server.name,
                    "server_icon_url": server.icon_url,
                    "is_vanity": true,
                }))
                .into_response(),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
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
    let pool = &state.db;
    let limit = params.limit.unwrap_or(50).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);
    match community::list_discoverable_servers(pool, params.category.as_deref(), limit, offset)
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
            body.username.as_deref(),
            body.avatar_url.as_deref(),
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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

// ── Bluesky Profile Sync ──────────────────────────────────────

/// POST /api/bluesky/sync-profile — fetch and store the user's Bluesky profile.
pub async fn sync_bluesky_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    // user_id IS the DID (since migration 014)
    let did = auth.user_id.clone();

    // Fetch public profile from Bluesky
    let http_client = reqwest::Client::new();
    let profile = match super::atproto::fetch_full_bsky_profile(&http_client, &did).await {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Could not fetch Bluesky profile"})),
            )
                .into_response();
        }
    };

    // Store sync data
    if let Err(e) =
        atproto_queries::store_bsky_profile_sync(&atproto_queries::StoreBskyProfileParams {
            pool: &state.db,
            user_id: &auth.user_id,
            handle: &profile.handle,
            display_name: profile.display_name.as_deref(),
            description: profile.description.as_deref(),
            banner_url: profile.banner.as_deref(),
            followers_count: profile.followers_count,
            follows_count: profile.follows_count,
        })
        .await
    {
        error!(error = %e, "Failed to store profile sync");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to save profile data"})),
        )
            .into_response();
    }

    // Also update the user's avatar and bio from Bluesky data
    if let Some(ref avatar) = profile.avatar {
        let _ = sqlx::query("UPDATE users SET avatar_url = ? WHERE id = ?")
            .bind(avatar)
            .bind(&auth.user_id)
            .execute(&state.db)
            .await;
    }
    if let Some(ref description) = profile.description {
        let _ = profiles::upsert_profile(
            &state.db,
            &auth.user_id,
            Some(description),
            None,
            profile.banner.as_deref(),
        )
        .await;
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

/// GET /api/users/{id}/bluesky — get Bluesky identity info for a user.
pub async fn get_bluesky_identity(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
) -> impl IntoResponse {
    match atproto_queries::get_bsky_profile_sync(&state.db, &user_id).await {
        Ok(Some(sync)) => Json(serde_json::json!({
            "did": sync.did,
            "bsky_handle": sync.bsky_handle,
            "display_name": sync.bsky_display_name,
            "description": sync.bsky_description,
            "banner_url": sync.bsky_banner_url,
            "followers_count": sync.bsky_followers_count,
            "follows_count": sync.bsky_follows_count,
            "last_sync": sync.last_profile_sync,
        }))
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No Bluesky account found for user"})),
        )
            .into_response(),
        Err(e) => {
            error!(error = %e, "Failed to fetch Bluesky identity");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Database error"})),
            )
                .into_response()
        }
    }
}

/// POST /api/messages/{id}/share-bluesky — share a message to Bluesky.
pub async fn share_to_bluesky(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(message_id): Path<String>,
) -> impl IntoResponse {
    info!(user_id = %auth.user_id, message_id = %message_id, "share_to_bluesky request received");
    // Check if already shared
    match atproto_queries::get_shared_post(&state.db, &message_id, &auth.user_id).await {
        Ok(Some(uri)) => {
            return Json(serde_json::json!({
                "success": true,
                "already_shared": true,
                "post_uri": uri,
            }))
            .into_response();
        }
        Ok(None) => {}
        Err(e) => {
            error!(error = %e, "Failed to check shared post");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Database error"})),
            )
                .into_response();
        }
    }

    // Get the message content
    let msg = match messages::get_message_by_id(&state.db, &message_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Message not found"})),
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch message");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Database error"})),
            )
                .into_response();
        }
    };

    // Verify the user owns this message
    if msg.sender_id != auth.user_id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "You can only share your own messages"})),
        )
            .into_response();
    }

    // Truncate to 300 graphemes for Bluesky's limit
    let text: String = msg.content.chars().take(300).collect();

    // Create the Bluesky post via PDS
    let public_url = &state.auth_config.public_url;
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);
    let redirect_uri = format!("{}/api/auth/atproto/callback", public_url);

    let post_record = serde_json::json!({
        "$type": "app.bsky.feed.post",
        "text": text,
        "createdAt": chrono::Utc::now().to_rfc3339(),
    });

    match crate::web::pds_client::create_record(
        &state.db,
        &auth.user_id,
        "app.bsky.feed.post",
        &post_record,
        &state.atproto.signing_key,
        &client_id,
        &redirect_uri,
    )
    .await
    {
        Ok(resp) => {
            // Record the share
            let share_id = Uuid::new_v4().to_string();
            let _ = atproto_queries::insert_shared_post(
                &state.db,
                &share_id,
                &message_id,
                &auth.user_id,
                &resp.uri,
                &resp.cid,
            )
            .await;

            Json(serde_json::json!({
                "success": true,
                "post_uri": resp.uri,
                "cid": resp.cid,
            }))
            .into_response()
        }
        Err(e) => {
            error!(error = %e, "Failed to share to Bluesky");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Failed to share to Bluesky"})),
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
    match crate::web::atproto_records::is_sync_enabled(&state.db, &auth.user_id).await {
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

    match crate::web::atproto_records::set_sync_enabled(&state.db, &auth.user_id, enabled).await {
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

// ── Sticker endpoints ──

pub async fn list_server_stickers(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> impl IntoResponse {
    let is_member = servers::is_server_member(&state.db, &server_id, &auth.user_id)
        .await
        .unwrap_or(false);
    if !is_member {
        return (StatusCode::FORBIDDEN, "Not a member of this server").into_response();
    }

    match stickers::list_stickers(&state.db, &server_id).await {
        Ok(rows) => {
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
    if let Err(resp) = check_server_permission(
        &state.db,
        &server_id,
        &user.user_id,
        Permissions::MANAGE_SERVER,
    )
    .await
    {
        return resp.into_response();
    }

    let name = body.name.trim().to_lowercase();
    if name.is_empty() || name.len() > 32 {
        return (
            StatusCode::BAD_REQUEST,
            "Sticker name must be 1-32 characters",
        )
            .into_response();
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return (
            StatusCode::BAD_REQUEST,
            "Sticker name must be alphanumeric with underscores",
        )
            .into_response();
    }

    let id = Uuid::new_v4().to_string();
    match stickers::insert_sticker(
        &state.db,
        &id,
        &server_id,
        &name,
        &body.image_url,
        body.description.as_deref(),
        &user.user_id,
    )
    .await
    {
        Ok(()) => Json(serde_json::json!({
            "id": id,
            "server_id": server_id,
            "name": name,
            "image_url": body.image_url,
            "description": body.description,
        }))
        .into_response(),
        Err(e) if e.to_string().contains("UNIQUE") => (
            StatusCode::CONFLICT,
            "A sticker with that name already exists",
        )
            .into_response(),
        Err(e) => {
            error!(error = %e, "Failed to create sticker");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

pub async fn delete_server_sticker(
    State(state): State<Arc<AppState>>,
    Path((server_id, sticker_id)): Path<(String, String)>,
    user: AuthUser,
) -> impl IntoResponse {
    if let Err(resp) = check_server_permission(
        &state.db,
        &server_id,
        &user.user_id,
        Permissions::MANAGE_SERVER,
    )
    .await
    {
        return resp.into_response();
    }

    match stickers::delete_sticker(&state.db, &sticker_id, &server_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Sticker not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to delete sticker");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── Cross-server emoji endpoint ──

pub async fn list_user_emoji(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> impl IntoResponse {
    match emoji::list_emoji_for_user_servers(&state.db, &user.user_id).await {
        Ok(rows) => {
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
    if let Err(resp) = check_server_permission(
        &state.db,
        &server_id,
        &user.user_id,
        Permissions::MANAGE_SERVER,
    )
    .await
    {
        return resp.into_response();
    }

    let allow_external = body.allow_external_emoji.unwrap_or(true);
    let shareable = body.shareable_emoji.unwrap_or(true);

    match servers::update_emoji_settings(&state.db, &server_id, allow_external, shareable).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            error!(error = %e, "Failed to update emoji settings");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
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
        let json = r#"{"content": "Hello from webhook", "username": "Bot", "avatar_url": "https://example.com/bot.png"}"#;
        let req: WebhookExecuteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "Hello from webhook");
        assert_eq!(req.username, Some("Bot".into()));
        assert_eq!(req.avatar_url, Some("https://example.com/bot.png".into()));
    }

    #[test]
    fn test_webhook_execute_request_content_only() {
        let json = r#"{"content": "test message"}"#;
        let req: WebhookExecuteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "test message");
        assert!(req.username.is_none());
        assert!(req.avatar_url.is_none());
    }

    #[test]
    fn test_webhook_execute_request_missing_content_fails() {
        let json = r#"{"username": "Bot"}"#;
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
