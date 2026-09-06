use super::{
    Actor, AuthService, AuthorizationError, AuthorizationService, AuthorizationStamp, ChannelRow,
    MessageRow, MessageSearch, Permissions, QueryBuilder, Sqlite,
};

impl AuthorizationService {
    pub async fn search_messages(
        &self,
        auth: &AuthService,
        actor: &Actor,
        request: MessageSearch<'_>,
    ) -> Result<(Vec<MessageRow>, i64, AuthorizationStamp), AuthorizationError> {
        let MessageSearch {
            server_id,
            query,
            requested_channel_id,
            sender,
            has_attachment,
            has_link,
            before,
            after,
            after_inclusive,
            limit,
            offset,
            cursor_created_at,
            cursor_message_id,
        } = request;
        let mut transaction = self.pool.begin().await?;
        auth.validate_actor_in(&mut transaction, actor)
            .await
            .map_err(AuthorizationError::Authentication)?;
        if !actor.scopes().contains("web") && !actor.scopes().contains("irc") {
            return Err(AuthorizationError::Unavailable);
        }
        let user_id = actor.user_id().as_str();
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let channels =
            sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels WHERE server_id=? ORDER BY id")
                .bind(server_id)
                .fetch_all(&mut *transaction)
                .await?;
        let mut readable = Vec::new();
        for channel in channels {
            let permissions = self
                .channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?;
            if permissions.contains(Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY)
                && self
                    .visibility_granted(&mut transaction, user_id, &channel, &authority)
                    .await?
            {
                readable.push(channel.id);
            }
        }
        if let Some(requested) = requested_channel_id {
            if !readable.iter().any(|id| id == requested) {
                return Err(AuthorizationError::Unavailable);
            }
            readable.retain(|id| id == requested);
        }
        if readable.is_empty() {
            let stamp = self
                .authorization_stamp(&mut transaction, server_id, &readable)
                .await?;
            transaction.commit().await?;
            return Ok((Vec::new(), 0, stamp));
        }

        // Keep the authorized set on this transaction's SQLite connection.
        // Inserts use a fixed two-parameter statement, avoiding an unbounded
        // `IN (?, …)` list while retaining every readable channel for global
        // ordering and counts.
        sqlx::query(
            "CREATE TEMP TABLE IF NOT EXISTS search_readable_channels(\
             channel_id TEXT PRIMARY KEY) WITHOUT ROWID",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM search_readable_channels")
            .execute(&mut *transaction)
            .await?;
        for channel_id in &readable {
            sqlx::query("INSERT INTO search_readable_channels(channel_id) VALUES(?)")
                .bind(channel_id)
                .execute(&mut *transaction)
                .await?;
        }

        let safe_query = query.map(|query| format!("\"{}\"", query.replace('"', "")));
        let append_predicate = |builder: &mut QueryBuilder<Sqlite>, include_cursor: bool| {
            builder.push(" FROM messages m ");
            if safe_query.is_some() {
                builder.push("JOIN messages_fts f ON m.rowid=f.rowid ");
            }
            builder.push("WHERE ");
            if let Some(safe_query) = &safe_query {
                builder
                    .push("f.content MATCH ")
                    .push_bind(safe_query.clone())
                    .push(" AND ");
            }
            builder
                .push("m.server_id=")
                .push_bind(server_id.to_owned())
                .push(
                    " AND m.deleted_at IS NULL AND m.channel_id IN (\
                     SELECT channel_id FROM search_readable_channels)",
                );
            if let Some(sender) = sender {
                builder
                    .push(" AND (m.sender_id=")
                    .push_bind(sender.to_owned())
                    .push(" OR m.sender_nick COLLATE NOCASE=")
                    .push_bind(sender.to_owned())
                    .push(")");
            }
            if has_attachment {
                builder.push(" AND EXISTS(SELECT 1 FROM attachments a WHERE a.message_id=m.id)");
            }
            if has_link {
                builder.push(" AND (m.content LIKE '%http://%' OR m.content LIKE '%https://%')");
            }
            if let Some(before) = before {
                builder
                    .push(" AND julianday(m.created_at)<julianday(")
                    .push_bind(before.to_owned())
                    .push(")");
            }
            if let Some(after) = after {
                builder
                    .push(if after_inclusive {
                        " AND julianday(m.created_at)>=julianday("
                    } else {
                        " AND julianday(m.created_at)>julianday("
                    })
                    .push_bind(after.to_owned())
                    .push(")");
            }
            if include_cursor
                && let (Some(created_at), Some(message_id)) = (cursor_created_at, cursor_message_id)
            {
                builder
                    .push(" AND (julianday(m.created_at)<julianday(")
                    .push_bind(created_at.to_owned())
                    .push(") OR (julianday(m.created_at)=julianday(")
                    .push_bind(created_at.to_owned())
                    .push(") AND m.id<")
                    .push_bind(message_id.to_owned())
                    .push("))");
            }
        };
        let mut count_builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*)");
        append_predicate(&mut count_builder, false);
        let total: i64 = count_builder
            .build_query_scalar()
            .fetch_one(&mut *transaction)
            .await?;
        let mut rows_builder = QueryBuilder::<Sqlite>::new(
            "SELECT m.id,m.server_id,m.channel_id,m.sender_id,m.sender_nick,m.content,m.created_at,m.target_user_id,m.edited_at,m.deleted_at,m.reply_to_id",
        );
        append_predicate(&mut rows_builder, true);
        rows_builder
            .push(" ORDER BY julianday(m.created_at) DESC,m.id DESC LIMIT ")
            .push_bind(limit.clamp(1, 50))
            .push(" OFFSET ")
            .push_bind(offset.clamp(0, 10_000));
        let rows = rows_builder
            .build_query_as::<MessageRow>()
            .fetch_all(&mut *transaction)
            .await?;
        let stamp = self
            .authorization_stamp(&mut transaction, server_id, &readable)
            .await?;
        transaction.commit().await?;
        Ok((rows, total, stamp))
    }
}
