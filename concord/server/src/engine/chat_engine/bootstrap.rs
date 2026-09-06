use super::{
    ChannelState, ChatEngine, DEFAULT_ADMIN, DEFAULT_EVERYONE, DEFAULT_MODERATOR, Permissions,
    ServerState, Uuid, info, warn,
};

impl ChatEngine {
    /// Load servers from the database into memory on startup.
    pub async fn load_servers_from_db(&self) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Ok(());
        };

        let rows = crate::db::queries::servers::list_all_servers(pool)
            .await
            .map_err(|e| format!("Failed to load servers: {e}"))?;

        for row in rows {
            let mut state =
                ServerState::new(row.id.clone(), row.name, row.owner_id.clone(), row.icon_url);

            let members = crate::db::queries::servers::get_server_members(pool, &row.id)
                .await
                .map_err(|e| format!("Failed to load server members: {e}"))?;
            for m in members {
                state.member_user_ids.insert(m.user_id);
            }

            // Bootstrap default roles for servers that don't have any
            if !crate::db::queries::roles::server_has_roles(pool, &row.id)
                .await
                .unwrap_or(true)
            {
                info!(server_id = %row.id, "bootstrapping default roles for existing server");
                let default_roles = [
                    ("@everyone", None, 0, DEFAULT_EVERYONE.bits() as i64, true),
                    ("Moderator", None, 1, DEFAULT_MODERATOR.bits() as i64, false),
                    ("Admin", None, 2, DEFAULT_ADMIN.bits() as i64, false),
                    ("Owner", None, 3, Permissions::all().bits() as i64, false),
                ];
                let mut owner_role_id = None;
                for (role_name, color, position, perms, is_default) in &default_roles {
                    let role_id = Uuid::new_v4().to_string();
                    let params = crate::db::queries::roles::CreateRoleParams {
                        id: &role_id,
                        server_id: &row.id,
                        name: role_name,
                        color: *color,
                        icon_url: None,
                        position: *position,
                        permissions: *perms,
                        is_default: *is_default,
                    };
                    if let Err(e) = crate::db::queries::roles::create_role(pool, &params).await {
                        warn!(error = %e, role = role_name, "failed to create default role on load");
                    }
                    if *role_name == "Owner" {
                        owner_role_id = Some(role_id);
                    }
                }
                // Assign Owner role to the server owner
                if let Some(role_id) = owner_role_id
                    && let Err(e) = crate::db::queries::roles::assign_role(
                        pool,
                        &row.id,
                        &row.owner_id,
                        &role_id,
                    )
                    .await
                {
                    warn!(error = %e, "failed to assign Owner role on load");
                }
            }

            self.servers.insert(row.id, state);
        }
        let aliases = sqlx::query_as::<_, (String, String)>(
            "SELECT alias,server_id FROM server_aliases ORDER BY is_canonical DESC,created_at",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to load server aliases: {e}"))?;
        for (alias, server_id) in aliases {
            self.server_alias_index
                .insert(alias.to_lowercase(), server_id.clone());
            self.server_aliases.entry(server_id).or_insert(alias);
        }

        info!(count = self.servers.len(), "loaded servers from database");
        Ok(())
    }
    /// Load channels from the database into memory on startup.
    pub async fn load_channels_from_db(&self) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Ok(());
        };

        // Collect server IDs first to avoid holding a read lock on self.servers
        // while later acquiring a write lock via get_mut (DashMap deadlock).
        let server_ids: Vec<String> = self.servers.iter().map(|s| s.id.clone()).collect();

        for server_id in &server_ids {
            let rows = crate::db::queries::channels::list_channels(pool, server_id)
                .await
                .map_err(|e| format!("Failed to load channels: {e}"))?;

            for row in rows {
                let mut ch =
                    ChannelState::new(row.id.clone(), row.server_id.clone(), row.name.clone());
                ch.topic = row.topic;
                ch.topic_set_by = row.topic_set_by;
                ch.category_id = row.category_id;
                ch.position = row.position;
                ch.is_private = row.is_private != 0;
                ch.channel_type = row.channel_type;
                ch.thread_parent_message_id = row.thread_parent_message_id;
                ch.thread_creator_user_id = row.thread_creator_user_id;
                ch.auto_archive_minutes = row.thread_auto_archive_minutes;
                ch.archived = row.archived != 0;
                ch.thread_state_version = row.thread_state_version;
                ch.thread_tags_version = row.thread_tags_version;
                if matches!(ch.channel_type.as_str(), "public_thread" | "private_thread") {
                    ch.thread_tag_ids = sqlx::query_scalar(
                        "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
                    )
                    .bind(&row.id)
                    .fetch_all(pool)
                    .await
                    .map_err(|error| format!("Failed to load thread tags: {error}"))?;
                }
                ch.slowmode_seconds = row.slowmode_seconds;
                ch.is_nsfw = row.is_nsfw != 0;

                self.channel_name_index
                    .insert((row.server_id.clone(), row.name), row.id.clone());

                if let Some(mut srv) = self.servers.get_mut(&row.server_id) {
                    srv.channel_ids.insert(row.id.clone());
                }

                self.channels.insert(row.id, ch);
            }
        }

        info!(count = self.channels.len(), "loaded channels from database");
        Ok(())
    }
}
