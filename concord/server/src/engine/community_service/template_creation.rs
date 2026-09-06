use super::{
    Actor, AuthorizationStamp, CommunityError, CommunityService, CreatedTemplate, Permissions, Row,
    ServerId, ServerTemplateRow, SqliteConnection, TemplateCategory, TemplateChannel,
    TemplateConfig, TemplateRole, TemplateRoleOverride, Uuid,
};

impl CommunityService {
    pub(super) async fn require_private_template_authority_in(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(), CommunityError> {
        let privileged: i64 = sqlx::query_scalar(
            "SELECT EXISTS( \
                SELECT 1 FROM servers s \
                JOIN server_members sm ON sm.server_id=s.id AND sm.user_id=? \
                WHERE s.id=? AND (s.owner_id=? OR sm.role IN ('owner','admin') OR EXISTS( \
                    SELECT 1 FROM user_roles ur JOIN roles r ON r.id=ur.role_id \
                    WHERE ur.server_id=s.id AND ur.user_id=sm.user_id \
                      AND (r.permissions & ?) != 0 \
                )) \
            )",
        )
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .bind(Permissions::ADMINISTRATOR.bits() as i64)
        .fetch_one(connection)
        .await?;
        if privileged == 0 {
            return Err(CommunityError::Forbidden);
        }
        Ok(())
    }

    pub async fn create_template(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        name: &str,
        description: Option<&str>,
    ) -> Result<CreatedTemplate, CommunityError> {
        let server_id = server_id.as_str();
        crate::engine::validation::validate_server_name(name)
            .map_err(|_| CommunityError::InvalidInput("invalid template name"))?;
        if description
            .is_some_and(|value| value.len() > 1_000 || value.chars().any(char::is_control))
        {
            return Err(CommunityError::InvalidInput("invalid template description"));
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

        let has_private_channels: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE server_id=? \
             AND parent_channel_id IS NULL AND channel_type IN ('text','forum') \
             AND is_private=1)",
        )
        .bind(server_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_private_channels != 0 {
            self.require_private_template_authority_in(&mut tx, actor, server_id)
                .await?;
        }

        let category_rows = sqlx::query(
            "SELECT id,name,position FROM channel_categories WHERE server_id=? ORDER BY position,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let categories = category_rows
            .into_iter()
            .map(|row| TemplateCategory {
                id: row.get(0),
                name: row.get(1),
                position: row.get(2),
            })
            .collect();
        let role_rows = sqlx::query(
            "SELECT id,name,color,position,permissions,is_default FROM roles \
             WHERE server_id=? ORDER BY position,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let roles = role_rows
            .into_iter()
            .map(|row| TemplateRole {
                id: row.get(0),
                name: row.get(1),
                color: row.get(2),
                position: row.get(3),
                permissions: row.get(4),
                is_default: row.get::<i64, _>(5) != 0,
            })
            .collect();
        let channel_rows = sqlx::query(
            "SELECT id,name,topic,category_id,position,is_private,channel_type, \
                    slowmode_seconds,is_nsfw,is_announcement,is_default \
             FROM channels WHERE server_id=? AND parent_channel_id IS NULL \
               AND channel_type IN ('text','forum') \
             ORDER BY position,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut channels = Vec::with_capacity(channel_rows.len());
        for row in channel_rows {
            let channel_id: String = row.get(0);
            let aliases = sqlx::query_scalar(
                "SELECT alias FROM channel_aliases WHERE server_id=? AND channel_id=? \
                 ORDER BY alias COLLATE NOCASE",
            )
            .bind(server_id)
            .bind(&channel_id)
            .fetch_all(&mut *tx)
            .await?;
            let override_rows = sqlx::query(
                "SELECT target_id,allow_bits,deny_bits FROM channel_permission_overrides \
                 WHERE channel_id=? AND target_type='role' ORDER BY target_id",
            )
            .bind(&channel_id)
            .fetch_all(&mut *tx)
            .await?;
            let role_overrides = override_rows
                .into_iter()
                .map(|override_row| TemplateRoleOverride {
                    role_id: override_row.get(0),
                    allow_bits: override_row.get(1),
                    deny_bits: override_row.get(2),
                })
                .collect();
            channels.push(TemplateChannel {
                id: channel_id,
                name: row.get(1),
                topic: row.get(2),
                category_id: row.get(3),
                position: row.get(4),
                is_private: row.get::<i64, _>(5) != 0,
                channel_type: row.get(6),
                slowmode_seconds: row.get(7),
                is_nsfw: row.get::<i64, _>(8) != 0,
                is_announcement: row.get::<i64, _>(9) != 0,
                is_default: row.get::<i64, _>(10) != 0,
                aliases,
                role_overrides,
            });
        }
        let config = TemplateConfig {
            format_version: 1,
            channels,
            categories,
            roles,
        };
        config.validate()?;
        let template_id = Uuid::new_v4().to_string();
        let created_at: String = sqlx::query_scalar(
            "INSERT INTO server_templates( \
                id,name,description,server_id,created_by,config,format_version \
             ) VALUES(?,?,?,?,?,?,1) RETURNING created_at",
        )
        .bind(&template_id)
        .bind(name)
        .bind(description)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .bind(
            serde_json::to_string(&config)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?,
        )
        .fetch_one(&mut *tx)
        .await?;
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut tx,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "server_template_create",
                target_type: Some("server_template"),
                target_id: Some(&template_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(CreatedTemplate {
            id: template_id,
            created_at,
        })
    }

    pub async fn list_templates(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(Vec<ServerTemplateRow>, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let rows = sqlx::query_as::<_, ServerTemplateRow>(
            "SELECT * FROM server_templates WHERE server_id=? ORDER BY created_at DESC,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((rows, stamp))
    }
}
