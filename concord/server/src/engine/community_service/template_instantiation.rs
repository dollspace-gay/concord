use super::{
    Actor, CommunityError, CommunityService, HashMap, Permissions, ServerId, TemplateConfig, Uuid,
};

impl CommunityService {
    pub async fn instantiate_template(
        &self,
        actor: &Actor,
        template_id: &str,
        server_name: &str,
    ) -> Result<ServerId, CommunityError> {
        crate::engine::validation::validate_server_name(server_name)
            .map_err(|_| CommunityError::InvalidInput("invalid server name"))?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.auth.validate_actor_in(&mut tx, actor).await?;
        let template: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT server_id,format_version,config FROM server_templates WHERE id=?",
        )
        .bind(template_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (source_server_id, format_version, config_json) =
            template.ok_or(CommunityError::Forbidden)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                &source_server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        if format_version != 1 {
            return Err(CommunityError::InvalidInput(
                "unsupported server template version",
            ));
        }
        let config: TemplateConfig = serde_json::from_str(&config_json)
            .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
        config.validate()?;
        if config.channels.iter().any(|channel| channel.is_private) {
            self.require_private_template_authority_in(&mut tx, actor, &source_server_id)
                .await?;
        }
        let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM servers WHERE owner_id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&mut *tx)
            .await?;
        if owned >= 100 {
            return Err(CommunityError::InvalidInput("server limit reached"));
        }

        let server_id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES(?,?,?)")
            .bind(&server_id)
            .bind(server_name)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,'owner')")
            .bind(&server_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        let alias = format!("s-{}", server_id.replace('-', ""));
        sqlx::query("INSERT INTO server_aliases(alias,server_id) VALUES(?,?)")
            .bind(alias)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO user_default_servers(user_id,server_id) VALUES(?,?)")
            .bind(actor.user_id().as_str())
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;

        let mut category_ids = HashMap::new();
        for category in config.categories {
            let new_id = Uuid::new_v4().to_string();
            category_ids.insert(category.id, new_id.clone());
            sqlx::query(
                "INSERT INTO channel_categories(id,server_id,name,position) VALUES(?,?,?,?)",
            )
            .bind(new_id)
            .bind(&server_id)
            .bind(category.name)
            .bind(category.position)
            .execute(&mut *tx)
            .await?;
        }
        let mut role_ids = HashMap::new();
        for role in config.roles {
            let new_id = Uuid::new_v4().to_string();
            role_ids.insert(role.id, new_id.clone());
            sqlx::query(
                "INSERT INTO roles(id,server_id,name,color,position,permissions,is_default) \
                 VALUES(?,?,?,?,?,?,?)",
            )
            .bind(new_id)
            .bind(&server_id)
            .bind(role.name)
            .bind(role.color)
            .bind(role.position)
            .bind(role.permissions)
            .bind(i64::from(role.is_default))
            .execute(&mut *tx)
            .await?;
        }
        for channel in config.channels {
            let category_id = channel
                .category_id
                .as_ref()
                .and_then(|id| category_ids.get(id));
            let channel_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO channels( \
                    id,server_id,name,topic,category_id,position,is_private,channel_type, \
                    slowmode_seconds,is_nsfw,is_announcement,is_default \
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&channel_id)
            .bind(&server_id)
            .bind(channel.name)
            .bind(channel.topic)
            .bind(category_id)
            .bind(channel.position)
            .bind(i64::from(channel.is_private))
            .bind(channel.channel_type)
            .bind(channel.slowmode_seconds)
            .bind(i64::from(channel.is_nsfw))
            .bind(i64::from(channel.is_announcement))
            .bind(i64::from(channel.is_default))
            .execute(&mut *tx)
            .await?;
            for alias in channel.aliases {
                sqlx::query(
                    "INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES(?,?,?)",
                )
                .bind(&server_id)
                .bind(alias)
                .bind(&channel_id)
                .execute(&mut *tx)
                .await?;
            }
            for rule in channel.role_overrides {
                let target_role_id = role_ids
                    .get(&rule.role_id)
                    .ok_or(CommunityError::InvalidInput("invalid server template"))?;
                sqlx::query(
                    "INSERT INTO channel_permission_overrides( \
                        id,channel_id,target_type,target_id,allow_bits,deny_bits \
                     ) VALUES(?,?,'role',?,?,?)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(&channel_id)
                .bind(target_role_id)
                .bind(rule.allow_bits)
                .bind(rule.deny_bits)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query("UPDATE server_templates SET use_count=use_count+1 WHERE id=?")
            .bind(template_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(ServerId::parse(server_id).expect("UUID server IDs satisfy the resource ID boundary"))
    }

    pub async fn delete_template(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        template_id: &str,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
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
        let deleted = sqlx::query("DELETE FROM server_templates WHERE id=? AND server_id=?")
            .bind(template_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut tx,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "server_template_delete",
                target_type: Some("server_template"),
                target_id: Some(template_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
