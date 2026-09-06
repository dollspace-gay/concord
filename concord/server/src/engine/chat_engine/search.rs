use super::{
    ChatEngine, DecodingKey, EncodingKey, Header, SearchContinuationClaims, SearchError,
    SearchMessagesRequest, SearchResultsPage, Utc, Validation, decode, encode,
    normalize_channel_name, parse_search_query, search_fingerprint,
};

impl ChatEngine {
    /// Search messages in a server using full-text search.
    pub async fn search_messages(
        &self,
        actor: &crate::auth::authority::Actor,
        request: SearchMessagesRequest<'_>,
    ) -> Result<SearchResultsPage, SearchError> {
        let SearchMessagesRequest {
            server_id,
            query,
            channel_name,
            limit,
            offset,
            continuation,
        } = request;
        let pool = self.db.as_ref().ok_or_else(|| {
            SearchError::DependencyUnavailable(
                crate::engine::authorization::AuthorizationError::Unavailable,
            )
        })?;
        let plan = parse_search_query(query).map_err(SearchError::InvalidInput)?;
        if channel_name.is_some() && plan.channel.is_some() {
            return Err(SearchError::InvalidInput(
                "channel filter supplied twice".into(),
            ));
        }
        let effective_channel = channel_name.or(plan.channel.as_deref());

        // Resolve channel name to ID if provided (normalize for case-insensitive lookup)
        let channel_id = if let Some(ch_name) = effective_channel {
            let ch_name = normalize_channel_name(ch_name);
            Some(
                self.resolve_channel_id(server_id, &ch_name)
                    .map_err(SearchError::InvalidInput)?,
            )
        } else {
            None
        };

        let fingerprint = search_fingerprint(server_id, query, channel_id.as_deref());
        let decoded = continuation
            .map(|token| {
                let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
                validation.validate_exp = true;
                decode::<SearchContinuationClaims>(
                    token,
                    &DecodingKey::from_secret(self.search_token_secret.as_bytes()),
                    &validation,
                )
                .map(|data| data.claims)
                .map_err(|_| SearchError::InvalidContinuation)
            })
            .transpose()?;
        if decoded.as_ref().is_some_and(|claims| {
            claims.fingerprint != fingerprint
                || claims.credential_id != actor.credential_id().as_str()
        }) {
            return Err(SearchError::InvalidContinuation);
        }

        let auth = self.auth.get().ok_or_else(|| {
            SearchError::DependencyUnavailable(
                crate::engine::authorization::AuthorizationError::Unavailable,
            )
        })?;
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        let result_offset = decoded.as_ref().map_or(offset, |claims| claims.position);
        // Continuations are keyset cursors. Applying their logical display
        // position as a SQL OFFSET would skip a second page of rows.
        let query_offset = if decoded.is_some() { 0 } else { offset };
        let (mut rows, mut total, mut stamp) = authorization
            .search_messages(
                auth,
                actor,
                crate::engine::authorization::MessageSearch {
                    server_id,
                    query: plan.text.as_deref(),
                    requested_channel_id: channel_id.as_deref(),
                    sender: plan.sender.as_deref(),
                    has_attachment: plan.has_attachment,
                    has_link: plan.has_link,
                    before: plan.before.as_deref(),
                    after: plan.after.as_deref(),
                    after_inclusive: plan.after_inclusive,
                    limit,
                    offset: query_offset,
                    cursor_created_at: decoded
                        .as_ref()
                        .map(|claims| claims.before_created_at.as_str()),
                    cursor_message_id: decoded
                        .as_ref()
                        .map(|claims| claims.before_message_id.as_str()),
                },
            )
            .await
            .map_err(SearchError::from_authorization)?;
        let restarted = decoded
            .as_ref()
            .is_some_and(|claims| claims.authorization_version != stamp.server_version);
        let result_offset = if restarted { 0 } else { result_offset };
        if restarted {
            (rows, total, stamp) = authorization
                .search_messages(
                    auth,
                    actor,
                    crate::engine::authorization::MessageSearch {
                        server_id,
                        query: plan.text.as_deref(),
                        requested_channel_id: channel_id.as_deref(),
                        sender: plan.sender.as_deref(),
                        has_attachment: plan.has_attachment,
                        has_link: plan.has_link,
                        before: plan.before.as_deref(),
                        after: plan.after.as_deref(),
                        after_inclusive: plan.after_inclusive,
                        limit,
                        offset: 0,
                        cursor_created_at: None,
                        cursor_message_id: None,
                    },
                )
                .await
                .map_err(SearchError::from_authorization)?;
        }

        let next_continuation = if result_offset + rows.len() as i64 >= total {
            None
        } else {
            let last = rows.last().ok_or(SearchError::InvalidContinuation)?;
            let claims = SearchContinuationClaims {
                exp: (Utc::now() + chrono::Duration::minutes(15)).timestamp(),
                credential_id: actor.credential_id().as_str().to_owned(),
                fingerprint,
                authorization_version: stamp.server_version,
                before_created_at: last.created_at.clone(),
                before_message_id: last.id.clone(),
                position: result_offset + rows.len() as i64,
            };
            Some(
                encode(
                    &Header::default(),
                    &claims,
                    &EncodingKey::from_secret(self.search_token_secret.as_bytes()),
                )
                .map_err(|_| {
                    SearchError::DependencyUnavailable(
                        crate::engine::authorization::AuthorizationError::Unavailable,
                    )
                })?,
            )
        };

        let mut channel_names = std::collections::HashMap::new();
        for channel_id in rows.iter().filter_map(|row| row.channel_id.as_deref()) {
            if channel_names.contains_key(channel_id) {
                continue;
            }
            let name: String =
                sqlx::query_scalar("SELECT name FROM channels WHERE id=? AND server_id=?")
                    .bind(channel_id)
                    .bind(server_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|error| {
                        SearchError::DependencyUnavailable(
                            crate::engine::authorization::AuthorizationError::Database(error),
                        )
                    })?
                    .ok_or(SearchError::ResourceUnavailable)?;
            channel_names.insert(channel_id.to_owned(), name);
        }

        let results = rows
            .drain(..)
            .map(|row| {
                let channel_id = row.channel_id.unwrap_or_default();
                let channel_name = channel_names.get(&channel_id).cloned().unwrap_or_default();
                crate::engine::events::SearchResultMessage {
                    id: row.id,
                    from: row.sender_nick,
                    content: row.content,
                    timestamp: row.created_at,
                    channel_id,
                    channel_name,
                    edited_at: row.edited_at,
                }
            })
            .collect();

        Ok(SearchResultsPage {
            results,
            total_count: total,
            offset: result_offset,
            next_continuation,
            restarted,
            stamp,
        })
    }
}
