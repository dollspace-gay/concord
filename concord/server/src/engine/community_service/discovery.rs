use super::{
    Actor, AuthorizationStamp, CommunityError, CommunityService, Permissions, PublicInvitePreview,
    PublicInvitePreviewError, PublicInviteRow, ServerId, ServerRow, UpdateCommunityParams, Uuid,
};

impl CommunityService {
    pub async fn update_community(
        &self,
        actor: &Actor,
        params: &UpdateCommunityParams<'_>,
    ) -> Result<bool, CommunityError> {
        let server_id = params.server_id.as_str();
        if params.description.is_some_and(|value| value.len() > 2_000)
            || params.welcome.is_some_and(|value| value.len() > 2_000)
            || params.rules.is_some_and(|value| value.len() > 20_000)
            || params
                .category
                .is_some_and(|value| value.len() > 100 || value.chars().any(char::is_control))
        {
            return Err(CommunityError::InvalidInput("invalid community settings"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        sqlx::query("UPDATE servers SET description=?,is_discoverable=?,welcome_message=?,rules_text=?,category=?,rules_version=rules_version+CASE WHEN rules_text IS NOT ? THEN 1 ELSE 0 END,updated_at=datetime('now') WHERE id=?")
            .bind(params.description).bind(i64::from(params.discoverable)).bind(params.welcome).bind(params.rules).bind(params.category).bind(params.rules).bind(server_id)
            .execute(&mut *tx).await?;
        let rules_accepted: bool = sqlx::query_scalar(
            "SELECT accepted_rules_version=(SELECT rules_version FROM servers WHERE id=?) \
             FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        tx.commit().await?;
        Ok(rules_accepted)
    }

    pub async fn get_community(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(ServerRow, bool, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await?;
        let server = sqlx::query_as::<_, ServerRow>("SELECT * FROM servers WHERE id=?")
            .bind(server_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CommunityError::Forbidden)?;
        let rules_accepted: bool = sqlx::query_scalar(
            "SELECT accepted_rules_version=(SELECT rules_version FROM servers WHERE id=?) \
             FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((server, rules_accepted, stamp))
    }

    pub async fn discover(
        &self,
        actor: &Actor,
        category: Option<&str>,
    ) -> Result<Vec<ServerRow>, CommunityError> {
        if category.is_some_and(|value| {
            value.is_empty() || value.len() > 100 || value.chars().any(char::is_control)
        }) {
            return Err(CommunityError::InvalidInput("invalid community category"));
        }
        let mut tx = self.pool.begin().await?;
        self.auth.validate_actor_in(&mut tx, actor).await?;
        let rows = match category {
            Some(category) => {
                sqlx::query_as::<_, ServerRow>(
                    "SELECT * FROM servers WHERE is_discoverable=1 AND category=? \
                     ORDER BY name,id LIMIT 100",
                )
                .bind(category)
                .fetch_all(&mut *tx)
                .await?
            }
            None => {
                sqlx::query_as::<_, ServerRow>(
                    "SELECT * FROM servers WHERE is_discoverable=1 ORDER BY name,id LIMIT 100",
                )
                .fetch_all(&mut *tx)
                .await?
            }
        };
        tx.commit().await?;
        Ok(rows)
    }

    pub async fn discover_public(
        &self,
        category: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ServerRow>, CommunityError> {
        if category.is_some_and(|value| {
            value.is_empty() || value.len() > 100 || value.chars().any(char::is_control)
        }) {
            return Err(CommunityError::InvalidInput("invalid community category"));
        }
        let rows = sqlx::query_as::<_, ServerRow>(
            "SELECT * FROM servers WHERE is_discoverable=1 \
             AND (? IS NULL OR category=?) ORDER BY name,id LIMIT ? OFFSET ?",
        )
        .bind(category)
        .bind(category)
        .bind(limit.clamp(1, 100))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn public_invite_preview(
        &self,
        code: &str,
    ) -> Result<Option<PublicInvitePreview>, PublicInvitePreviewError> {
        let invitation: Option<PublicInviteRow> = sqlx::query_as(
            "SELECT i.code,s.id AS server_id,i.expires_at,i.max_uses,i.use_count, \
             s.name AS server_name,s.icon_url AS server_icon_url \
             FROM invites i JOIN servers s ON s.id=i.server_id WHERE i.code=?",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(PublicInvitePreviewError::Database)?;
        if let Some(invitation) = invitation {
            let expired = invitation.expires_at.is_some_and(|expires_at| {
                expires_at < chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
            });
            if expired
                || invitation
                    .max_uses
                    .is_some_and(|maximum| invitation.use_count >= maximum)
            {
                return Err(PublicInvitePreviewError::ExpiredOrExhausted);
            }
            return Ok(Some(PublicInvitePreview {
                code: invitation.code,
                server_id: invitation.server_id,
                server_name: invitation.server_name,
                server_icon_url: invitation.server_icon_url,
                is_vanity: false,
            }));
        }
        let vanity: Option<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id,name,icon_url FROM servers WHERE vanity_code=?")
                .bind(code)
                .fetch_optional(&self.pool)
                .await
                .map_err(PublicInvitePreviewError::Database)?;
        Ok(vanity.map(
            |(server_id, server_name, server_icon_url)| PublicInvitePreview {
                code: code.to_owned(),
                server_id,
                server_name,
                server_icon_url,
                is_vanity: true,
            },
        ))
    }

    pub async fn set_vanity_code(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        vanity_code: Option<&str>,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        if let Some(vanity_code) = vanity_code {
            crate::engine::validation::validate_vanity_code(vanity_code)
                .map_err(|_| CommunityError::InvalidInput("invalid vanity code"))?;
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        if let Some(vanity_code) = vanity_code {
            let unavailable: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM servers WHERE vanity_code=? AND id<>?)",
            )
            .bind(vanity_code)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await?;
            if unavailable {
                return Err(CommunityError::Conflict("vanity code unavailable"));
            }
        }
        let updated =
            sqlx::query("UPDATE servers SET vanity_code=?,updated_at=datetime('now') WHERE id=?")
                .bind(vanity_code)
                .bind(server_id)
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        let audit_id = Uuid::new_v4().to_string();
        let changes = serde_json::json!({"vanity_code": vanity_code}).to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut tx,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "server_vanity_update",
                target_type: Some("server"),
                target_id: Some(server_id),
                reason: None,
                changes: Some(&changes),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
