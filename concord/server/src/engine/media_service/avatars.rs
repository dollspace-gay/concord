use super::{
    Actor, MediaService, MemberMediaUpdate, Permissions, authorization_error, dependency_error,
    schedule_replaced_media,
};

impl MediaService {
    pub async fn update_server_icon(
        &self,
        actor: &Actor,
        server_id: &str,
        icon_url: &str,
    ) -> Result<(), String> {
        let attachment_id = crate::media::local_attachment_id(icon_url).ok_or_else(|| {
            "INVALID_INPUT: server icon must be a managed local upload".to_string()
        })?;
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(authorization_error)?;
        let previous_icon: Option<String> =
            sqlx::query_scalar("SELECT icon_url FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(dependency_error)?;
        let claimed = sqlx::query(
            "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
             WHERE id=? AND uploader_id=? AND managed_server_id=? \
             AND media_purpose='server_avatar' AND media_state='ready' AND message_id IS NULL",
        )
        .bind(attachment_id)
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        if claimed.rows_affected() != 1 {
            return Err("CONFLICT: server icon upload is unavailable or already claimed".into());
        }
        sqlx::query("UPDATE servers SET icon_url=?,updated_at=datetime('now') WHERE id=?")
            .bind(icon_url)
            .bind(server_id)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        if previous_icon.as_deref() != Some(icon_url)
            && let Some(previous_id) = previous_icon
                .as_deref()
                .and_then(crate::media::local_attachment_id)
        {
            schedule_replaced_media(
                &mut transaction,
                previous_id,
                "server_avatar",
                Some(server_id),
                None,
            )
            .await?;
        }
        transaction.commit().await.map_err(dependency_error)?;
        Ok(())
    }

    pub async fn update_member_avatar(
        &self,
        actor: &Actor,
        server_id: &str,
        avatar_url: &str,
    ) -> Result<MemberMediaUpdate, String> {
        let attachment_id = crate::media::local_attachment_id(avatar_url).ok_or_else(|| {
            "INVALID_INPUT: member avatar must be a managed local upload".to_string()
        })?;
        let user_id = actor.user_id().as_str();
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .server_actor_permissions_in(&mut transaction, &self.auth, actor, server_id)
            .await
            .map_err(authorization_error)?;
        let previous_avatar: Option<String> = sqlx::query_scalar(
            "SELECT avatar_url FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        let claimed = sqlx::query(
            "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
             WHERE id=? AND uploader_id=? AND managed_user_id=? AND managed_server_id=? \
             AND media_purpose='server_member_avatar' AND media_state='ready' AND message_id IS NULL",
        )
        .bind(attachment_id)
        .bind(user_id)
        .bind(user_id)
        .bind(server_id)
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        if claimed.rows_affected() != 1 {
            return Err("CONFLICT: member avatar upload is unavailable or already claimed".into());
        }
        sqlx::query("UPDATE server_members SET avatar_url=? WHERE server_id=? AND user_id=?")
            .bind(avatar_url)
            .bind(server_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        if previous_avatar.as_deref() != Some(avatar_url)
            && let Some(previous_id) = previous_avatar
                .as_deref()
                .and_then(crate::media::local_attachment_id)
        {
            schedule_replaced_media(
                &mut transaction,
                previous_id,
                "server_member_avatar",
                Some(server_id),
                Some(user_id),
            )
            .await?;
        }
        let (nickname, username): (Option<String>, String) = sqlx::query_as(
            "SELECT sm.nickname,u.username FROM server_members sm \
             JOIN users u ON u.id=sm.user_id WHERE sm.server_id=? AND sm.user_id=?",
        )
        .bind(server_id)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(MemberMediaUpdate {
            nickname,
            username,
            avatar_url: avatar_url.to_owned(),
        })
    }
}
