use super::{
    Actor, CategoryInfo, ChannelPositionInfo, OrganizationError, OrganizationService, Permissions,
    Row, ServerId,
};

impl OrganizationService {
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
}
