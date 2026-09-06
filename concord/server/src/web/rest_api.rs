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

use tokio::io::SeekFrom;

use tokio_util::io::ReaderStream;

use super::app_state::AppState;

use super::auth_middleware::{AuthUser, auth_error_response};

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

#[cfg(test)]
mod tests;

mod accounts;
mod administration;
mod atproto;
mod discovery;
mod emoji;
mod history;
mod profiles;
mod search;
mod server_folders;
mod servers;
mod stickers;
mod uploads;
mod user_emoji;
mod webhooks;
pub use accounts::AuthStatusResponse;
pub use accounts::CreateTokenRequest;
pub use accounts::CreateTokenResponse;
pub use accounts::IrcTokenInfo;
pub use accounts::PublicUserProfile;
pub use accounts::UserProfile;
pub use accounts::auth_status;
pub use accounts::create_irc_token;
pub use accounts::delete_irc_token;
pub use accounts::get_me;
pub use accounts::get_user_profile;
pub use accounts::list_irc_tokens;
pub use administration::SetAdminRequest;
pub use administration::admin_delete_server;
pub use administration::admin_list_servers;
pub use administration::admin_set_admin;
pub use atproto::configure_atproto_channel;
pub use atproto::get_atproto_channel_policy;
pub use atproto::get_atproto_sync_setting;
pub use atproto::get_bluesky_identity;
pub use atproto::list_atproto_publications;
pub use atproto::retry_atproto_publication;
pub use atproto::share_to_bluesky;
pub use atproto::sync_bluesky_profile;
pub use atproto::update_atproto_sync_setting;
pub use discovery::DiscoverParams;
pub use discovery::discover_servers;
pub use discovery::get_invite_preview;
pub use emoji::CreateEmojiRequest;
pub use emoji::EmojiResponse;
pub use emoji::create_server_emoji;
pub use emoji::delete_server_emoji;
pub use emoji::list_server_emoji;
pub use history::ChannelListParams;
pub use history::HistoryParams;
pub use history::HistoryResponse;
pub use history::get_channel_history;
pub use history::get_channels;
pub use profiles::UpdateProfileRequest;
pub use profiles::get_user_full_profile;
pub use profiles::update_profile;
pub use search::SearchParams;
pub use search::search_messages;
pub use server_folders::ServerFolderRequest;
pub use server_folders::get_server_folders;
pub use server_folders::get_server_limits;
pub use server_folders::replace_server_folders;
pub use servers::CreateServerRequest;
pub use servers::UpdateServerMediaRequest;
pub use servers::create_server;
pub use servers::delete_server;
pub use servers::get_server;
pub use servers::get_server_channel_history;
pub use servers::list_server_channels;
pub use servers::list_server_members;
pub use servers::list_servers;
use servers::organization_error_response;
pub use servers::update_server_media;
pub use servers::update_server_member_media;
pub use stickers::CreateStickerRequest;
pub use stickers::create_server_sticker;
pub use stickers::delete_server_sticker;
pub use stickers::list_server_stickers;
pub use uploads::UploadQuery;
pub use uploads::UploadResponse;
pub use uploads::delete_upload;
pub use uploads::get_upload;
pub use uploads::upload_file;
pub use user_emoji::UpdateEmojiSettingsRequest;
pub use user_emoji::UserEmojiQuery;
pub use user_emoji::list_user_emoji;
pub use user_emoji::update_emoji_settings;
pub use webhooks::WebhookDeliveryListParams;
pub use webhooks::WebhookExecuteRequest;
pub use webhooks::execute_webhook;
pub use webhooks::list_webhook_deliveries;
pub use webhooks::retry_webhook_delivery;
pub use webhooks::test_outgoing_webhook;
