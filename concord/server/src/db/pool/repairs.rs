use super::{MigrationError, RepairFinding, Row, SqliteConnection, object_exists};

pub(super) async fn apply_safe_repairs(
    conn: &mut SqliteConnection,
    findings: &[RepairFinding],
) -> Result<(), MigrationError> {
    for finding in findings
        .iter()
        .filter(|item| !item.blocks_upgrade && item.code == "legacy_default_server")
    {
        if object_exists(conn, "table", "conversations").await? {
            sqlx::query("DELETE FROM conversations WHERE channel_id=?")
                .bind(&finding.object_id)
                .execute(&mut *conn)
                .await?;
        }
        sqlx::query("DELETE FROM channels WHERE id=? AND server_id='default' AND is_default=1 AND name IN ('#general','#random') AND NOT EXISTS(SELECT 1 FROM messages WHERE channel_id=channels.id) AND NOT EXISTS(SELECT 1 FROM channel_members WHERE channel_id=channels.id)")
            .bind(&finding.object_id).execute(&mut *conn).await?;
        sqlx::query("INSERT INTO migration_repair_log(migration_version,repair_kind,object_type,object_id,outcome,details) VALUES (17,?,?,?,'repaired',?)")
            .bind(finding.code).bind(finding.object_type).bind(&finding.object_id).bind(&finding.detail).execute(&mut *conn).await?;
    }
    Ok(())
}

pub(super) async fn apply_notification_scope_repairs(
    conn: &mut SqliteConnection,
    findings: &[RepairFinding],
) -> Result<(), MigrationError> {
    if !findings
        .iter()
        .any(|finding| finding.code == "duplicate_notification_scope" && !finding.blocks_upgrade)
    {
        return Ok(());
    }
    let scopes = sqlx::query(
        "SELECT user_id,server_id,channel_id FROM notification_settings \
         GROUP BY user_id,server_id,channel_id HAVING count(*)>1",
    )
    .fetch_all(&mut *conn)
    .await?;
    for scope in scopes {
        let user_id: String = scope.get(0);
        let server_id: Option<String> = scope.get(1);
        let channel_id: Option<String> = scope.get(2);
        let winner_id: String = sqlx::query_scalar(
            "SELECT id FROM notification_settings \
             WHERE user_id=? AND server_id IS ? AND channel_id IS ? \
             ORDER BY julianday(updated_at) DESC,updated_at DESC,id DESC LIMIT 1",
        )
        .bind(&user_id)
        .bind(&server_id)
        .bind(&channel_id)
        .fetch_one(&mut *conn)
        .await?;
        let exported: String = sqlx::query_scalar(
            "SELECT json_group_array(json_object( \
                'id',id,'level',level,'suppress_everyone',suppress_everyone, \
                'suppress_roles',suppress_roles,'muted',muted, \
                'mute_until',mute_until,'created_at',created_at,'updated_at',updated_at \
             )) FROM ( \
                SELECT * FROM notification_settings \
                WHERE user_id=? AND server_id IS ? AND channel_id IS ? \
                ORDER BY updated_at,id \
             )",
        )
        .bind(&user_id)
        .bind(&server_id)
        .bind(&channel_id)
        .fetch_one(&mut *conn)
        .await?;
        let exported: serde_json::Value =
            serde_json::from_str(&exported).map_err(|_| MigrationError::Integrity {
                check: "notification repair export",
                detail: "duplicate notification rows could not be encoded".into(),
            })?;
        let details = serde_json::json!({
            "user_id": user_id,
            "server_id": server_id,
            "channel_id": channel_id,
            "winner_id": winner_id,
            "selection": "latest valid updated_at, then greatest stable id",
            "pre_repair_rows": exported,
        });
        sqlx::query(
            "INSERT INTO migration_repair_log( \
                migration_version,repair_kind,object_type,object_id,outcome,details \
             ) VALUES(27,'duplicate_notification_scope','notification_settings',?, \
                      'deduplicated',?)",
        )
        .bind(&winner_id)
        .bind(details.to_string())
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "DELETE FROM notification_settings \
             WHERE user_id=? AND server_id IS ? AND channel_id IS ? AND id<>?",
        )
        .bind(&user_id)
        .bind(&server_id)
        .bind(&channel_id)
        .bind(&winner_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub(super) async fn verify_integrity(conn: &mut SqliteConnection) -> Result<(), MigrationError> {
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *conn)
        .await?;
    if !violations.is_empty() {
        return Err(MigrationError::Integrity {
            check: "foreign_key_check",
            detail: format!("{} violation(s) remain", violations.len()),
        });
    }
    let result: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut *conn)
        .await?;
    if result != "ok" {
        return Err(MigrationError::Integrity {
            check: "integrity_check",
            detail: result,
        });
    }
    Ok(())
}
