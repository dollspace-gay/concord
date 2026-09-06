use super::{
    CategoryInfo, ChannelPositionInfo, ChatEngine, ConnectionId, Uuid, category_row_to_info,
    referenced_server_id,
};

impl ChatEngine {
    /// List categories for a server.
    pub async fn list_categories(&self, server_id: &str) -> Result<Vec<CategoryInfo>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = crate::db::queries::categories::list_categories(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        Ok(rows.into_iter().map(category_row_to_info).collect())
    }
    /// Create a channel category.
    pub async fn create_category(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let cat_id = Uuid::new_v4().to_string();
        crate::engine::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .create_category(&actor, &referenced_server_id(server_id)?, &cat_id, name)
        .await
        .map_err(Into::into)
    }
    /// Update a channel category name.
    pub async fn update_category(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        category_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        crate::engine::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .update_category(&actor, &referenced_server_id(server_id)?, category_id, name)
        .await
        .map_err(Into::into)
    }
    /// Delete a channel category.
    pub async fn delete_category(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        category_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        crate::engine::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .delete_category(&actor, &referenced_server_id(server_id)?, category_id)
        .await?;
        // Channels referencing this category get NULL (ON DELETE SET NULL)
        // Update in-memory state
        for mut ch in self.channels.iter_mut() {
            if ch.category_id.as_deref() == Some(category_id) {
                ch.category_id = None;
            }
        }
        Ok(())
    }
    /// Reorder channels: update position and category for a batch of channels.
    pub async fn reorder_channels(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        updates: &[ChannelPositionInfo],
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        crate::engine::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .reorder_channels(&actor, &referenced_server_id(server_id)?, updates)
        .await?;
        for update in updates {
            // Update in-memory state
            if let Some(mut ch) = self.channels.get_mut(&update.id) {
                ch.position = update.position;
                ch.category_id.clone_from(&update.category_id);
            }
        }
        Ok(())
    }
}
