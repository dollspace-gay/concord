use super::{
    AuthorizationError, MessagingError, RATE_WINDOW_MESSAGES, RATE_WINDOW_SECONDS, Row,
    SqliteConnection,
};

pub(super) fn normalize_channel_name(channel: &str) -> String {
    if channel.starts_with('#') {
        channel.to_owned()
    } else {
        format!("#{channel}")
    }
}

pub(super) fn map_authorization_error(error: AuthorizationError) -> MessagingError {
    match error {
        AuthorizationError::Unavailable => MessagingError::Unavailable,
        AuthorizationError::Authentication(_) => MessagingError::Unauthenticated,
        AuthorizationError::Database(error) => MessagingError::Internal(error),
    }
}

pub(super) async fn enforce_timeout(
    connection: &mut SqliteConnection,
    server_id: &str,
    user_id: &str,
) -> Result<(), MessagingError> {
    let timed_out: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
            SELECT 1 FROM server_members \
            WHERE server_id=? AND user_id=? AND timeout_until IS NOT NULL \
              AND timeout_until > datetime('now') \
         )",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_one(connection)
    .await?;
    if timed_out != 0 {
        return Err(MessagingError::Unavailable);
    }
    Ok(())
}

pub(super) async fn enforce_rate_and_slow_mode(
    connection: &mut SqliteConnection,
    channel_id: &str,
    user_id: &str,
    slowmode_seconds: i64,
) -> Result<(), MessagingError> {
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages \
         WHERE channel_id=? AND sender_id=? \
           AND julianday(created_at) >= julianday('now', ?)",
    )
    .bind(channel_id)
    .bind(user_id)
    .bind(format!("-{RATE_WINDOW_SECONDS} seconds"))
    .fetch_one(&mut *connection)
    .await?;
    if recent >= RATE_WINDOW_MESSAGES {
        return Err(MessagingError::RateLimited);
    }
    if slowmode_seconds > 0 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT EXISTS( \
                SELECT 1 FROM messages WHERE channel_id=? AND sender_id=? \
                  AND julianday(created_at) > julianday('now', ?) \
             )",
        )
        .bind(channel_id)
        .bind(user_id)
        .bind(format!("-{slowmode_seconds} seconds"))
        .fetch_one(&mut *connection)
        .await?;
        if blocked != 0 {
            return Err(MessagingError::RateLimited);
        }
    }
    Ok(())
}

pub(super) async fn enforce_automod(
    connection: &mut SqliteConnection,
    server_id: &str,
    actor_id: &str,
    operation_id: &str,
    content: &str,
) -> Result<(), MessagingError> {
    let rows = sqlx::query(
        "SELECT id,name,rule_type,config,action_type,timeout_duration_seconds \
         FROM automod_rules WHERE server_id=? AND enabled=1 ORDER BY created_at,id",
    )
    .bind(server_id)
    .fetch_all(&mut *connection)
    .await?;
    for row in rows {
        let rule_id: String = row.get(0);
        let rule_name: String = row.get(1);
        let rule_type: String = row.get(2);
        let config: serde_json::Value = serde_json::from_str(row.get::<&str, _>(3))
            .map_err(|_| MessagingError::DependencyUnavailable)?;
        let triggered = match rule_type.as_str() {
            "keyword" => config
                .get("words")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|words| {
                    let message_words = content
                        .to_lowercase()
                        .split(|character: char| !character.is_alphanumeric())
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    words.iter().any(|word| {
                        word.as_str().is_some_and(|word| {
                            message_words
                                .iter()
                                .any(|candidate| candidate == &word.to_lowercase())
                        })
                    })
                }),
            "mention_spam" => {
                let maximum = config
                    .get("max_mentions")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(5) as usize;
                content.matches('@').count() > maximum
            }
            "link_filter" => {
                config
                    .get("block_all")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                    && (content.contains("http://") || content.contains("https://"))
            }
            _ => false,
        };
        if triggered {
            let action_type: String = row.get(4);
            let timeout_seconds: Option<i64> = row.get(5);
            if action_type == "timeout" {
                let timeout_seconds = timeout_seconds
                    .filter(|seconds| (1..=2_419_200).contains(seconds))
                    .ok_or(MessagingError::DependencyUnavailable)?;
                sqlx::query(
                    "UPDATE server_members SET timeout_until=datetime('now',?) \
                     WHERE server_id=? AND user_id=?",
                )
                .bind(format!("+{timeout_seconds} seconds"))
                .bind(server_id)
                .bind(actor_id)
                .execute(&mut *connection)
                .await?;
            }
            let audit_id = format!("automod:{rule_id}:{actor_id}:{operation_id}");
            let details = serde_json::json!({
                "rule_id": rule_id,
                "rule_name": rule_name,
                "outcome": action_type,
            })
            .to_string();
            sqlx::query(
                "INSERT OR IGNORE INTO audit_log( \
                    id,server_id,actor_id,actor_username_snapshot,actor_avatar_snapshot, \
                    action_type,target_type,target_id,changes \
                 ) SELECT ?,?,?,COALESCE(u.username,?),u.avatar_url,?,?,?,? \
                   FROM (SELECT 1) LEFT JOIN users u ON u.id=?",
            )
            .bind(audit_id)
            .bind(server_id)
            .bind(actor_id)
            .bind(actor_id)
            .bind(if action_type == "flag" {
                "automod_flag"
            } else {
                "automod_reject"
            })
            .bind("user")
            .bind(actor_id)
            .bind(details)
            .bind(actor_id)
            .execute(&mut *connection)
            .await?;
            if action_type != "flag" {
                return Err(MessagingError::AutoModRejected(format!(
                    "message blocked by AutoMod rule: {rule_name}"
                )));
            }
        }
    }
    Ok(())
}
