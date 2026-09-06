use super::{AtprotoPublicationError, ChatEngine, Row};

impl ChatEngine {
    pub async fn request_atproto_publication(
        &self,
        actor: &crate::auth::authority::Actor,
        message_id: &str,
    ) -> Result<crate::db::queries::atproto::AtprotoPublication, AtprotoPublicationError> {
        use crate::db::queries::atproto::PublicationRequestError;
        crate::db::queries::atproto::request_publication(
            self.write_admission
                .as_ref()
                .ok_or(PublicationRequestError::Unavailable)?,
            &crate::engine::authorization::AuthorizationService::new(
                self.db
                    .as_ref()
                    .ok_or(PublicationRequestError::Unavailable)?
                    .clone(),
            ),
            self.auth
                .get()
                .ok_or(PublicationRequestError::Unavailable)?,
            actor,
            message_id,
        )
        .await
        .map_err(AtprotoPublicationError::from)
    }
    pub async fn list_atproto_publications(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<crate::db::queries::atproto::AtprotoPublicationStatus>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        self.auth
            .get()
            .ok_or("Authentication unavailable")?
            .validate_actor(actor)
            .await
            .map_err(|error| error.to_string())?;
        crate::db::queries::atproto::list_publications(pool, actor.user_id().as_str())
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn retry_atproto_publication(
        &self,
        actor: &crate::auth::authority::Actor,
        publication_id: &str,
    ) -> Result<crate::db::queries::atproto::AtprotoPublication, AtprotoPublicationError> {
        use crate::db::queries::atproto::PublicationRequestError;
        crate::db::queries::atproto::retry_publication(
            self.write_admission
                .as_ref()
                .ok_or(PublicationRequestError::Unavailable)?,
            &crate::engine::authorization::AuthorizationService::new(
                self.db
                    .as_ref()
                    .ok_or(PublicationRequestError::Unavailable)?
                    .clone(),
            ),
            self.auth
                .get()
                .ok_or(PublicationRequestError::Unavailable)?,
            actor,
            publication_id,
        )
        .await
        .map_err(AtprotoPublicationError::from)
    }
    pub async fn atproto_channel_publication_policy(
        &self,
        actor: &crate::auth::authority::Actor,
        channel_id: &str,
    ) -> Result<crate::db::queries::atproto::AtprotoChannelPublicationPolicy, String> {
        let admission = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = admission.begin().await.map_err(|e| e.to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let authorization = crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        );
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|e| e.to_string())?;
        let row = sqlx::query(
            "SELECT c.is_private,c.visibility_repair_required,c.parent_channel_id,c.channel_type,
                    c.atproto_publication_enabled,COALESCE(g.enabled,0)
             FROM channels c LEFT JOIN atproto_publication_grants g
               ON g.channel_id=c.id AND g.user_id=? WHERE c.id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(channel_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Channel unavailable")?;
        let eligible = row.get::<i64, _>(0) == 0
            && row.get::<i64, _>(1) == 0
            && row.get::<Option<String>, _>(2).is_none()
            && !matches!(
                row.get::<String, _>(3).as_str(),
                "public_thread" | "private_thread"
            );
        let policy = crate::db::queries::atproto::AtprotoChannelPublicationPolicy {
            channel_id: channel_id.to_owned(),
            eligible,
            channel_enabled: row.get::<i64, _>(4) == 1,
            user_granted: row.get::<i64, _>(5) == 1,
        };
        transaction.commit().await.map_err(|e| e.to_string())?;
        Ok(policy)
    }
    pub async fn configure_atproto_channel(
        &self,
        actor: &crate::auth::authority::Actor,
        channel_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let admission = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = admission.begin().await.map_err(|e| e.to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let authorization = crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        );
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                channel_id,
                crate::engine::authorization::ChannelAction::Manage,
            )
            .await
            .map_err(|e| e.to_string())?;
        let changed = sqlx::query(
            "UPDATE channels SET atproto_publication_enabled=?,authorization_version=authorization_version+1
             WHERE id=? AND is_private=0 AND visibility_repair_required=0
               AND parent_channel_id IS NULL
               AND channel_type NOT IN ('public_thread','private_thread')",
        )
        .bind(i64::from(enabled))
        .bind(channel_id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
        if changed.rows_affected() != 1 {
            return Err("Channel is not eligible for publication".into());
        }
        if !enabled {
            sqlx::query("UPDATE atproto_publications SET status='cancelled',safe_error_code='channel_disabled',updated_at=datetime('now') WHERE source_message_id IN (SELECT id FROM messages WHERE channel_id=?) AND status IN ('pending','update_pending')")
                .bind(channel_id).execute(&mut *transaction).await.map_err(|e| e.to_string())?;
        }
        transaction.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }
    pub async fn set_atproto_publication_grant(
        &self,
        actor: &crate::auth::authority::Actor,
        channel_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let admission = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = admission.begin().await.map_err(|e| e.to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let authorization = crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        );
        for action in [
            crate::engine::authorization::ChannelAction::View,
            crate::engine::authorization::ChannelAction::ReadHistory,
        ] {
            authorization
                .authorize_actor_in(&mut transaction, auth, actor, channel_id, action)
                .await
                .map_err(|e| e.to_string())?;
        }
        let eligible: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND is_private=0
              AND atproto_publication_enabled=1 AND visibility_repair_required=0
              AND parent_channel_id IS NULL
              AND channel_type NOT IN ('public_thread','private_thread'))",
        )
        .bind(channel_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
        if !eligible {
            return Err("Channel is not eligible for publication".into());
        }
        sqlx::query(
            "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled)
             VALUES(?,?,?)
             ON CONFLICT(user_id,channel_id) DO UPDATE SET
               enabled=excluded.enabled,grant_version=atproto_publication_grants.grant_version+1,
               updated_at=datetime('now')",
        )
        .bind(actor.user_id().as_str())
        .bind(channel_id)
        .bind(i64::from(enabled))
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
        if !enabled {
            sqlx::query("UPDATE atproto_publications SET status='cancelled',safe_error_code='grant_revoked',updated_at=datetime('now') WHERE user_id=? AND source_message_id IN (SELECT id FROM messages WHERE channel_id=?) AND status IN ('pending','update_pending')")
                .bind(actor.user_id().as_str()).bind(channel_id).execute(&mut *transaction).await.map_err(|e| e.to_string())?;
        }
        transaction.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
