use super::{Actor, OrganizationError, OrganizationService, ServerId, ServerInfo};

impl OrganizationService {
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
}
