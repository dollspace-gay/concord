use super::{
    Arc, ChatEngine, ChatEvent, ConnectionId, Instant, ReplyInfo, UserSession, Utc, Uuid, error,
    normalize_channel_name,
};
use crate::engine::validation;

impl ChatEngine {
    /// Legacy in-memory compatibility path retained only while non-message callers migrate.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn send_message(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target: &str,
        content: &str,
        reply_to_id: Option<&str>,
        attachment_ids: Option<&[String]>,
        nonce: Option<&str>,
    ) -> Result<(), String> {
        validation::validate_message_with_limit(content, self.max_message_length)?;
        let content = &validation::sanitize_html(content);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        if !self.message_limiter.check(&session.nickname) {
            return Err("Rate limit exceeded. Please slow down.".into());
        }

        // Enforce timeout: timed-out users cannot send messages
        if let Some(pool) = &self.db
            && let Some(ref uid) = session.user_id
        {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let uid = uid.clone();
            let timed_out = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Ok(Some(until)) =
                        crate::db::queries::moderation::get_member_timeout(&pool, &srv, &uid).await
                        && let Ok(timeout_dt) =
                            chrono::NaiveDateTime::parse_from_str(&until, "%Y-%m-%d %H:%M:%S")
                    {
                        let timeout_utc = timeout_dt.and_utc();
                        return timeout_utc > chrono::Utc::now();
                    }
                    false
                })
            });
            if timed_out {
                return Err("You are timed out and cannot send messages".into());
            }
        }

        // Enforce slow mode: check per-channel cooldown.
        // Uses both a DB query and an in-memory DashMap cache to prevent
        // concurrent requests (e.g. two browser tabs) from bypassing the check.
        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let tgt = target.to_string();
            let sender_uid = session
                .user_id
                .clone()
                .unwrap_or_else(|| session.nickname.clone());
            let slowmode_map = &self.slowmode_last_sent;
            let slow_err = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Ok(Some(ch)) =
                        crate::db::queries::channels::get_channel_by_name(&pool, &srv, &tgt).await
                        && ch.slowmode_seconds > 0
                    {
                        let cooldown_dur =
                            std::time::Duration::from_secs(ch.slowmode_seconds as u64);
                        let cache_key = (sender_uid.clone(), ch.id.clone());

                        // Check the in-memory cache first (catches concurrent sends)
                        if let Some(last_instant) = slowmode_map.get(&cache_key)
                            && last_instant.elapsed() < cooldown_dur
                        {
                            return Some(format!(
                                "Slow mode: wait {} seconds between messages",
                                ch.slowmode_seconds
                            ));
                        }

                        // Also check DB (catches sends from before this process started)
                        if let Ok(Some(last)) =
                            crate::db::queries::messages::get_last_user_message_time(
                                &pool,
                                &ch.id,
                                &sender_uid,
                            )
                            .await
                            && let Ok(last_dt) =
                                chrono::NaiveDateTime::parse_from_str(&last, "%Y-%m-%d %H:%M:%S")
                        {
                            let last_utc = last_dt.and_utc();
                            let cooldown = chrono::Duration::seconds(ch.slowmode_seconds as i64);
                            if chrono::Utc::now() - last_utc < cooldown {
                                return Some(format!(
                                    "Slow mode: wait {} seconds between messages",
                                    ch.slowmode_seconds
                                ));
                            }
                        }

                        // Both checks passed — record this send in the in-memory cache
                        slowmode_map.insert(cache_key, Instant::now());
                    }
                    None
                })
            });
            if let Some(err) = slow_err {
                return Err(err);
            }
        }

        // Evaluate automod rules (keyword, mention_spam, link_filter)
        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let content_clone = content.to_string();
            let automod_err = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let rules = crate::db::queries::automod::get_enabled_rules(&pool, &srv)
                        .await
                        .unwrap_or_default();
                    for rule in rules {
                        let triggered = match rule.rule_type.as_str() {
                            "keyword" => {
                                // Config: {"words":["bad","spam"]}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    if let Some(words) =
                                        config.get("words").and_then(|w| w.as_array())
                                    {
                                        let lower = content_clone.to_lowercase();
                                        let msg_words: Vec<&str> =
                                            lower.split(|c: char| !c.is_alphanumeric()).collect();
                                        words.iter().any(|w| {
                                            w.as_str().is_some_and(|kw| {
                                                let kw_lower = kw.to_lowercase();
                                                msg_words.iter().any(|mw| *mw == kw_lower)
                                            })
                                        })
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            "mention_spam" => {
                                // Config: {"max_mentions":5}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    let max = config
                                        .get("max_mentions")
                                        .and_then(|m| m.as_i64())
                                        .unwrap_or(5)
                                        as usize;
                                    let mention_count = content_clone.matches('@').count();
                                    mention_count > max
                                } else {
                                    false
                                }
                            }
                            "link_filter" => {
                                // Config: {"block_all":true}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    let block_all = config
                                        .get("block_all")
                                        .and_then(|b| b.as_bool())
                                        .unwrap_or(false);
                                    if block_all {
                                        content_clone.contains("http://")
                                            || content_clone.contains("https://")
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };
                        if triggered {
                            return Some(format!("Message blocked by automod rule: {}", rule.name));
                        }
                    }
                    None
                })
            });
            if let Some(err) = automod_err {
                return Err(err);
            }
        }

        // Build reply info if replying to a message
        let reply_to: Option<ReplyInfo> = if let Some(ref_id) = reply_to_id {
            if let Some(pool) = &self.db {
                // Synchronous lookup via block_in_place — reply info is needed before broadcast
                let pool = pool.clone();
                let ref_id = ref_id.to_string();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        match crate::db::queries::messages::get_message_by_id(&pool, &ref_id).await
                        {
                            Ok(Some(row)) => Some(ReplyInfo {
                                id: row.id,
                                from: row.sender_nick,
                                content_preview: row.content.chars().take(100).collect::<String>(),
                            }),
                            _ => None,
                        }
                    })
                })
            } else {
                None
            }
        } else {
            None
        };

        // Look up attachment metadata if attachment_ids provided
        let attachments: Option<Vec<crate::engine::events::AttachmentInfo>> = if let Some(ids) =
            attachment_ids
            && !ids.is_empty()
        {
            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let ids = ids.to_vec();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let infos =
                            crate::db::queries::attachments::get_attachments_by_ids(&pool, &ids)
                                .await
                                .unwrap_or_default();
                        if infos.is_empty() {
                            None
                        } else {
                            Some(
                                infos
                                    .into_iter()
                                    .map(|a| crate::engine::events::AttachmentInfo {
                                        id: a.id.clone(),
                                        filename: a.original_filename,
                                        content_type: a.content_type,
                                        file_size: a.file_size,
                                        url: format!("/api/uploads/{}", a.id),
                                    })
                                    .collect(),
                            )
                        }
                    })
                })
            } else {
                None
            }
        } else {
            None
        };

        let msg_id = crate::engine::ids::MessageId::from(Uuid::new_v4());
        let event = ChatEvent::Message {
            id: msg_id.clone(),
            server_id: Some(server_id.to_string()),
            conversation_id: None,
            from: session.nickname.clone(),
            target: target.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            avatar_url: session.avatar_url.clone(),
            reply_to: reply_to.clone(),
            attachments: attachments.clone(),
        };

        if target.starts_with('#') {
            let channel_name = normalize_channel_name(target);
            let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

            let channel = self
                .channels
                .get(&channel_id)
                .ok_or(format!("No such channel: {channel_name}"))?;

            // Check if thread is archived
            if channel.archived {
                return Err("This thread is archived and no longer accepts messages".to_string());
            }

            if !channel.members.contains(&session_id) {
                return Err(format!("You are not in channel {channel_name}"));
            }

            // Private channel access control: require VIEW_CHANNELS even if user is in-memory member
            let ch_is_private = channel.is_private;
            drop(channel);

            if ch_is_private {
                if let Some(ref uid) = session.user_id
                    && self.db.is_some()
                {
                    let has_view = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let perms = self
                                .get_effective_permissions(server_id, Some(&channel_id), uid)
                                .await;
                            perms.contains(crate::engine::permissions::Permissions::VIEW_CHANNELS)
                        })
                    });
                    if !has_view {
                        return Err(
                            "You do not have permission to access this private channel".to_string()
                        );
                    }
                } else if session.user_id.is_none() {
                    return Err("Authentication required to access private channels".to_string());
                }
            }

            // Check SEND_MESSAGES permission (only when DB is available for role/override lookups)
            if self.db.is_some() {
                let sender_user_id = session
                    .user_id
                    .clone()
                    .unwrap_or_else(|| session_id.to_string());
                let perms = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(self.get_effective_permissions(
                        server_id,
                        Some(&channel_id),
                        &sender_user_id,
                    ))
                });
                if !perms.contains(crate::engine::permissions::Permissions::SEND_MESSAGES) {
                    return Err(
                        "You do not have permission to send messages in this channel".to_string(),
                    );
                }
            }

            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let id = msg_id.to_string();
                let srv = server_id.to_string();
                let ch = channel_id.clone();
                let sid = session_id.to_string();
                let nick = session.nickname.clone();
                let uid = session.user_id.clone().unwrap_or_else(|| sid.clone());
                let msg = content.to_string();
                let reply_id = reply_to_id.map(|s| s.to_string());
                let att_ids = attachment_ids.map(|ids| ids.to_vec());
                tokio::spawn(async move {
                    let params = crate::db::queries::messages::InsertMessageParams {
                        id: &id,
                        server_id: &srv,
                        channel_id: &ch,
                        sender_id: &uid,
                        sender_nick: &nick,
                        content: &msg,
                        reply_to_id: reply_id.as_deref(),
                    };
                    if let Err(e) =
                        crate::db::queries::messages::insert_message(&pool, &params).await
                    {
                        error!(error = %e, "failed to persist message");
                    }
                    // Link attachments to the message (use user_id, not session_id)
                    if let Some(att_ids) = att_ids
                        && let Err(e) =
                            crate::db::queries::attachments::link_attachments_to_message(
                                &pool, &id, &att_ids, &uid,
                            )
                            .await
                    {
                        error!(error = %e, "failed to link attachments");
                    }
                });
            }

            self.broadcast_to_channel(&channel_id, &event, Some(session_id));

            // Send MessageAck back to the sender with the server-generated message ID
            if let Some(sender_session) = self.sessions.get(&session_id) {
                let _ = sender_session.send(ChatEvent::MessageAck {
                    id: msg_id.clone(),
                    server_id: server_id.to_string(),
                    channel: target.to_string(),
                    conversation_id: None,
                    request_id: nonce.unwrap_or_default().to_owned(),
                    client_message_id: nonce.unwrap_or_default().to_owned(),
                    sequence: String::new(),
                    persisted_at: Utc::now().to_rfc3339(),
                    replayed: false,
                    nonce: nonce.map(|s| s.to_string()),
                });
            }

            // Async link embed unfurling — extract URLs and resolve OG metadata
            let urls = crate::engine::embeds::extract_urls(content);
            if !urls.is_empty()
                && let Some(pool) = &self.db
            {
                let pool = pool.clone();
                let client = crate::egress::ControlledHttpClient::internet()
                    .expect("static controlled HTTP client limits are valid");
                let server_id_owned = server_id.to_string();
                let channel_name_owned = channel_name.clone();
                let channel_id_owned = channel_id.clone();
                // Collect senders for channel members before spawning
                let member_sessions: Vec<Arc<UserSession>> =
                    if let Some(channel) = self.channels.get(&channel_id) {
                        channel
                            .members
                            .iter()
                            .filter_map(|sid| self.sessions.get(sid).map(|s| s.clone()))
                            .collect()
                    } else {
                        vec![]
                    };
                tokio::spawn(async move {
                    let mut embeds = Vec::new();
                    for url in urls {
                        // Check cache first
                        if let Ok(Some(cached)) =
                            crate::db::queries::embeds::get_cached_embed(&pool, &url).await
                        {
                            embeds.push(crate::engine::events::EmbedInfo {
                                url: cached.url,
                                title: cached.title,
                                description: cached.description,
                                image_url: cached.image_url,
                                site_name: cached.site_name,
                            });
                            continue;
                        }
                        // Unfurl
                        if let Some(info) = crate::engine::embeds::unfurl_url(&client, &url).await {
                            let _ = crate::db::queries::embeds::upsert_embed(
                                &pool,
                                &info.url,
                                info.title.as_deref(),
                                info.description.as_deref(),
                                info.image_url.as_deref(),
                                info.site_name.as_deref(),
                            )
                            .await;
                            embeds.push(info);
                        }
                    }
                    if !embeds.is_empty() {
                        let embed_event = ChatEvent::MessageEmbed {
                            message_id: msg_id.clone(),
                            server_id: server_id_owned,
                            channel: channel_name_owned,
                            embeds,
                        };
                        for session in &member_sessions {
                            let _ = session.send_guarded(
                                embed_event.clone(),
                                Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                                    channel_id_owned.clone(),
                                ])),
                            );
                        }
                    }
                });
            }
        } else {
            // DM
            let target_session_id = self
                .nick_to_session
                .get(&crate::auth::authority::rfc1459_casefold(target))
                .ok_or(format!("No such user: {target}"))?;

            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let id = msg_id.to_string();
                let sender_uid = session
                    .user_id
                    .clone()
                    .unwrap_or_else(|| session_id.to_string());
                let nick = session.nickname.clone();
                let target_uid = self
                    .sessions
                    .get(target_session_id.value())
                    .and_then(|s| s.user_id.clone())
                    .unwrap_or_else(|| target_session_id.value().to_string());
                let msg = content.to_string();
                tokio::spawn(async move {
                    if let Err(e) = crate::db::queries::messages::insert_dm(
                        &pool,
                        &id,
                        &sender_uid,
                        &nick,
                        &target_uid,
                        &msg,
                    )
                    .await
                    {
                        error!(error = %e, "failed to persist DM");
                    }
                });
            }

            let target_user_id = self
                .sessions
                .get(target_session_id.value())
                .and_then(|target_session| target_session.user_id.clone());
            if let Some(target_user_id) = target_user_id {
                if let Some(connections) = self.user_connections.get(&target_user_id) {
                    for connection_id in connections.iter() {
                        if let Some(target_session) = self.sessions.get(connection_id) {
                            let _ = target_session.send_guarded(
                                event.clone(),
                                Some(crate::engine::user_session::DeliveryGuard::ActorCurrent),
                            );
                        }
                    }
                }
            } else if let Some(target_session) = self.sessions.get(target_session_id.value()) {
                let _ = target_session.send_guarded(
                    event,
                    Some(crate::engine::user_session::DeliveryGuard::ActorCurrent),
                );
            }

            // Send MessageAck back to the DM sender
            if let Some(sender_session) = self.sessions.get(&session_id) {
                let _ = sender_session.send(ChatEvent::MessageAck {
                    id: msg_id.clone(),
                    server_id: String::new(),
                    channel: target.to_string(),
                    conversation_id: None,
                    request_id: nonce.unwrap_or_default().to_owned(),
                    client_message_id: nonce.unwrap_or_default().to_owned(),
                    sequence: String::new(),
                    persisted_at: Utc::now().to_rfc3339(),
                    replayed: false,
                    nonce: nonce.map(|s| s.to_string()),
                });
            }
        }

        Ok(())
    }
}
