use super::{
    Actor, ChannelId, OrganizationError, OrganizationService, Permissions, ProvisionedServer,
    ServerId, ServerInfo, ServerMemberSummary,
};

impl OrganizationService {
    pub async fn provision_server(
        &self,
        actor: &Actor,
        name: &str,
        icon_url: Option<&str>,
        server_id: &ServerId,
        channel_id: &ChannelId,
        server_alias: &str,
    ) -> Result<ProvisionedServer, OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        crate::engine::validation::validate_server_name(name)
            .map_err(|_| OrganizationError::InvalidInput("invalid server name"))?;
        if icon_url.is_some_and(|value| value.len() > 2_000 || value.chars().any(char::is_control))
        {
            return Err(OrganizationError::InvalidInput("invalid server icon"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let owner_user_id = actor.user_id().as_str();
        let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM servers WHERE owner_id=?")
            .bind(owner_user_id)
            .fetch_one(&mut *tx)
            .await?;
        if owned >= 100 {
            return Err(OrganizationError::InvalidInput("server limit reached"));
        }
        sqlx::query("INSERT INTO servers(id,name,owner_id,icon_url) VALUES(?,?,?,?)")
            .bind(server_id)
            .bind(name)
            .bind(owner_user_id)
            .bind(icon_url)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,'owner')")
            .bind(server_id)
            .bind(owner_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO server_aliases(alias,server_id) VALUES(?,?)")
            .bind(server_alias)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO user_default_servers(user_id,server_id) VALUES(?,?)")
            .bind(owner_user_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;

        let roles = [
            (
                "@everyone",
                0,
                crate::engine::permissions::DEFAULT_EVERYONE.bits() as i64,
                true,
            ),
            (
                "Moderator",
                1,
                crate::engine::permissions::DEFAULT_MODERATOR.bits() as i64,
                false,
            ),
            (
                "Admin",
                2,
                crate::engine::permissions::DEFAULT_ADMIN.bits() as i64,
                false,
            ),
            ("Owner", 3, Permissions::all().bits() as i64, false),
        ];
        for (role_name, position, permissions, is_default) in roles {
            let role_id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO roles(id,server_id,name,position,permissions,is_default) \
                 VALUES(?,?,?,?,?,?)",
            )
            .bind(&role_id)
            .bind(server_id)
            .bind(role_name)
            .bind(position)
            .bind(permissions)
            .bind(i64::from(is_default))
            .execute(&mut *tx)
            .await?;
            if role_name == "Owner" {
                sqlx::query("INSERT INTO user_roles(server_id,user_id,role_id) VALUES(?,?,?)")
                    .bind(server_id)
                    .bind(owner_user_id)
                    .bind(role_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,is_default,position) \
             VALUES(?,?,'#general',1,0)",
        )
        .bind(channel_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES(?,'general',?)",
        )
        .bind(server_id)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ProvisionedServer {
            server_id: server_id.to_string(),
            channel_id: channel_id.to_string(),
            server_alias: server_alias.to_string(),
        })
    }

    pub async fn list_servers_for_actor(
        &self,
        actor: &Actor,
    ) -> Result<Vec<ServerInfo>, OrganizationError> {
        let mut tx = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let rows: Vec<(String, String, Option<String>, String, i64)> = sqlx::query_as(
            "SELECT s.id,s.name,s.icon_url,sm.role, \
             (SELECT count(*) FROM server_members members WHERE members.server_id=s.id) \
             FROM servers s JOIN server_members sm ON sm.server_id=s.id \
             WHERE sm.user_id=? ORDER BY s.name,s.id",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *tx)
        .await?;
        let mut servers = Vec::with_capacity(rows.len());
        for (id, name, icon_url, role, member_count) in rows {
            let permissions = self
                .authorization
                .server_actor_permissions_in(&mut tx, &self.auth, actor, &id)
                .await?;
            servers.push(ServerInfo {
                id,
                name,
                icon_url,
                member_count: member_count as usize,
                role: Some(role),
                my_permissions: permissions.bits() as i64,
            });
        }
        tx.commit().await?;
        Ok(servers)
    }

    pub async fn server_for_actor(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(ServerInfo, crate::engine::authorization::AuthorizationStamp), OrganizationError>
    {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        let permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await?;
        let row: (String, String, Option<String>, String, i64) = sqlx::query_as(
            "SELECT s.id,s.name,s.icon_url,sm.role, \
             (SELECT count(*) FROM server_members members WHERE members.server_id=s.id) \
             FROM servers s JOIN server_members sm ON sm.server_id=s.id AND sm.user_id=? \
             WHERE s.id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OrganizationError::Forbidden)?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((
            ServerInfo {
                id: row.0,
                name: row.1,
                icon_url: row.2,
                member_count: row.4 as usize,
                role: Some(row.3),
                my_permissions: permissions.bits() as i64,
            },
            stamp,
        ))
    }

    pub async fn server_members_for_actor(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<
        (
            Vec<ServerMemberSummary>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        OrganizationError,
    > {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await?;
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT user_id,role,joined_at FROM server_members WHERE server_id=? \
             ORDER BY joined_at,user_id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((
            rows.into_iter()
                .map(|(user_id, role, joined_at)| ServerMemberSummary {
                    user_id,
                    role,
                    joined_at,
                })
                .collect(),
            stamp,
        ))
    }

    pub async fn delete_owned_server(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let deleted = sqlx::query("DELETE FROM servers WHERE id=? AND owner_id=?")
            .bind(server_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }
}
