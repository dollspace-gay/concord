use super::{AuthorizationError, AuthorizationService, AuthorizationStamp, SqliteConnection};

impl AuthorizationService {
    pub(crate) async fn authorization_stamp(
        &self,
        connection: &mut SqliteConnection,
        server_id: &str,
        channel_ids: &[String],
    ) -> Result<AuthorizationStamp, AuthorizationError> {
        let server_version =
            sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_optional(&mut *connection)
                .await?
                .ok_or(AuthorizationError::Unavailable)?;
        let mut channel_versions = Vec::with_capacity(channel_ids.len());
        for channel_id in channel_ids {
            let version = sqlx::query_scalar(
                "SELECT authorization_version FROM channels WHERE id=? AND server_id=?",
            )
            .bind(channel_id)
            .bind(server_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(AuthorizationError::Unavailable)?;
            channel_versions.push((channel_id.clone(), version));
        }
        Ok(AuthorizationStamp {
            server_id: server_id.to_owned(),
            server_version,
            channel_versions,
        })
    }

    pub async fn stamp_is_current(
        &self,
        stamp: &AuthorizationStamp,
    ) -> Result<bool, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        let server_version: Option<i64> =
            sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id=?")
                .bind(&stamp.server_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if server_version != Some(stamp.server_version) {
            return Ok(false);
        }
        for (channel_id, expected) in &stamp.channel_versions {
            let actual: Option<i64> = sqlx::query_scalar(
                "SELECT authorization_version FROM channels WHERE id=? AND server_id=?",
            )
            .bind(channel_id)
            .bind(&stamp.server_id)
            .fetch_optional(&mut *transaction)
            .await?;
            if actual != Some(*expected) {
                return Ok(false);
            }
        }
        transaction.commit().await?;
        Ok(true)
    }
}
