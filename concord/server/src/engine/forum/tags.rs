use super::{
    Actor, AuthService, AuthorizationService, ChannelAction, CreateForumTag, ForumError,
    ForumService, ForumTagInfo, ForumTagRow, Permissions, ThreadTagMutation, UpdateForumTag, Uuid,
    insert_audit, require_forum_channel, row_to_info, validate_tag,
};

impl ForumService {
    pub async fn create_tag(
        &self,
        auth: &AuthService,
        actor: &Actor,
        params: CreateForumTag<'_>,
    ) -> Result<ForumTagInfo, ForumError> {
        validate_tag(params.name, params.emoji, None)?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        let authorization = AuthorizationService::new(self.pool.clone());
        authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                params.server_id,
                params.channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        require_forum_channel(&mut transaction, params.server_id, params.channel_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM forum_tags WHERE channel_id=?")
            .bind(params.channel_id)
            .fetch_one(&mut *transaction)
            .await?;
        if count >= 20 {
            return Err(ForumError::Validation("forum tag limit reached"));
        }
        let position: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position),-1)+1 FROM forum_tags WHERE channel_id=?",
        )
        .bind(params.channel_id)
        .fetch_one(&mut *transaction)
        .await?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO forum_tags(id,channel_id,name,emoji,moderated,position) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(params.channel_id)
        .bind(params.name.trim())
        .bind(params.emoji)
        .bind(params.moderated)
        .bind(position)
        .execute(&mut *transaction)
        .await?;
        let changes =
            serde_json::json!({"moderated": params.moderated, "position": position}).to_string();
        insert_audit(
            &mut transaction,
            params.server_id,
            actor,
            "forum_tag_create",
            "forum_tag",
            &id,
            Some(&changes),
        )
        .await?;
        transaction.commit().await?;
        Ok(ForumTagInfo {
            id,
            name: params.name.trim().to_string(),
            emoji: params.emoji.map(str::to_string),
            moderated: params.moderated,
            position,
        })
    }

    pub async fn update_tag(
        &self,
        auth: &AuthService,
        actor: &Actor,
        params: UpdateForumTag<'_>,
    ) -> Result<ForumTagInfo, ForumError> {
        validate_tag(params.name, params.emoji, Some(params.position))?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                params.server_id,
                params.channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        require_forum_channel(&mut transaction, params.server_id, params.channel_id).await?;
        let updated = sqlx::query(
            "UPDATE forum_tags SET name=?,emoji=?,moderated=?,position=? \
             WHERE id=? AND channel_id=?",
        )
        .bind(params.name.trim())
        .bind(params.emoji)
        .bind(params.moderated)
        .bind(params.position)
        .bind(params.tag_id)
        .bind(params.channel_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ForumError::Unavailable);
        }
        let changes = serde_json::json!({
            "moderated": params.moderated,
            "position": params.position,
        })
        .to_string();
        insert_audit(
            &mut transaction,
            params.server_id,
            actor,
            "forum_tag_update",
            "forum_tag",
            params.tag_id,
            Some(&changes),
        )
        .await?;
        transaction.commit().await?;
        Ok(ForumTagInfo {
            id: params.tag_id.to_string(),
            name: params.name.trim().to_string(),
            emoji: params.emoji.map(str::to_string),
            moderated: params.moderated,
            position: params.position,
        })
    }

    pub async fn delete_tag(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
        tag_id: &str,
    ) -> Result<Vec<ThreadTagMutation>, ForumError> {
        let (_permit, mut transaction) = self.writes.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                server_id,
                channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        require_forum_channel(&mut transaction, server_id, channel_id).await?;
        let affected: Vec<(String, i64, i64, String)> = sqlx::query_as(
            "SELECT c.id,c.thread_tags_version,c.authorization_version,conversations.id \
             FROM thread_tags tt \
             JOIN channels c ON c.id=tt.thread_id \
             JOIN conversations ON conversations.channel_id=c.id \
             WHERE tt.tag_id=? ORDER BY c.id",
        )
        .bind(tag_id)
        .fetch_all(&mut *transaction)
        .await?;
        let deleted = sqlx::query("DELETE FROM forum_tags WHERE id=? AND channel_id=?")
            .bind(tag_id)
            .bind(channel_id)
            .execute(&mut *transaction)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(ForumError::Unavailable);
        }
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let mut mutations = Vec::with_capacity(affected.len());
        for (thread_id, old_version, authorization_version, conversation_id) in affected {
            let tag_ids: Vec<String> = sqlx::query_scalar(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(&thread_id)
            .fetch_all(&mut *transaction)
            .await?;
            let version: i64 = sqlx::query_scalar(
                "UPDATE channels SET thread_tags_version=thread_tags_version+1 \
                 WHERE id=? AND thread_tags_version=? RETURNING thread_tags_version",
            )
            .bind(&thread_id)
            .bind(old_version)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(ForumError::Conflict)?;
            sqlx::query(
                "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
                 VALUES('thread_tags',?,?,datetime('now')) \
                 ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
                    version=excluded.version,updated_at=excluded.updated_at",
            )
            .bind(&thread_id)
            .bind(version)
            .execute(&mut *transaction)
            .await?;
            let descriptor =
                serde_json::json!({"thread_id": thread_id, "tag_ids": tag_ids}).to_string();
            let event_sequence: i64 = sqlx::query_scalar(
                "INSERT INTO event_log( \
                    database_generation,conversation_id,event_kind,entity_type,entity_id, \
                    entity_version,authorization_version,actor_id,descriptor_json \
                 ) VALUES(?,?,'thread_tags_updated','thread_tags',?,?,?,?,?) \
                 RETURNING event_sequence",
            )
            .bind(&generation)
            .bind(conversation_id)
            .bind(&thread_id)
            .bind(version)
            .bind(authorization_version)
            .bind(actor.user_id().as_str())
            .bind(descriptor)
            .fetch_one(&mut *transaction)
            .await?;
            sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
                .bind(event_sequence)
                .execute(&mut *transaction)
                .await?;
            let changes = serde_json::json!({
                "removed_deleted_tag_id": tag_id,
                "new_tag_ids": tag_ids,
                "version": version,
            })
            .to_string();
            insert_audit(
                &mut transaction,
                server_id,
                actor,
                "thread_tags_update",
                "thread",
                &thread_id,
                Some(&changes),
            )
            .await?;
            mutations.push(ThreadTagMutation {
                thread_id,
                version,
                tag_ids,
            });
        }
        insert_audit(
            &mut transaction,
            server_id,
            actor,
            "forum_tag_delete",
            "forum_tag",
            tag_id,
            None,
        )
        .await?;
        transaction.commit().await?;
        Ok(mutations)
    }

    pub async fn list_tags(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
    ) -> Result<Vec<ForumTagInfo>, ForumError> {
        let mut transaction = self.pool.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                channel_id,
                ChannelAction::View,
            )
            .await?;
        require_forum_channel(&mut transaction, server_id, channel_id).await?;
        let rows = sqlx::query_as::<_, ForumTagRow>(
            "SELECT * FROM forum_tags WHERE channel_id=? ORDER BY position,id",
        )
        .bind(channel_id)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows.into_iter().map(row_to_info).collect())
    }
}
