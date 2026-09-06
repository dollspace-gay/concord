use super::{
    Actor, AuthorizedUpload, ConversationAction, MediaIntent, MediaService, MediaUpload,
    Permissions, ServerMediaPurpose, StartMedia, UploadAuthorization, UploadReservation,
    UploadTarget, UserMediaPurpose, authentication_error, authorization_error, dependency_error,
};

impl MediaService {
    pub async fn authorize_upload(
        &self,
        actor: &Actor,
        target: UploadTarget<'_>,
        instance_max_bytes: u64,
    ) -> Result<AuthorizedUpload, String> {
        let purpose = target.purpose;
        let (intent, authorization, purpose_limit, images_only) = match purpose {
            "message" => {
                let conversation_id = match target.conversation_id {
                    Some(conversation_id)
                        if target.server_id.is_none() && target.channel.is_none() =>
                    {
                        conversation_id.to_owned()
                    }
                    Some(_) => {
                        return Err(
                            "INVALID_INPUT: use conversation_id or the legacy server/channel target"
                                .into(),
                        );
                    }
                    None => {
                        let (Some(server_id), Some(channel)) = (target.server_id, target.channel)
                        else {
                            return Err(
                                "INVALID_INPUT: message uploads require conversation_id or server_id and channel"
                                    .into(),
                            );
                        };
                        sqlx::query_scalar(
                            "SELECT v.id FROM channels c \
                             JOIN conversations v ON v.channel_id=c.id \
                             WHERE c.server_id=? AND (c.id=? OR c.name=?) LIMIT 1",
                        )
                        .bind(server_id)
                        .bind(channel)
                        .bind(channel)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(dependency_error)?
                        .ok_or_else(|| "FORBIDDEN: resource unavailable".to_string())?
                    }
                };
                (
                    MediaIntent::Message {
                        conversation_id: conversation_id.clone(),
                    },
                    UploadAuthorization::Conversation(conversation_id),
                    instance_max_bytes,
                    false,
                )
            }
            "emoji" | "sticker" | "server_avatar" | "server_member_avatar" => {
                let server_id = target
                    .server_id
                    .ok_or_else(|| "INVALID_INPUT: server media requires server_id".to_string())?
                    .to_owned();
                let (asset_purpose, limit) = match purpose {
                    "emoji" => (ServerMediaPurpose::Emoji, 256 * 1024),
                    "sticker" => (ServerMediaPurpose::Sticker, 512 * 1024),
                    "server_avatar" => (ServerMediaPurpose::Avatar, 8 * 1024 * 1024),
                    _ => (ServerMediaPurpose::MemberAvatar, 8 * 1024 * 1024),
                };
                (
                    MediaIntent::ServerAsset {
                        server_id: server_id.clone(),
                        purpose: asset_purpose,
                    },
                    UploadAuthorization::ManagedServer {
                        server_id,
                        member_asset: purpose == "server_member_avatar",
                    },
                    instance_max_bytes.min(limit),
                    true,
                )
            }
            "user_avatar" | "user_banner" => {
                let (asset_purpose, limit) = if purpose == "user_avatar" {
                    (UserMediaPurpose::Avatar, 8 * 1024 * 1024)
                } else {
                    (UserMediaPurpose::Banner, 16 * 1024 * 1024)
                };
                (
                    MediaIntent::UserAsset {
                        purpose: asset_purpose,
                    },
                    UploadAuthorization::OwnProfile,
                    instance_max_bytes.min(limit),
                    true,
                )
            }
            _ => return Err("INVALID_INPUT: unknown media purpose".into()),
        };
        self.authorize_upload_scope(actor, &authorization).await?;
        Ok(AuthorizedUpload {
            intent,
            authorization,
            max_bytes: purpose_limit,
            images_only,
        })
    }

    pub async fn reserve_upload(
        &self,
        actor: &Actor,
        plan: AuthorizedUpload,
        request: UploadReservation<'_>,
    ) -> Result<MediaUpload, crate::media::MediaError> {
        self.authorize_upload_scope(actor, &plan.authorization)
            .await
            .map_err(|_| crate::media::MediaError::Invalid)?;
        MediaUpload::start(
            self.pool.clone(),
            request.media_root,
            StartMedia {
                owner_id: actor.user_id().as_str(),
                intent: plan.intent,
                original_filename: request.filename,
                content_type: request.content_type,
                max_bytes: plan.max_bytes,
                per_user_bytes: request.per_user_bytes,
                total_bytes: request.total_bytes,
            },
        )
        .await
    }

    pub(super) async fn authorize_upload_scope(
        &self,
        actor: &Actor,
        authorization: &UploadAuthorization,
    ) -> Result<(), String> {
        let mut connection = self.pool.acquire().await.map_err(dependency_error)?;
        match authorization {
            UploadAuthorization::Conversation(conversation_id) => self
                .authorization
                .authorize_conversation_actor_in(
                    &mut connection,
                    &self.auth,
                    actor,
                    conversation_id,
                    ConversationAction::Send,
                )
                .await
                .map(|_| ())
                .map_err(authorization_error),
            UploadAuthorization::ManagedServer {
                server_id,
                member_asset,
            } => {
                if *member_asset {
                    self.authorization
                        .server_actor_permissions_in(&mut connection, &self.auth, actor, server_id)
                        .await
                        .map(|_| ())
                        .map_err(authorization_error)
                } else {
                    self.authorization
                        .require_server_actor_in(
                            &mut connection,
                            &self.auth,
                            actor,
                            server_id,
                            Permissions::MANAGE_SERVER,
                        )
                        .await
                        .map(|_| ())
                        .map_err(authorization_error)
                }
            }
            UploadAuthorization::OwnProfile => self
                .auth
                .validate_actor_in(&mut connection, actor)
                .await
                .map_err(authentication_error),
        }
    }
}
