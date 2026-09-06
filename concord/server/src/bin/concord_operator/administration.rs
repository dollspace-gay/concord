use super::{Result, Uuid, bail};

pub(super) fn validate_operator_reason(reason: &str) -> Result<&str> {
    let reason = reason.trim();
    if reason.is_empty() || reason.len() > 1000 {
        bail!("operator reason must contain between 1 and 1000 bytes");
    }
    Ok(reason)
}

pub(super) async fn require_verified_human(
    connection: &mut sqlx::SqliteConnection,
    user_id: &str,
    allow_disabled: bool,
) -> Result<()> {
    let row: Option<(i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT u.is_bot,u.disabled_at,EXISTS( \
           SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=u.id \
             AND oa.provider='atproto' AND length(trim(oa.provider_id))>0) \
         FROM users u WHERE u.id=?",
    )
    .bind(user_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((is_bot, disabled_at, has_verified_identity)) = row else {
        bail!("stable user ID was not found: {user_id}");
    };
    if is_bot != 0 {
        bail!("operator recovery requires a human user ID");
    }
    if disabled_at.is_some() && !allow_disabled {
        bail!("operator recovery target is disabled");
    }
    if has_verified_identity != 1 {
        bail!("operator recovery target has no verified AT Protocol identity mapping");
    }
    Ok(())
}

pub(super) async fn insert_operator_audit(
    connection: &mut sqlx::SqliteConnection,
    action_type: &str,
    target_type: &str,
    target_id: &str,
    reason: &str,
    details: &serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO operator_audit_log( \
           id,action_type,target_type,target_id,reason,details_json) \
         VALUES(?,?,?,?,?,?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(action_type)
    .bind(target_type)
    .bind(target_id)
    .bind(reason)
    .bind(serde_json::to_string(details)?)
    .execute(connection)
    .await?;
    Ok(())
}

pub(super) async fn print_admin_inventory(
    pool: &sqlx::SqlitePool,
    configured_ids: &[String],
) -> Result<()> {
    let rows: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT u.id,u.username,u.disabled_at,EXISTS( \
           SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=u.id \
             AND oa.provider='atproto' AND length(trim(oa.provider_id))>0) \
         FROM users u WHERE u.is_system_admin=1 ORDER BY u.id",
    )
    .fetch_all(pool)
    .await?;
    let admins: Vec<_> = rows
        .into_iter()
        .map(|(user_id, username, disabled_at, verified_identity)| {
            let configured_bootstrap = configured_ids.iter().any(|item| item == &user_id);
            serde_json::json!({
                "user_id": user_id,
                "username": username,
                "disabled": disabled_at.is_some(),
                "verified_atproto_identity": verified_identity == 1,
                "configured_bootstrap": configured_bootstrap,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&admins)?);
    Ok(())
}

pub(super) async fn transfer_admin(
    pool: &sqlx::SqlitePool,
    configured_ids: &[String],
    from_user_id: &str,
    to_user_id: &str,
    reason: &str,
) -> Result<()> {
    let reason = validate_operator_reason(reason)?;
    if from_user_id == to_user_id {
        bail!("administrator transfer requires two different stable user IDs");
    }
    if configured_ids.iter().any(|item| item == from_user_id) {
        bail!(
            "remove {from_user_id} from admin.admin_user_ids and validate the configuration before transferring; otherwise a later verified login would restore that privilege"
        );
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_verified_human(&mut transaction, from_user_id, false).await?;
    require_verified_human(&mut transaction, to_user_id, false).await?;
    let source_is_admin: i64 = sqlx::query_scalar("SELECT is_system_admin FROM users WHERE id=?")
        .bind(from_user_id)
        .fetch_one(&mut *transaction)
        .await?;
    if source_is_admin != 1 {
        bail!("transfer source is not a current administrator");
    }
    sqlx::query("UPDATE users SET is_system_admin=1 WHERE id=?")
        .bind(to_user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE users SET is_system_admin=0 WHERE id=? AND is_system_admin=1")
        .bind(from_user_id)
        .execute(&mut *transaction)
        .await?;
    insert_operator_audit(
        &mut transaction,
        "admin_transfer",
        "user",
        to_user_id,
        reason,
        &serde_json::json!({"from_user_id": from_user_id, "to_user_id": to_user_id}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) async fn recover_admin(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    reason: &str,
) -> Result<()> {
    let reason = validate_operator_reason(reason)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_verified_human(&mut transaction, user_id, false).await?;
    let changed =
        sqlx::query("UPDATE users SET is_system_admin=1 WHERE id=? AND is_system_admin=0")
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
    if changed.rows_affected() != 1 {
        bail!("recovery target is already a current administrator");
    }
    insert_operator_audit(
        &mut transaction,
        "admin_recovery",
        "user",
        user_id,
        reason,
        &serde_json::json!({"user_id": user_id}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) async fn revoke_all_user_credentials(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    reason: &str,
) -> Result<u64> {
    let reason = validate_operator_reason(reason)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_verified_human(&mut transaction, user_id, true).await?;
    let local_credentials = sqlx::query(
        "UPDATE auth_credentials SET revoked_at=unixepoch(),version=version+1 \
         WHERE user_id=? AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let delegated_tokens = sqlx::query(
        "UPDATE oauth2_tokens SET revoked_at=COALESCE(revoked_at,datetime('now')) \
         WHERE grant_id IN(SELECT id FROM oauth2_grants WHERE user_id=?) \
           AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let delegated_grants = sqlx::query(
        "UPDATE oauth2_grants SET state='revoked',revoked_at=datetime('now'), \
                grant_version=grant_version+1 \
         WHERE user_id=? AND state='active'",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let authorization_codes = sqlx::query(
        "UPDATE oauth2_codes SET consumed_at=COALESCE(consumed_at,datetime('now')) \
         WHERE user_id=? AND consumed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let consent_requests = sqlx::query(
        "UPDATE oauth2_consent_requests \
         SET consumed_at=COALESCE(consumed_at,datetime('now')) \
         WHERE user_id=? AND consumed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let revoked = local_credentials.rows_affected()
        + delegated_tokens.rows_affected()
        + delegated_grants.rows_affected()
        + authorization_codes.rows_affected()
        + consent_requests.rows_affected();
    insert_operator_audit(
        &mut transaction,
        "credential_revoke_all",
        "user",
        user_id,
        reason,
        &serde_json::json!({
            "user_id": user_id,
            "local_credentials": local_credentials.rows_affected(),
            "delegated_tokens": delegated_tokens.rows_affected(),
            "delegated_grants": delegated_grants.rows_affected(),
            "authorization_codes": authorization_codes.rows_affected(),
            "consent_requests": consent_requests.rows_affected(),
        }),
    )
    .await?;
    transaction.commit().await?;
    Ok(revoked)
}
