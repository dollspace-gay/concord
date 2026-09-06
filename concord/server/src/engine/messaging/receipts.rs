use super::{MessagingError, SqliteConnection};

pub(super) async fn database_generation(
    connection: &mut SqliteConnection,
) -> Result<String, MessagingError> {
    Ok(
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(connection)
            .await?,
    )
}
