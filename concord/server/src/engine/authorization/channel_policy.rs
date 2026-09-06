use super::{
    AuthorizationError, AuthorizationService, ChannelAction, ChannelOverride, ChannelRow,
    OverrideTargetType, Permissions, Row, ServerAuthority, ServerRole, SqliteConnection,
    compute_effective_permissions,
};

impl AuthorizationService {
    pub(crate) async fn authorize_channel_in(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let channel = self.load_channel(connection, channel_id).await?;
        let authority = self
            .server_authority(connection, user_id, &channel.server_id)
            .await?;
        let permissions = self
            .channel_permissions(connection, user_id, &channel, &authority)
            .await?;
        if !permissions.contains(Permissions::VIEW_CHANNELS)
            || !permissions.contains(action.permission())
            || !self
                .visibility_granted(connection, user_id, &channel, &authority)
                .await?
        {
            return Err(AuthorizationError::Unavailable);
        }
        Ok(())
    }

    pub(super) async fn load_channel(
        &self,
        connection: &mut SqliteConnection,
        channel_id: &str,
    ) -> Result<ChannelRow, AuthorizationError> {
        sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels WHERE id=?")
            .bind(channel_id)
            .fetch_optional(connection)
            .await?
            .ok_or(AuthorizationError::Unavailable)
    }

    pub(super) async fn server_authority(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        server_id: &str,
    ) -> Result<ServerAuthority, AuthorizationError> {
        let row = sqlx::query(
            "SELECT s.owner_id,sm.role FROM servers s JOIN server_members sm ON sm.server_id=s.id AND sm.user_id=? WHERE s.id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id=sm.user_id)",
        )
        .bind(user_id)
        .bind(server_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(AuthorizationError::Unavailable)?;
        let owner_id: String = row.get(0);
        let member_role: String = row.get(1);
        let default = sqlx::query("SELECT id,permissions FROM roles WHERE server_id=? AND is_default=1 ORDER BY id LIMIT 1")
            .bind(server_id).fetch_optional(&mut *connection).await?;
        let (default_role_id, base_permissions) = match default {
            Some(row) => (
                row.get(0),
                Permissions::from_bits_truncate(row.get::<i64, _>(1) as u64),
            ),
            None => (
                String::new(),
                ServerRole::parse(&member_role).to_default_permissions(),
            ),
        };
        let rows = sqlx::query("SELECT r.id,r.permissions FROM roles r JOIN user_roles ur ON ur.role_id=r.id AND ur.server_id=r.server_id WHERE ur.server_id=? AND ur.user_id=?")
            .bind(server_id).bind(user_id).fetch_all(&mut *connection).await?;
        let role_permissions: Vec<(String, Permissions)> = rows
            .into_iter()
            .map(|row| {
                (
                    row.get(0),
                    Permissions::from_bits_truncate(row.get::<i64, _>(1) as u64),
                )
            })
            .collect();
        let privileged = owner_id == user_id
            || matches!(member_role.as_str(), "owner" | "admin")
            || role_permissions
                .iter()
                .any(|(_, permissions)| permissions.contains(Permissions::ADMINISTRATOR));
        Ok(ServerAuthority {
            owner_id,
            role_permissions,
            default_role_id,
            base_permissions,
            privileged,
        })
    }

    pub(super) async fn channel_permissions(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel: &ChannelRow,
        authority: &ServerAuthority,
    ) -> Result<Permissions, AuthorizationError> {
        if authority.privileged {
            return Ok(Permissions::all());
        }
        let rows = sqlx::query("SELECT target_type,target_id,allow_bits,deny_bits FROM channel_permission_overrides WHERE channel_id=?")
            .bind(&channel.id).fetch_all(connection).await?;
        let overrides = rows
            .into_iter()
            .map(|row| ChannelOverride {
                target_type: if row.get::<String, _>(0) == "role" {
                    OverrideTargetType::Role
                } else {
                    OverrideTargetType::User
                },
                target_id: row.get(1),
                allow: Permissions::from_bits_truncate(row.get::<i64, _>(2) as u64),
                deny: Permissions::from_bits_truncate(row.get::<i64, _>(3) as u64),
            })
            .collect::<Vec<_>>();
        Ok(compute_effective_permissions(
            authority.base_permissions,
            &authority.role_permissions,
            &overrides,
            &authority.default_role_id,
            user_id,
            false,
        ))
    }

    pub(super) async fn visibility_granted(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel: &ChannelRow,
        authority: &ServerAuthority,
    ) -> Result<bool, AuthorizationError> {
        if channel.visibility_repair_required != 0 {
            return Ok(false);
        }
        if channel.channel_type == "public_thread" || channel.channel_type == "private_thread" {
            let Some(parent_id) = channel.parent_channel_id.as_deref() else {
                return Ok(false);
            };
            let parent = self.load_channel(connection, parent_id).await?;
            if parent.server_id != channel.server_id || parent.channel_type.ends_with("thread") {
                return Ok(false);
            }
            let parent_permissions = self
                .channel_permissions(connection, user_id, &parent, authority)
                .await?;
            if !parent_permissions.contains(Permissions::VIEW_CHANNELS) {
                return Ok(false);
            }
            if !self
                .visibility_granted_non_thread(connection, user_id, &parent, authority)
                .await?
            {
                return Ok(false);
            }
            if channel.channel_type == "private_thread" && !authority.privileged {
                return sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM thread_members WHERE thread_id=? AND user_id=?)",
                )
                .bind(&channel.id)
                .bind(user_id)
                .fetch_one(connection)
                .await
                .map_err(Into::into);
            }
            return Ok(true);
        }
        self.visibility_granted_non_thread(connection, user_id, channel, authority)
            .await
    }

    pub(super) async fn visibility_granted_non_thread(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel: &ChannelRow,
        authority: &ServerAuthority,
    ) -> Result<bool, AuthorizationError> {
        if channel.visibility_repair_required != 0 {
            return Ok(false);
        }
        if channel.is_private == 0 || authority.privileged {
            return Ok(true);
        }
        let granted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channel_visibility_grants g WHERE g.channel_id=? AND ((g.target_type='user' AND g.target_id=?) OR (g.target_type='role' AND g.target_id IN (SELECT role_id FROM user_roles WHERE server_id=? AND user_id=?))))",
        )
        .bind(&channel.id).bind(user_id)
        .bind(&channel.server_id).bind(user_id).fetch_one(connection).await?;
        Ok(granted)
    }
}
