use super::{ReplayError, SqliteConnection};

pub(super) async fn resolve_current_event_state(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    recorded_version: i64,
    descriptor: &mut serde_json::Value,
) -> Result<i64, ReplayError> {
    if entity_type == "thread_state" {
        let current: Option<(i64, i64, Option<String>)> = sqlx::query_as(
            "SELECT thread_state_version,archived,thread_archive_reason FROM channels WHERE id=?",
        )
        .bind(entity_id)
        .fetch_optional(&mut *connection)
        .await?;
        if let Some((version, archived, reason)) = current {
            *descriptor = serde_json::json!({
                "archived": archived != 0,
                "reason": reason,
            });
            return Ok(version);
        }
    }
    if entity_type == "thread_tags" {
        let version: Option<i64> =
            sqlx::query_scalar("SELECT thread_tags_version FROM channels WHERE id=?")
                .bind(entity_id)
                .fetch_optional(&mut *connection)
                .await?;
        if let Some(version) = version {
            let tag_ids: Vec<String> = sqlx::query_scalar(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(entity_id)
            .fetch_all(&mut *connection)
            .await?;
            *descriptor = serde_json::json!({
                "thread_id": entity_id,
                "tag_ids": tag_ids,
            });
            return Ok(version);
        }
    }

    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT version FROM entity_versions WHERE entity_type=? AND entity_id=?",
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_optional(&mut *connection)
    .await?
    .unwrap_or(recorded_version))
}
