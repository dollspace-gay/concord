use super::{RepairFinding, Row, SqliteConnection, column_exists, object_exists};

pub(super) async fn inspect_repairs(
    conn: &mut SqliteConnection,
    version: i64,
) -> Result<Vec<RepairFinding>, sqlx::Error> {
    let mut findings = Vec::new();
    if version >= 1 {
        let query = if version == 1 {
            "SELECT c.name, c.name, EXISTS(SELECT 1 FROM messages m WHERE m.channel_name=c.name), EXISTS(SELECT 1 FROM channel_members cm WHERE cm.channel_name=c.name) FROM channels c WHERE c.name IN ('#general','#random') AND c.is_default=1"
        } else {
            "SELECT c.id, c.name, EXISTS(SELECT 1 FROM messages m WHERE m.channel_id=c.id), EXISTS(SELECT 1 FROM channel_members cm WHERE cm.channel_id=c.id) FROM channels c WHERE c.server_id='default' AND NOT EXISTS(SELECT 1 FROM servers s WHERE s.id=c.server_id)"
        };
        for row in sqlx::query(query).fetch_all(&mut *conn).await? {
            let id: String = row.get(0);
            let name: String = row.get(1);
            let populated = row.get::<i64, _>(2) != 0 || row.get::<i64, _>(3) != 0;
            let known = matches!(name.as_str(), "#general" | "#random");
            findings.push(RepairFinding {
                code: "legacy_default_server",
                object_type: "channel",
                object_id: id,
                detail: if !populated && known {
                    "known generated empty channel will be removed and recorded".into()
                } else {
                    "orphan channel is populated or unrecognized; operator server mapping required"
                        .into()
                },
                blocks_upgrade: populated || !known,
            });
        }
    }
    if version >= 14 && object_exists(conn, "table", "channel_permission_overrides").await? {
        let rows = sqlx::query("SELECT id,target_id FROM channel_permission_overrides o WHERE target_type='user' AND NOT EXISTS(SELECT 1 FROM users u WHERE u.id=o.target_id)").fetch_all(&mut *conn).await?;
        for row in rows {
            findings.push(RepairFinding { code: "unresolved_user_override", object_type: "channel_permission_override", object_id: row.get(0), detail: format!("target {} is not a current user; audited identity mapping required and grant remains denied", row.get::<String, _>(1)), blocks_upgrade: true });
        }
    }
    if (7..14).contains(&version)
        && object_exists(conn, "table", "oauth_accounts").await?
        && object_exists(conn, "table", "channel_permission_overrides").await?
    {
        let rows = sqlx::query(
            "SELECT user_id,group_concat(provider_id) FROM oauth_accounts \
             WHERE provider='atproto' AND provider_id IS NOT NULL AND provider_id<>user_id \
             GROUP BY user_id HAVING count(DISTINCT provider_id)>1",
        )
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            findings.push(RepairFinding {
                code: "ambiguous_pre014_at_identity",
                object_type: "user",
                object_id: row.get(0),
                detail: format!(
                    "multiple AT Protocol subjects ({}) claim this legacy user; no identity or permission mapping can be inferred",
                    row.get::<String, _>(1)
                ),
                blocks_upgrade: true,
            });
        }
        let rows = sqlx::query(
            "SELECT legacy.id,legacy.target_id,account.provider_id,current.id \
             FROM channel_permission_overrides legacy \
             JOIN oauth_accounts account \
               ON account.user_id=legacy.target_id AND account.provider='atproto' \
              AND account.provider_id IS NOT NULL AND account.provider_id<>account.user_id \
             JOIN channel_permission_overrides current \
               ON current.channel_id=legacy.channel_id AND current.target_type='user' \
              AND current.target_id=account.provider_id AND current.id<>legacy.id \
             WHERE legacy.target_type='user'",
        )
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            findings.push(RepairFinding {
                code: "pre014_override_target_collision",
                object_type: "channel_permission_override",
                object_id: row.get(0),
                detail: format!(
                    "legacy target {} maps to {}, already granted by override {}; operator must reconcile permission bits explicitly",
                    row.get::<String, _>(1),
                    row.get::<String, _>(2),
                    row.get::<String, _>(3)
                ),
                blocks_upgrade: true,
            });
        }
    }
    if version >= 9 && object_exists(conn, "table", "channels").await? {
        let has_parent_channel = column_exists(conn, "channels", "parent_channel_id").await?;
        let rows = if has_parent_channel {
            sqlx::query(
                "SELECT id,parent_channel_id FROM channels c \
                 WHERE c.channel_type IN ('thread','public_thread','private_thread') \
                   AND (c.parent_channel_id IS NULL OR NOT EXISTS( \
                       SELECT 1 FROM channels parent WHERE parent.id=c.parent_channel_id \
                   ))",
            )
            .fetch_all(&mut *conn)
            .await?
        } else {
            sqlx::query(
                "SELECT c.id,c.thread_parent_message_id FROM channels c \
                 WHERE c.channel_type IN ('thread','public_thread','private_thread') \
                   AND (c.thread_parent_message_id IS NULL OR NOT EXISTS( \
                       SELECT 1 FROM messages parent \
                       WHERE parent.id=c.thread_parent_message_id \
                         AND parent.channel_id IS NOT NULL \
                   ))",
            )
            .fetch_all(&mut *conn)
            .await?
        };
        for row in rows {
            findings.push(RepairFinding {
                code: "thread_parent_missing",
                object_type: "channel",
                object_id: row.get(0),
                detail: format!(
                    "thread references unavailable parent {:?}; operator must map a validated parent or quarantine the thread from visibility; destructive removal is not automatic",
                    row.get::<Option<String>, _>(1)
                ),
                blocks_upgrade: true,
            });
        }
    }
    if version >= 2 {
        for (table, column) in [
            ("messages", "created_at"),
            ("channels", "created_at"),
            ("servers", "created_at"),
            ("notification_settings", "updated_at"),
        ] {
            if !column_exists(conn, table, column).await? {
                continue;
            }
            let quoted_table = table.replace('"', "\"\"");
            let quoted_column = column.replace('"', "\"\"");
            let query = format!(
                "SELECT id,{quoted_column} FROM \"{quoted_table}\" \
                 WHERE {quoted_column} IS NULL OR julianday({quoted_column}) IS NULL"
            );
            // Identifiers come from the fixed table/column list above.
            for row in sqlx::query(sqlx::AssertSqlSafe(query))
                .fetch_all(&mut *conn)
                .await?
            {
                findings.push(RepairFinding {
                    code: "malformed_timestamp",
                    object_type: table,
                    object_id: row.get(0),
                    detail: format!(
                        "{column} value {:?} is not a recognized SQLite timestamp; explicit replacement required",
                        row.get::<Option<String>, _>(1)
                    ),
                    blocks_upgrade: true,
                });
            }
        }
        for table in ["users", "servers", "channels", "messages"] {
            if !object_exists(conn, "table", table).await? {
                continue;
            }
            let quoted = table.replace('"', "\"\"");
            let query = format!("SELECT id FROM \"{quoted}\" WHERE trim(id)='' OR length(id)>512");
            // Identifiers come from the fixed table/column list above.
            for row in sqlx::query(sqlx::AssertSqlSafe(query))
                .fetch_all(&mut *conn)
                .await?
            {
                findings.push(RepairFinding {
                    code: "malformed_identifier",
                    object_type: table,
                    object_id: row.get(0),
                    detail: "identifier is empty or exceeds the 512-byte repair bound; explicit remapping required".into(),
                    blocks_upgrade: true,
                });
            }
        }
    }
    if version >= 8 && object_exists(conn, "table", "notification_settings").await? {
        for row in sqlx::query(
            "SELECT id,mute_until FROM notification_settings \
             WHERE mute_until IS NOT NULL AND julianday(mute_until) IS NULL",
        )
        .fetch_all(&mut *conn)
        .await?
        {
            findings.push(RepairFinding {
                code: "malformed_timestamp",
                object_type: "notification_settings",
                object_id: row.get(0),
                detail: format!(
                    "mute_until value {:?} is not a recognized SQLite timestamp; explicit replacement required",
                    row.get::<Option<String>, _>(1)
                ),
                blocks_upgrade: true,
            });
        }
        for row in sqlx::query(
            "SELECT n.id,n.server_id,n.channel_id FROM notification_settings n \
             WHERE (n.channel_id IS NOT NULL AND n.server_id IS NULL) \
                OR (n.channel_id IS NOT NULL AND NOT EXISTS( \
                    SELECT 1 FROM channels c \
                    WHERE c.id=n.channel_id AND c.server_id=n.server_id \
                ))",
        )
        .fetch_all(&mut *conn)
        .await?
        {
            findings.push(RepairFinding {
                code: "malformed_notification_scope",
                object_type: "notification_settings",
                object_id: row.get(0),
                detail: format!(
                    "notification server={:?} channel={:?} is not global, server-scoped, or bound to a channel in that server",
                    row.get::<Option<String>, _>(1),
                    row.get::<Option<String>, _>(2)
                ),
                blocks_upgrade: true,
            });
        }
        let rows = sqlx::query(
            "SELECT user_id,server_id,channel_id,group_concat(id), \
                    count(DISTINCT level || ':' || suppress_everyone || ':' || \
                          suppress_roles || ':' || muted || ':' || COALESCE(mute_until,'')), \
                    sum(CASE WHEN julianday(updated_at) IS NULL THEN 1 ELSE 0 END) \
             FROM notification_settings \
             GROUP BY user_id,server_id,channel_id HAVING count(*)>1",
        )
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            let variants: i64 = row.get(4);
            let invalid_timestamps: i64 = row.get(5);
            findings.push(RepairFinding {
                code: "duplicate_notification_scope",
                object_type: "notification_settings",
                object_id: row.get(3),
                detail: format!(
                    "duplicate scope user={} server={:?} channel={:?} has {variants} distinct setting variant(s) and {invalid_timestamps} invalid updated_at value(s); valid rows are exported and the latest updated_at then greatest stable ID wins",
                    row.get::<String, _>(0),
                    row.get::<Option<String>, _>(1),
                    row.get::<Option<String>, _>(2)
                ),
                blocks_upgrade: invalid_timestamps != 0,
            });
        }
    }
    if version >= 2 {
        for row in sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *conn)
            .await?
        {
            let table: String = row.get(0);
            let row_id: i64 = row.get(1);
            let parent: String = row.get(2);
            let has_safe_default = findings
                .iter()
                .any(|item| item.code == "legacy_default_server" && !item.blocks_upgrade);
            let known_default = if !has_safe_default || parent != "servers" {
                false
            } else if table == "channels" {
                true
            } else if table == "channel_aliases" {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM channel_aliases a JOIN channels c \
                     ON c.id=a.channel_id WHERE a.rowid=? AND a.server_id='default' \
                     AND c.server_id='default' AND c.is_default=1 \
                     AND c.name IN ('#general','#random'))",
                )
                .bind(row_id)
                .fetch_one(&mut *conn)
                .await?
            } else if table == "conversations" {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM conversations cv JOIN channels c \
                     ON c.id=cv.channel_id WHERE cv.rowid=? AND cv.server_id='default' \
                     AND c.server_id='default' AND c.is_default=1 \
                     AND c.name IN ('#general','#random'))",
                )
                .bind(row_id)
                .fetch_one(&mut *conn)
                .await?
            } else {
                false
            };
            if !known_default {
                findings.push(RepairFinding {
                    code: "foreign_key_violation",
                    object_type: "row",
                    object_id: format!("{table}:{row_id}"),
                    detail: format!(
                        "references missing parent table {parent}; explicit repair required"
                    ),
                    blocks_upgrade: true,
                });
            }
        }
    }
    Ok(findings)
}
