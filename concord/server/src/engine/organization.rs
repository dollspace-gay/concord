use sqlx::{Row, SqlitePool};

use crate::auth::authority::{Actor, AuthService};
use crate::db::models::RoleRow;

use super::authorization::{AuthorizationService, ChannelAction};
use super::events::{CategoryInfo, ChannelPermissionOverrideInfo, ChannelPositionInfo, ServerInfo};
use super::ids::{ChannelId, ServerId};
use super::permissions::Permissions;

pub struct ProvisionedServer {
    pub server_id: String,
    pub channel_id: String,
    pub server_alias: String,
}

pub struct ServerMemberSummary {
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}

pub struct CreateChannel<'a> {
    pub server_id: &'a ServerId,
    pub channel_id: &'a ChannelId,
    pub name: &'a str,
    pub category_id: Option<&'a str>,
    pub is_private: bool,
    pub channel_type: &'a str,
}

pub struct ChannelOverrideUpdate<'a> {
    pub server_id: &'a ServerId,
    pub channel_id: &'a ChannelId,
    pub target_type: &'a str,
    pub target_id: &'a str,
    pub allow_bits: i64,
    pub deny_bits: i64,
}

#[derive(Debug)]
pub enum OrganizationError {
    Authentication(crate::auth::authority::AuthError),
    Forbidden,
    InvalidInput(&'static str),
    Admission(super::write_admission::WriteAdmissionError),
    Database(sqlx::Error),
}

impl OrganizationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ) => "DEPENDENCY_UNAVAILABLE",
            Self::Authentication(_) => "UNAUTHENTICATED",
            Self::Forbidden => "FORBIDDEN",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Admission(_) | Self::Database(_) => "DEPENDENCY_UNAVAILABLE",
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ) => "dependency unavailable",
            Self::Authentication(_) => "authentication required",
            Self::Forbidden => "resource unavailable",
            Self::InvalidInput(message) => message,
            Self::Admission(_) | Self::Database(_) => "dependency unavailable",
        }
    }
}

impl std::fmt::Display for OrganizationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.safe_message())
    }
}

impl std::error::Error for OrganizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::Authentication(error) => Some(error),
            _ => None,
        }
    }
}

impl From<OrganizationError> for String {
    fn from(error: OrganizationError) -> Self {
        format!("{}: {}", error.code(), error.safe_message())
    }
}

impl From<sqlx::Error> for OrganizationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<super::write_admission::WriteAdmissionError> for OrganizationError {
    fn from(error: super::write_admission::WriteAdmissionError) -> Self {
        Self::Admission(error)
    }
}

impl From<super::authorization::AuthorizationError> for OrganizationError {
    fn from(error: super::authorization::AuthorizationError) -> Self {
        match error {
            super::authorization::AuthorizationError::Unavailable => Self::Forbidden,
            super::authorization::AuthorizationError::Database(error) => Self::Database(error),
            super::authorization::AuthorizationError::Authentication(error) => {
                Self::Authentication(error)
            }
        }
    }
}

#[derive(Clone)]
pub struct OrganizationService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

impl OrganizationService {
    const CHANNEL_OVERRIDE_PERMISSIONS: Permissions = Permissions::VIEW_CHANNELS
        .union(Permissions::MANAGE_CHANNELS)
        .union(Permissions::SEND_MESSAGES)
        .union(Permissions::EMBED_LINKS)
        .union(Permissions::ATTACH_FILES)
        .union(Permissions::ADD_REACTIONS)
        .union(Permissions::MENTION_EVERYONE)
        .union(Permissions::MANAGE_MESSAGES)
        .union(Permissions::READ_MESSAGE_HISTORY)
        .union(Permissions::CONNECT)
        .union(Permissions::SPEAK)
        .union(Permissions::MUTE_MEMBERS)
        .union(Permissions::DEAFEN_MEMBERS)
        .union(Permissions::MOVE_MEMBERS);

    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self {
            pool: pool.clone(),
            authorization: AuthorizationService::new(pool.clone()),
            auth,
            writes,
        }
    }

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
                super::permissions::DEFAULT_EVERYONE.bits() as i64,
                true,
            ),
            (
                "Moderator",
                1,
                super::permissions::DEFAULT_MODERATOR.bits() as i64,
                false,
            ),
            (
                "Admin",
                2,
                super::permissions::DEFAULT_ADMIN.bits() as i64,
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
    ) -> Result<(ServerInfo, super::authorization::AuthorizationStamp), OrganizationError> {
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
            super::authorization::AuthorizationStamp,
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

    pub async fn join_server(
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
        let admitted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM servers s WHERE s.id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id=?))",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        if !admitted {
            return Err(OrganizationError::Forbidden);
        }
        sqlx::query(
            "INSERT OR IGNORE INTO server_members(server_id,user_id,role) VALUES(?,?,'member')",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn leave_server(
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
        let removed = sqlx::query(
            "DELETE FROM server_members WHERE server_id=? AND user_id=? AND EXISTS(SELECT 1 FROM servers s WHERE s.id=? AND s.owner_id<>?)",
        )
        .bind(server_id).bind(actor.user_id().as_str()).bind(server_id).bind(actor.user_id().as_str())
        .execute(&mut *tx).await?;
        if removed.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_server_nickname(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        nickname: Option<&str>,
    ) -> Result<Option<String>, OrganizationError> {
        let server_id = server_id.as_str();
        if let Some(nickname) = nickname {
            crate::engine::validation::validate_display_name(nickname)
                .map_err(|_| OrganizationError::InvalidInput("invalid server nickname"))?;
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        let updated =
            sqlx::query("UPDATE server_members SET nickname=? WHERE server_id=? AND user_id=?")
                .bind(nickname.map(str::trim))
                .bind(server_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        let avatar = sqlx::query_scalar(
            "SELECT avatar_url FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(avatar)
    }

    pub async fn update_server(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(String, Option<String>), OrganizationError> {
        let server_id = server_id.as_str();
        if let Some(name) = name {
            crate::engine::validation::validate_server_name(name)
                .map_err(|_| OrganizationError::InvalidInput("invalid server name"))?;
        }
        if icon_url.is_some_and(|url| url.len() > 2_000 || url.chars().any(char::is_control)) {
            return Err(OrganizationError::InvalidInput("invalid server icon"));
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
            .await
            .map_err(OrganizationError::from)?;
        let current: (String, Option<String>) =
            sqlx::query_as("SELECT name,icon_url FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?;
        let next_name = name.unwrap_or(&current.0).to_owned();
        let next_icon = icon_url.map(str::to_owned).or(current.1);
        sqlx::query("UPDATE servers SET name=?,icon_url=?,updated_at=datetime('now') WHERE id=?")
            .bind(&next_name)
            .bind(&next_icon)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok((next_name, next_icon))
    }

    pub async fn update_emoji_settings(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        allow_external: bool,
        shareable: bool,
    ) -> Result<(), OrganizationError> {
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
        sqlx::query("UPDATE servers SET allow_external_emoji=?,shareable_emoji=?,updated_at=datetime('now') WHERE id=?")
            .bind(i64::from(allow_external)).bind(i64::from(shareable)).bind(server_id)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_member_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        role: &str,
    ) -> Result<(), OrganizationError> {
        use super::permissions::ServerRole;

        let desired = match role {
            "admin" => ServerRole::Admin,
            "moderator" => ServerRole::Moderator,
            "member" => ServerRole::Member,
            _ => return Err(OrganizationError::InvalidInput("invalid server role")),
        };
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_ROLES,
            )
            .await?;
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT caller.role,target.role FROM server_members caller \
             JOIN server_members target ON target.server_id=caller.server_id \
             JOIN servers s ON s.id=caller.server_id \
             WHERE caller.server_id=? AND caller.user_id=? AND target.user_id=? \
               AND target.user_id<>s.owner_id",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .bind(target_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OrganizationError::Forbidden)?;
        let caller = ServerRole::parse(&row.0);
        let current = ServerRole::parse(&row.1);
        if !caller.can_manage_roles(&current) || !caller.can_manage_roles(&desired) {
            return Err(OrganizationError::Forbidden);
        }
        let updated =
            sqlx::query("UPDATE server_members SET role=? WHERE server_id=? AND user_id=?")
                .bind(desired.as_str())
                .bind(server_id)
                .bind(target_user_id)
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_member_avatar(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        avatar_url: Option<&str>,
    ) -> Result<(), OrganizationError> {
        if avatar_url.is_some_and(|url| url.len() > 2_000 || url.chars().any(char::is_control)) {
            return Err(OrganizationError::InvalidInput("invalid server avatar"));
        }
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
        let updated =
            sqlx::query("UPDATE server_members SET avatar_url=? WHERE server_id=? AND user_id=?")
                .bind(avatar_url)
                .bind(server_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn admin_delete_server(
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
        let is_admin: bool = sqlx::query_scalar(
            "SELECT is_system_admin=1 FROM users WHERE id=? AND disabled_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        if !is_admin {
            return Err(OrganizationError::Forbidden);
        }
        let deleted = sqlx::query("DELETE FROM servers WHERE id=?")
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_servers_as_admin(
        &self,
        actor: &Actor,
    ) -> Result<Vec<ServerInfo>, OrganizationError> {
        let mut tx = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let is_admin: bool = sqlx::query_scalar(
            "SELECT is_system_admin=1 FROM users WHERE id=? AND disabled_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        if !is_admin {
            return Err(OrganizationError::Forbidden);
        }
        let rows: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
            "SELECT s.id,s.name,s.icon_url,count(sm.user_id) FROM servers s \
             LEFT JOIN server_members sm ON sm.server_id=s.id \
             GROUP BY s.id,s.name,s.icon_url ORDER BY s.name,s.id",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|(id, name, icon_url, member_count)| ServerInfo {
                id,
                name,
                icon_url,
                member_count: member_count as usize,
                role: None,
                my_permissions: 0,
            })
            .collect())
    }

    pub async fn set_system_admin(
        &self,
        actor: &Actor,
        target_user_id: &str,
        is_admin: bool,
    ) -> Result<(), OrganizationError> {
        let (_permit, mut tx) = self.writes.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let is_actor_admin: bool = sqlx::query_scalar(
            "SELECT is_system_admin=1 FROM users WHERE id=? AND disabled_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        if !is_actor_admin {
            return Err(OrganizationError::Forbidden);
        }
        let updated = sqlx::query("UPDATE users SET is_system_admin=? WHERE id=?")
            .bind(i64::from(is_admin))
            .bind(target_user_id)
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_category(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        category_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, OrganizationError> {
        let server_id = server_id.as_str();
        crate::engine::validation::validate_server_name(name)
            .map_err(|_| OrganizationError::InvalidInput("invalid category name"))?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await
            .map_err(OrganizationError::from)?;
        let position: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position),-1)+1 FROM channel_categories WHERE server_id=?",
        )
        .bind(server_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        sqlx::query("INSERT INTO channel_categories(id,server_id,name,position) VALUES(?,?,?,?)")
            .bind(category_id)
            .bind(server_id)
            .bind(name)
            .bind(position)
            .execute(&mut *tx)
            .await
            .map_err(OrganizationError::from)?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(CategoryInfo {
            id: category_id.into(),
            server_id: server_id.into(),
            name: name.into(),
            position,
        })
    }

    pub async fn update_category(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        category_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, OrganizationError> {
        let server_id = server_id.as_str();
        crate::engine::validation::validate_server_name(name)
            .map_err(|_| OrganizationError::InvalidInput("invalid category name"))?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await
            .map_err(OrganizationError::from)?;
        let row = sqlx::query(
            "UPDATE channel_categories SET name=? WHERE id=? AND server_id=? \
             RETURNING id,server_id,name,position",
        )
        .bind(name)
        .bind(category_id)
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(OrganizationError::from)?
        .ok_or(OrganizationError::Forbidden)?;
        let category = CategoryInfo {
            id: row.get(0),
            server_id: row.get(1),
            name: row.get(2),
            position: row.get(3),
        };
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(category)
    }

    pub async fn delete_category(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        category_id: &str,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await
            .map_err(OrganizationError::from)?;
        let result = sqlx::query("DELETE FROM channel_categories WHERE id=? AND server_id=?")
            .bind(category_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await
            .map_err(OrganizationError::from)?;
        if result.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(())
    }

    pub async fn reorder_channels(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        updates: &[ChannelPositionInfo],
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await
            .map_err(OrganizationError::from)?;
        let channel_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM channels WHERE server_id=? AND parent_channel_id IS NULL \
             AND channel_type IN ('text','forum') ORDER BY id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        if channel_ids.len() > 500 || updates.len() != channel_ids.len() {
            return Err(OrganizationError::InvalidInput(
                "incomplete channel reorder",
            ));
        }
        let expected: std::collections::HashSet<&str> =
            channel_ids.iter().map(String::as_str).collect();
        let supplied: std::collections::HashSet<&str> =
            updates.iter().map(|update| update.id.as_str()).collect();
        let positions: std::collections::HashSet<i32> =
            updates.iter().map(|update| update.position).collect();
        if supplied != expected
            || positions.len() != updates.len()
            || !positions.iter().all(|position| {
                *position >= 0 && usize::try_from(*position).is_ok_and(|p| p < updates.len())
            })
        {
            return Err(OrganizationError::InvalidInput("invalid channel reorder"));
        }
        for update in updates {
            let category_matches = match update.category_id.as_deref() {
                Some(category_id) => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM channel_categories WHERE id=? AND server_id=?)",
                )
                .bind(category_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?,
                None => true,
            };
            if !category_matches {
                return Err(OrganizationError::Forbidden);
            }
            sqlx::query("UPDATE channels SET position=?,category_id=? WHERE id=? AND server_id=?")
                .bind(update.position)
                .bind(update.category_id.as_deref())
                .bind(&update.id)
                .bind(server_id)
                .execute(&mut *tx)
                .await
                .map_err(OrganizationError::from)?;
        }
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(())
    }

    pub async fn create_channel(
        &self,
        actor: &Actor,
        command: CreateChannel<'_>,
    ) -> Result<(), OrganizationError> {
        let CreateChannel {
            server_id,
            channel_id,
            name,
            category_id,
            is_private,
            channel_type,
        } = command;
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !matches!(channel_type, "text" | "forum") {
            return Err(OrganizationError::InvalidInput("invalid channel type"));
        }
        crate::engine::validation::validate_channel_name(name)
            .map_err(|_| OrganizationError::InvalidInput("invalid channel name"))?;
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        if let Some(category_id) = category_id {
            let valid: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM channel_categories WHERE id=? AND server_id=?)",
            )
            .bind(category_id)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await?;
            if !valid {
                return Err(OrganizationError::Forbidden);
            }
        }
        let position: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position),-1)+1 FROM channels WHERE server_id=? \
             AND parent_channel_id IS NULL AND channel_type IN ('text','forum')",
        )
        .bind(server_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,category_id,position,is_private,channel_type) \
             VALUES(?,?,?,?,?,?,?)",
        )
        .bind(channel_id)
        .bind(server_id)
        .bind(name)
        .bind(category_id)
        .bind(position)
        .bind(i64::from(is_private))
        .bind(channel_type)
        .execute(&mut *tx)
        .await?;
        let alias = name.trim_start_matches('#').to_ascii_lowercase();
        sqlx::query("INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES(?,?,?)")
            .bind(server_id)
            .bind(alias)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        if is_private {
            sqlx::query(
                "INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) \
                 VALUES(?,'user',?)",
            )
            .bind(channel_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_channel(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let is_default: i64 = sqlx::query_scalar(
            "SELECT is_default FROM channels WHERE id=? AND server_id=? \
             AND parent_channel_id IS NULL AND channel_type IN ('text','forum')",
        )
        .bind(channel_id)
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OrganizationError::Forbidden)?;
        if is_default != 0 {
            let replacement: Option<String> = sqlx::query_scalar(
                "SELECT id FROM channels WHERE server_id=? AND id!=? \
                 AND parent_channel_id IS NULL AND channel_type IN ('text','forum') \
                 ORDER BY position,id LIMIT 1",
            )
            .bind(server_id)
            .bind(channel_id)
            .fetch_optional(&mut *tx)
            .await?;
            let replacement = replacement.ok_or(OrganizationError::InvalidInput(
                "cannot delete the only default channel",
            ))?;
            sqlx::query("UPDATE channels SET is_default=1 WHERE id=? AND server_id=?")
                .bind(replacement)
                .bind(server_id)
                .execute(&mut *tx)
                .await?;
        }
        let deleted = sqlx::query("DELETE FROM channels WHERE id=? AND server_id=?")
            .bind(channel_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_channel_overrides(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
    ) -> Result<Vec<ChannelPermissionOverrideInfo>, OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let channel_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)")
                .bind(channel_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?;
        if !channel_exists {
            return Err(OrganizationError::Forbidden);
        }
        let rows: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT id,channel_id,target_type,target_id,allow_bits,deny_bits \
             FROM channel_permission_overrides WHERE channel_id=? \
             ORDER BY target_type,target_id",
        )
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, channel_id, target_type, target_id, allow_bits, deny_bits)| {
                    ChannelPermissionOverrideInfo {
                        id,
                        channel_id,
                        target_type,
                        target_id,
                        allow_bits,
                        deny_bits,
                    }
                },
            )
            .collect())
    }

    pub async fn set_channel_override(
        &self,
        actor: &Actor,
        update: ChannelOverrideUpdate<'_>,
    ) -> Result<(), OrganizationError> {
        let ChannelOverrideUpdate {
            server_id,
            channel_id,
            target_type,
            target_id,
            allow_bits,
            deny_bits,
        } = update;
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !matches!(target_type, "role" | "user") || target_id.trim().is_empty() {
            return Err(OrganizationError::InvalidInput("invalid override target"));
        }
        let allow = Permissions::from_bits(allow_bits as u64).ok_or(
            OrganizationError::InvalidInput("invalid override permissions"),
        )?;
        let deny = Permissions::from_bits(deny_bits as u64).ok_or(
            OrganizationError::InvalidInput("invalid override permissions"),
        )?;
        if allow.intersects(deny)
            || !(allow | deny)
                .difference(Self::CHANNEL_OVERRIDE_PERMISSIONS)
                .is_empty()
            || (allow | deny).is_empty()
        {
            return Err(OrganizationError::InvalidInput(
                "invalid override permissions",
            ));
        }

        let (_permit, mut tx) = self.writes.begin().await?;
        let actor_permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await?;
        if !actor_permissions.contains(Permissions::MANAGE_CHANNELS)
            || !actor_permissions.contains(allow)
        {
            return Err(OrganizationError::Forbidden);
        }
        let channel_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)")
                .bind(channel_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?;
        let target_exists: bool = match target_type {
            "role" => {
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roles WHERE id=? AND server_id=?)")
                    .bind(target_id)
                    .bind(server_id)
                    .fetch_one(&mut *tx)
                    .await?
            }
            "user" => {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE user_id=? AND server_id=?)",
                )
                .bind(target_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?
            }
            _ => false,
        };
        if !channel_exists || !target_exists {
            return Err(OrganizationError::Forbidden);
        }
        sqlx::query(
            "INSERT INTO channel_permission_overrides( \
                 id,channel_id,target_type,target_id,allow_bits,deny_bits \
             ) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(channel_id,target_type,target_id) DO UPDATE SET \
                 allow_bits=excluded.allow_bits,deny_bits=excluded.deny_bits",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(channel_id)
        .bind(target_type)
        .bind(target_id)
        .bind(allow_bits)
        .bind(deny_bits)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_channel_override(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        target_type: &str,
        target_id: &str,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !matches!(target_type, "role" | "user") || target_id.trim().is_empty() {
            return Err(OrganizationError::InvalidInput("invalid override target"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let result = sqlx::query(
            "DELETE FROM channel_permission_overrides WHERE channel_id=? \
             AND target_type=? AND target_id=? \
             AND EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)",
        )
        .bind(channel_id)
        .bind(target_type)
        .bind(target_id)
        .bind(channel_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_topic(
        &self,
        actor: &Actor,
        channel_id: &ChannelId,
        topic: &str,
        set_by: &str,
    ) -> Result<(), OrganizationError> {
        let channel_id = channel_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .authorize_actor_in(
                &mut tx,
                &self.auth,
                actor,
                channel_id,
                ChannelAction::Manage,
            )
            .await
            .map_err(OrganizationError::from)?;
        sqlx::query(
            "UPDATE channels SET topic=?,topic_set_by=?,topic_set_at=datetime('now'),updated_at=datetime('now') WHERE id=?",
        )
        .bind(topic)
        .bind(set_by)
        .bind(channel_id)
        .execute(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(())
    }

    pub async fn list_roles(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<Vec<RoleRow>, OrganizationError> {
        let server_id = server_id.as_str();
        let mut connection = self.pool.acquire().await.map_err(OrganizationError::from)?;
        self.authorization
            .server_actor_permissions_in(&mut connection, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        sqlx::query_as("SELECT * FROM roles WHERE server_id=? ORDER BY position DESC")
            .bind(server_id)
            .fetch_all(&mut *connection)
            .await
            .map_err(OrganizationError::from)
    }

    pub async fn create_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permission_bits: i64,
    ) -> Result<RoleRow, OrganizationError> {
        let server_id = server_id.as_str();
        validate_role_fields(name, color)?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        let actor_permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        require_role_grant(actor_permissions, permission_bits)?;
        let owner: bool = sqlx::query_scalar("SELECT owner_id=? FROM servers WHERE id=?")
            .bind(actor.user_id().as_str())
            .bind(server_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(OrganizationError::from)?
            .ok_or(OrganizationError::Forbidden)?;
        let actor_highest =
            highest_role_position(&mut tx, server_id, actor.user_id().as_str(), owner).await?;
        let mut position: i32 =
            sqlx::query_scalar("SELECT COALESCE(MAX(position),0)+1 FROM roles WHERE server_id=?")
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?;
        if !owner {
            position = position.min(actor_highest.saturating_sub(1));
            if position <= 0 {
                return Err(OrganizationError::Forbidden);
            }
        }
        sqlx::query("INSERT INTO roles(id,server_id,name,color,position,permissions,is_default) VALUES(?,?,?,?,?,?,0)")
            .bind(role_id).bind(server_id).bind(name).bind(color).bind(position).bind(permission_bits)
            .execute(&mut *tx).await.map_err(OrganizationError::from)?;
        let role = sqlx::query_as("SELECT * FROM roles WHERE id=? AND server_id=?")
            .bind(role_id)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(OrganizationError::from)?;
        bump_role_projection_version(&mut tx, server_id).await?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(role)
    }

    pub async fn update_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permission_bits: i64,
    ) -> Result<RoleRow, OrganizationError> {
        let server_id = server_id.as_str();
        validate_role_fields(name, color)?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        let actor_permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        require_role_grant(actor_permissions, permission_bits)?;
        let existing: RoleRow = sqlx::query_as("SELECT * FROM roles WHERE id=? AND server_id=?")
            .bind(role_id)
            .bind(server_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(OrganizationError::from)?
            .ok_or(OrganizationError::Forbidden)?;
        let role = if existing.is_default != 0 {
            if name != existing.name || color != existing.color.as_deref() {
                return Err(OrganizationError::Forbidden);
            }
            sqlx::query_as("UPDATE roles SET permissions=? WHERE id=? AND server_id=? AND is_default=1 RETURNING *")
                .bind(permission_bits)
                .bind(role_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
        } else {
            require_managed_role(&mut tx, actor, server_id, role_id).await?;
            sqlx::query_as("UPDATE roles SET name=?,color=?,permissions=? WHERE id=? AND server_id=? AND is_default=0 RETURNING *")
                .bind(name)
                .bind(color)
                .bind(permission_bits)
                .bind(role_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
        };
        bump_role_projection_version(&mut tx, server_id).await?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(role)
    }

    pub async fn delete_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        role_id: &str,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_ROLES,
            )
            .await
            .map_err(OrganizationError::from)?;
        require_managed_role(&mut tx, actor, server_id, role_id).await?;
        sqlx::query(
            "DELETE FROM channel_permission_overrides \
             WHERE target_type='role' AND target_id=? AND channel_id IN ( \
                 SELECT id FROM channels WHERE server_id=? \
             )",
        )
        .bind(role_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
        let changed = sqlx::query("DELETE FROM roles WHERE id=? AND server_id=? AND is_default=0")
            .bind(role_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await
            .map_err(OrganizationError::from)?
            .rows_affected();
        if changed != 1 {
            return Err(OrganizationError::Forbidden);
        }
        bump_role_projection_version(&mut tx, server_id).await?;
        tx.commit().await.map_err(OrganizationError::from)
    }

    pub async fn set_member_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        role_id: &str,
        assign: bool,
    ) -> Result<Vec<String>, OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_ROLES,
            )
            .await
            .map_err(OrganizationError::from)?;
        require_managed_role(&mut tx, actor, server_id, role_id).await?;
        let target_member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
        )
        .bind(server_id)
        .bind(target_user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        if !target_member {
            return Err(OrganizationError::Forbidden);
        }
        let owner: bool = sqlx::query_scalar("SELECT owner_id=? FROM servers WHERE id=?")
            .bind(actor.user_id().as_str())
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(OrganizationError::from)?;
        if !owner {
            let actor_highest =
                highest_role_position(&mut tx, server_id, actor.user_id().as_str(), false).await?;
            let target_highest =
                highest_role_position(&mut tx, server_id, target_user_id, false).await?;
            if actor_highest <= target_highest {
                return Err(OrganizationError::Forbidden);
            }
        }
        let changed = if assign {
            sqlx::query("INSERT OR IGNORE INTO user_roles(server_id,user_id,role_id) VALUES(?,?,?)")
                .bind(server_id)
                .bind(target_user_id)
                .bind(role_id)
                .execute(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
                .rows_affected()
        } else {
            sqlx::query("DELETE FROM user_roles WHERE server_id=? AND user_id=? AND role_id=?")
                .bind(server_id)
                .bind(target_user_id)
                .bind(role_id)
                .execute(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
                .rows_affected()
        };
        if changed != 0 {
            bump_role_projection_version(&mut tx, server_id).await?;
        }
        let roles = sqlx::query_scalar(
            "SELECT role_id FROM user_roles WHERE server_id=? AND user_id=? ORDER BY role_id",
        )
        .bind(server_id)
        .bind(target_user_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(roles)
    }
}

async fn bump_role_projection_version(
    connection: &mut sqlx::SqliteConnection,
    server_id: &str,
) -> Result<(), OrganizationError> {
    sqlx::query("UPDATE servers SET role_projection_version=role_projection_version+1 WHERE id=?")
        .bind(server_id)
        .execute(connection)
        .await
        .map_err(OrganizationError::from)?;
    Ok(())
}

fn require_role_grant(actor_permissions: Permissions, bits: i64) -> Result<(), OrganizationError> {
    if bits < 0 {
        return Err(OrganizationError::InvalidInput("invalid role permissions"));
    }
    let requested = Permissions::from_bits(bits as u64).ok_or(OrganizationError::Forbidden)?;
    if !actor_permissions.contains(Permissions::MANAGE_ROLES)
        || !actor_permissions.contains(requested)
    {
        return Err(OrganizationError::Forbidden);
    }
    Ok(())
}

fn validate_role_fields(name: &str, color: Option<&str>) -> Result<(), OrganizationError> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.len() > 100
        || name.chars().any(char::is_control)
    {
        return Err(OrganizationError::InvalidInput("invalid role name"));
    }
    if color.is_some_and(|value| {
        value.len() != 7
            || !value.starts_with('#')
            || !value[1..]
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    }) {
        return Err(OrganizationError::InvalidInput("invalid role color"));
    }
    Ok(())
}

async fn highest_role_position(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    server_id: &str,
    user_id: &str,
    owner: bool,
) -> Result<i32, OrganizationError> {
    if owner {
        return Ok(i32::MAX);
    }
    sqlx::query_scalar("SELECT COALESCE(MAX(r.position),0) FROM roles r JOIN user_roles ur ON ur.role_id=r.id AND ur.server_id=r.server_id WHERE r.server_id=? AND ur.user_id=?")
        .bind(server_id).bind(user_id).fetch_one(&mut **tx).await.map_err(OrganizationError::from)
}

async fn require_managed_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &Actor,
    server_id: &str,
    role_id: &str,
) -> Result<(), OrganizationError> {
    let owner: bool = sqlx::query_scalar("SELECT owner_id=? FROM servers WHERE id=?")
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(OrganizationError::from)?
        .ok_or(OrganizationError::Forbidden)?;
    let role: Option<(i32, i32)> =
        sqlx::query_as("SELECT position,is_default FROM roles WHERE id=? AND server_id=?")
            .bind(role_id)
            .bind(server_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(OrganizationError::from)?;
    let (position, is_default) = role.ok_or(OrganizationError::Forbidden)?;
    if is_default != 0 {
        return Err(OrganizationError::Forbidden);
    }
    let actor_highest =
        highest_role_position(tx, server_id, actor.user_id().as_str(), owner).await?;
    if !owner && actor_highest <= position {
        return Err(OrganizationError::Forbidden);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_id(value: &str) -> ServerId {
        ServerId::from_stored(value).unwrap()
    }

    fn channel_id(value: &str) -> ChannelId {
        ChannelId::from_stored(value).unwrap()
    }

    async fn fixture() -> (SqlitePool, OrganizationService, Actor, String) {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
            .execute(&pool)
            .await
            .unwrap();
        crate::db::queries::servers::create_server(&pool, "server", "Server", "owner", None)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roles(id,server_id,name,position,permissions,is_default) VALUES('everyone','server','@everyone',0,0,1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('server','member','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let default_role: String =
            sqlx::query_scalar("SELECT id FROM roles WHERE server_id='server' AND is_default=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = OrganizationService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        (pool, service, actor, default_role)
    }

    #[tokio::test]
    async fn default_role_permissions_can_change_without_structural_mutation() {
        let (_pool, service, actor, default_role) = fixture().await;
        let requested =
            (Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY).bits() as i64;
        let updated = service
            .update_role(
                &actor,
                &server_id("server"),
                &default_role,
                "@everyone",
                None,
                requested,
            )
            .await
            .unwrap();
        assert_eq!(updated.permissions, requested);
        assert!(
            service
                .update_role(
                    &actor,
                    &server_id("server"),
                    &default_role,
                    "renamed",
                    None,
                    requested
                )
                .await
                .is_err()
        );
        assert!(
            service
                .delete_role(&actor, &server_id("server"), &default_role)
                .await
                .is_err()
        );
        assert!(
            service
                .set_member_role(&actor, &server_id("server"), "member", &default_role, true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn built_in_role_and_member_avatar_mutations_are_actor_scoped() {
        let (pool, service, owner, _) = fixture().await;
        service
            .update_member_role(&owner, &server_id("server"), "member", "moderator")
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT role FROM server_members WHERE server_id='server' AND user_id='member'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "moderator"
        );
        assert!(matches!(
            service
                .update_member_role(&owner, &server_id("server"), "owner", "member")
                .await,
            Err(OrganizationError::Forbidden)
        ));
        service
            .set_member_avatar(
                &owner,
                &server_id("server"),
                Some("https://cdn.test/avatar.png"),
            )
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT avatar_url FROM server_members WHERE server_id='server' AND user_id='owner'",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
            .as_deref(),
            Some("https://cdn.test/avatar.png")
        );
    }

    #[tokio::test]
    async fn role_projection_version_advances_only_for_committed_changes() {
        let (pool, service, actor, _) = fixture().await;
        service
            .create_role(
                &actor,
                &server_id("server"),
                "colored",
                "Colored",
                Some("#123456"),
                0,
            )
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT role_projection_version FROM servers WHERE id='server'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        service
            .set_member_role(&actor, &server_id("server"), "member", "colored", true)
            .await
            .unwrap();
        service
            .set_member_role(&actor, &server_id("server"), "member", "colored", true)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT role_projection_version FROM servers WHERE id='server'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );
        service
            .delete_role(&actor, &server_id("server"), "colored")
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT role_projection_version FROM servers WHERE id='server'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
    }

    #[tokio::test]
    async fn server_provisioning_is_atomic_and_selects_a_real_default_channel() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = OrganizationService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        service
            .provision_server(
                &actor,
                "Created",
                None,
                &server_id("server"),
                &channel_id("general"),
                "created-server",
            )
            .await
            .unwrap();
        let defaults: (i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM servers WHERE id='server'), \
                (SELECT count(*) FROM server_members WHERE server_id='server' AND user_id='owner'), \
                (SELECT count(*) FROM roles WHERE server_id='server' AND is_default=1), \
                (SELECT count(*) FROM channels WHERE server_id='server' AND is_default=1), \
                (SELECT count(*) FROM channel_aliases WHERE server_id='server' AND alias='general')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(defaults, (1, 1, 1, 1, 1));

        assert!(
            service
                .provision_server(
                    &actor,
                    "Broken",
                    None,
                    &server_id("broken"),
                    &channel_id("broken-channel"),
                    "created-server",
                )
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM servers WHERE id='broken'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn revoked_actor_cannot_provision_or_delete_after_admission() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = OrganizationService::new(
            pool.clone(),
            auth.clone(),
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        service
            .provision_server(
                &actor,
                "Existing",
                None,
                &server_id("existing"),
                &channel_id("general"),
                "existing",
            )
            .await
            .unwrap();
        sqlx::query("UPDATE users SET is_system_admin=1 WHERE id='owner'")
            .execute(&pool)
            .await
            .unwrap();
        auth.revoke_credential(actor.credential_id()).await.unwrap();

        assert!(matches!(
            service
                .provision_server(
                    &actor,
                    "Denied",
                    None,
                    &server_id("denied"),
                    &channel_id("denied-general"),
                    "denied"
                )
                .await,
            Err(OrganizationError::Authentication(_))
        ));
        assert!(matches!(
            service
                .delete_owned_server(&actor, &server_id("existing"))
                .await,
            Err(OrganizationError::Authentication(_))
        ));
        assert!(matches!(
            service
                .update_server(&actor, &server_id("existing"), Some("Denied rename"), None)
                .await,
            Err(OrganizationError::Authentication(_))
        ));
        assert!(matches!(
            service
                .admin_delete_server(&actor, &server_id("existing"))
                .await,
            Err(OrganizationError::Authentication(_))
        ));
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM servers WHERE id='existing'), \
                    (SELECT count(*) FROM servers WHERE id='denied')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(counts, (1, 0));
    }

    #[tokio::test]
    async fn reorder_is_complete_and_role_deletion_invalidates_overrides() {
        let (pool, service, owner, _) = fixture().await;
        service
            .create_channel(
                &owner,
                CreateChannel {
                    server_id: &server_id("server"),
                    channel_id: &channel_id("one"),
                    name: "#one",
                    category_id: None,
                    is_private: false,
                    channel_type: "text",
                },
            )
            .await
            .unwrap();
        service
            .create_channel(
                &owner,
                CreateChannel {
                    server_id: &server_id("server"),
                    channel_id: &channel_id("two"),
                    name: "#two",
                    category_id: None,
                    is_private: false,
                    channel_type: "text",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            service
                .reorder_channels(
                    &owner,
                    &server_id("server"),
                    &[ChannelPositionInfo {
                        id: "one".into(),
                        position: 0,
                        category_id: None,
                    }],
                )
                .await,
            Err(OrganizationError::InvalidInput(_))
        ));
        service
            .reorder_channels(
                &owner,
                &server_id("server"),
                &[
                    ChannelPositionInfo {
                        id: "one".into(),
                        position: 1,
                        category_id: None,
                    },
                    ChannelPositionInfo {
                        id: "two".into(),
                        position: 0,
                        category_id: None,
                    },
                ],
            )
            .await
            .unwrap();

        let target = service
            .create_role(
                &owner,
                &server_id("server"),
                "target",
                "Target",
                Some("#123ABC"),
                0,
            )
            .await
            .unwrap();
        let manager_permissions =
            (Permissions::VIEW_CHANNELS | Permissions::MANAGE_ROLES).bits() as i64;
        service
            .create_role(
                &owner,
                &server_id("server"),
                "manager-role",
                "Manager",
                Some("#ABCDEF"),
                manager_permissions,
            )
            .await
            .unwrap();
        service
            .set_member_role(&owner, &server_id("server"), "member", "manager-role", true)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits,deny_bits) VALUES('override','one','role','target',1,0)")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, manager) = auth.issue_web_session("member").await.unwrap();
        let manager_service = OrganizationService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        assert!(matches!(
            manager_service
                .update_role(
                    &manager,
                    &server_id("server"),
                    &target.id,
                    "Target",
                    Some("#123ABC"),
                    Permissions::ADMINISTRATOR.bits() as i64,
                )
                .await,
            Err(OrganizationError::Forbidden)
        ));
        manager_service
            .delete_role(&manager, &server_id("server"), &target.id)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM channel_permission_overrides WHERE target_id='target'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn channel_overrides_are_scoped_validated_and_reversible() {
        let (pool, service, owner, default_role) = fixture().await;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')",
        )
        .execute(&pool)
        .await
        .unwrap();

        service
            .set_channel_override(
                &owner,
                ChannelOverrideUpdate {
                    server_id: &server_id("server"),
                    channel_id: &channel_id("channel"),
                    target_type: "role",
                    target_id: &default_role,
                    allow_bits: Permissions::SEND_MESSAGES.bits() as i64,
                    deny_bits: Permissions::ATTACH_FILES.bits() as i64,
                },
            )
            .await
            .unwrap();
        let listed = service
            .list_channel_overrides(&owner, &server_id("server"), &channel_id("channel"))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].target_id, default_role);
        assert_eq!(
            listed[0].allow_bits,
            Permissions::SEND_MESSAGES.bits() as i64
        );
        assert_eq!(listed[0].deny_bits, Permissions::ATTACH_FILES.bits() as i64);

        assert!(matches!(
            service
                .set_channel_override(
                    &owner,
                    ChannelOverrideUpdate {
                        server_id: &server_id("server"),
                        channel_id: &channel_id("channel"),
                        target_type: "role",
                        target_id: "missing-role",
                        allow_bits: Permissions::SEND_MESSAGES.bits() as i64,
                        deny_bits: 0,
                    },
                )
                .await,
            Err(OrganizationError::Forbidden)
        ));
        assert!(matches!(
            service
                .set_channel_override(
                    &owner,
                    ChannelOverrideUpdate {
                        server_id: &server_id("server"),
                        channel_id: &channel_id("channel"),
                        target_type: "role",
                        target_id: &default_role,
                        allow_bits: Permissions::ADMINISTRATOR.bits() as i64,
                        deny_bits: 0,
                    },
                )
                .await,
            Err(OrganizationError::InvalidInput(_))
        ));

        service
            .delete_channel_override(
                &owner,
                &server_id("server"),
                &channel_id("channel"),
                "role",
                &default_role,
            )
            .await
            .unwrap();
        assert!(
            service
                .list_channel_overrides(&owner, &server_id("server"), &channel_id("channel"))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
