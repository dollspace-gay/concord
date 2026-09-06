use super::{
    Actor, MediaService, ProfileUpdate, authentication_error, dependency_error,
    schedule_replaced_media, validate_profile,
};

impl MediaService {
    pub async fn update_profile(
        &self,
        actor: &Actor,
        update: ProfileUpdate<'_>,
    ) -> Result<(), String> {
        validate_profile(&update)?;
        let avatar_id = update
            .avatar_url
            .map(|url| {
                crate::media::local_attachment_id(url).ok_or_else(|| {
                    "INVALID_INPUT: avatar must be a managed local upload".to_string()
                })
            })
            .transpose()?;
        let banner_id = update
            .banner_url
            .map(|url| {
                crate::media::local_attachment_id(url).ok_or_else(|| {
                    "INVALID_INPUT: banner must be a managed local upload".to_string()
                })
            })
            .transpose()?;
        let user_id = actor.user_id().as_str();
        let (_permit, mut transaction) = self.begin_write().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(authentication_error)?;
        let previous_avatar: Option<String> =
            sqlx::query_scalar("SELECT avatar_url FROM users WHERE id=?")
                .bind(user_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(dependency_error)?;
        let previous_banner: Option<String> =
            sqlx::query_scalar("SELECT banner_url FROM user_profiles WHERE user_id=?")
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(dependency_error)?
                .flatten();
        for (attachment_id, purpose) in [(avatar_id, "user_avatar"), (banner_id, "user_banner")] {
            if let Some(attachment_id) = attachment_id {
                let claimed = sqlx::query(
                    "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
                     WHERE id=? AND uploader_id=? AND managed_user_id=? AND media_purpose=? \
                     AND media_state='ready' AND message_id IS NULL",
                )
                .bind(attachment_id)
                .bind(user_id)
                .bind(user_id)
                .bind(purpose)
                .execute(&mut *transaction)
                .await
                .map_err(dependency_error)?;
                if claimed.rows_affected() != 1 {
                    return Err("CONFLICT: profile media is unavailable or already claimed".into());
                }
            }
        }
        sqlx::query(
            "INSERT INTO user_profiles(user_id,bio,pronouns,banner_url) VALUES(?,?,?,?) \
             ON CONFLICT(user_id) DO UPDATE SET bio=excluded.bio,pronouns=excluded.pronouns,\
             banner_url=excluded.banner_url,updated_at=datetime('now')",
        )
        .bind(user_id)
        .bind(update.bio)
        .bind(update.pronouns)
        .bind(update.banner_url)
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        if update.avatar_url.is_some() {
            sqlx::query("UPDATE users SET avatar_url=? WHERE id=?")
                .bind(update.avatar_url)
                .bind(user_id)
                .execute(&mut *transaction)
                .await
                .map_err(dependency_error)?;
        }
        for (previous, replacement, purpose) in [
            (previous_avatar.as_deref(), update.avatar_url, "user_avatar"),
            (previous_banner.as_deref(), update.banner_url, "user_banner"),
        ] {
            if replacement.is_some()
                && previous != replacement
                && let Some(previous_id) = previous.and_then(crate::media::local_attachment_id)
            {
                schedule_replaced_media(
                    &mut transaction,
                    previous_id,
                    purpose,
                    None,
                    Some(user_id),
                )
                .await?;
            }
        }
        transaction.commit().await.map_err(dependency_error)?;
        Ok(())
    }
}
