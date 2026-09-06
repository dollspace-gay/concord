use sqlx::{Row, SqlitePool};

use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AtprotoPublication {
    pub id: String,
    pub status: String,
    pub remote_uri: Option<String>,
    pub remote_cid: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AtprotoPublicationStatus {
    pub id: String,
    pub source_message_id: String,
    pub source_version: i64,
    pub channel_id: String,
    pub status: String,
    pub remote_uri: Option<String>,
    pub remote_cid: Option<String>,
    pub safe_error_code: Option<String>,
    pub updated_at: String,
    pub retryable: bool,
    pub reauthentication_required: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AtprotoChannelPublicationPolicy {
    pub channel_id: String,
    pub eligible: bool,
    pub channel_enabled: bool,
    pub user_granted: bool,
}

pub async fn list_publications(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<AtprotoPublicationStatus>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id,p.source_message_id,p.source_version,m.channel_id,p.status,
                p.remote_uri,p.remote_cid,p.safe_error_code,p.updated_at,
                p.status IN ('failed','cancelled') AS retryable,
                NOT EXISTS(SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=p.user_id
                  AND oa.provider='atproto' AND oa.credential_state='active')
                  AS reauthentication_required
         FROM atproto_publications p JOIN messages m ON m.id=p.source_message_id
         WHERE p.user_id=? ORDER BY p.updated_at DESC,p.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum PublicationRequestError {
    #[error("publication source is unavailable")]
    Unavailable,
    #[error("publication authentication is no longer valid")]
    Authentication,
    #[error("publication dependency is unavailable")]
    DependencyUnavailable,
    #[error("publication storage failed")]
    Database(#[from] sqlx::Error),
}

/// Authorize and durably schedule a public export in one transaction. Remote
/// work re-reads this record and the source visibility before dispatch.
pub async fn request_publication(
    admission: &crate::engine::write_admission::WriteAdmission,
    authorization: &crate::engine::authorization::AuthorizationService,
    auth: &crate::auth::authority::AuthService,
    actor: &crate::auth::authority::Actor,
    message_id: &str,
) -> Result<AtprotoPublication, PublicationRequestError> {
    let (_permit, mut transaction) = admission.begin().await.map_err(|error| match error {
        crate::engine::write_admission::WriteAdmissionError::Database(error) => {
            PublicationRequestError::Database(error)
        }
        crate::engine::write_admission::WriteAdmissionError::Unavailable => {
            PublicationRequestError::DependencyUnavailable
        }
    })?;
    let source = sqlx::query(
        "SELECT m.entity_version,c.id,g.grant_version
         FROM messages m
         JOIN channels c ON c.id=m.channel_id
         JOIN atproto_publication_grants g ON g.user_id=? AND g.channel_id=c.id AND g.enabled=1
         WHERE m.id=? AND m.sender_id=? AND m.deleted_at IS NULL
           AND c.is_private=0 AND c.atproto_publication_enabled=1
           AND c.visibility_repair_required=0 AND c.parent_channel_id IS NULL
           AND c.channel_type NOT IN ('public_thread','private_thread')",
    )
    .bind(actor.user_id().as_str())
    .bind(message_id)
    .bind(actor.user_id().as_str())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(PublicationRequestError::Unavailable)?;
    let source_version: i64 = source.get(0);
    let channel_id: String = source.get(1);
    let grant_version: i64 = source.get(2);
    authorization
        .authorize_actor_in(
            &mut transaction,
            auth,
            actor,
            &channel_id,
            crate::engine::authorization::ChannelAction::View,
        )
        .await
        .map_err(map_publication_authorization_error)?;
    authorization
        .authorize_actor_in(
            &mut transaction,
            auth,
            actor,
            &channel_id,
            crate::engine::authorization::ChannelAction::ReadHistory,
        )
        .await
        .map_err(map_publication_authorization_error)?;

    if let Some(existing) = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT id,status,remote_uri,remote_cid FROM atproto_publications
         WHERE user_id=? AND source_message_id=? AND destination='bluesky'",
    )
    .bind(actor.user_id().as_str())
    .bind(message_id)
    .fetch_optional(&mut *transaction)
    .await?
    {
        transaction.commit().await?;
        return Ok(AtprotoPublication {
            id: existing.0,
            status: existing.1,
            remote_uri: existing.2,
            remote_cid: existing.3,
        });
    }

    let id = Uuid::new_v4().to_string();
    let record_key = id.replace('-', "");
    sqlx::query(
        "INSERT INTO atproto_publications
         (id,user_id,source_message_id,source_version,destination,collection,record_key,status)
         VALUES(?,?,?,?,'bluesky','app.bsky.feed.post',?,'pending')",
    )
    .bind(&id)
    .bind(actor.user_id().as_str())
    .bind(message_id)
    .bind(source_version)
    .bind(&record_key)
    .execute(&mut *transaction)
    .await?;
    let job_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO external_jobs
         (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json)
         VALUES(?,?, 'atproto_publish',?,?,?,?)",
    )
    .bind(job_id)
    .bind(format!("atproto-publication:{id}:{source_version}"))
    .bind(&id)
    .bind(source_version)
    .bind(format!(
        "atproto-user:{}:{grant_version}",
        actor.user_id().as_str()
    ))
    .bind(serde_json::json!({"publication_id": &id, "grant_version": grant_version}).to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(AtprotoPublication {
        id,
        status: "pending".into(),
        remote_uri: None,
        remote_cid: None,
    })
}

/// Requeue a failed/cancelled publication using its stable collection/key.
/// Create and update retries use `putRecord`, so an unknown remote success is
/// reconciled without creating another record.
pub async fn retry_publication(
    admission: &crate::engine::write_admission::WriteAdmission,
    authorization: &crate::engine::authorization::AuthorizationService,
    auth: &crate::auth::authority::AuthService,
    actor: &crate::auth::authority::Actor,
    publication_id: &str,
) -> Result<AtprotoPublication, PublicationRequestError> {
    let (_permit, mut transaction) = admission.begin().await.map_err(|error| match error {
        crate::engine::write_admission::WriteAdmissionError::Database(error) => {
            PublicationRequestError::Database(error)
        }
        crate::engine::write_admission::WriteAdmissionError::Unavailable => {
            PublicationRequestError::DependencyUnavailable
        }
    })?;
    auth.validate_actor_in(&mut transaction, actor)
        .await
        .map_err(|error| match error {
            crate::auth::authority::AuthError::Database(_)
            | crate::auth::authority::AuthError::VerificationBusy
            | crate::auth::authority::AuthError::HashWorker(_) => {
                PublicationRequestError::DependencyUnavailable
            }
            _ => PublicationRequestError::Authentication,
        })?;
    let row = sqlx::query(
        "SELECT p.source_message_id,p.source_version,p.status,p.remote_uri,p.remote_cid,
                m.channel_id,m.deleted_at,g.grant_version,c.atproto_publication_enabled,
                c.is_private,c.visibility_repair_required,c.parent_channel_id,c.channel_type,
                EXISTS(SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=p.user_id
                  AND oa.provider='atproto' AND oa.credential_state='active')
         FROM atproto_publications p JOIN messages m ON m.id=p.source_message_id
         JOIN channels c ON c.id=m.channel_id
         LEFT JOIN atproto_publication_grants g ON g.user_id=p.user_id
              AND g.channel_id=c.id AND g.enabled=1
         WHERE p.id=? AND p.user_id=? AND p.status IN ('failed','cancelled')",
    )
    .bind(publication_id)
    .bind(actor.user_id().as_str())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(PublicationRequestError::Unavailable)?;
    let channel_id: String = row.get(5);
    let deleted = row.get::<Option<String>, _>(6).is_some();
    let remote_uri: Option<String> = row.get(3);
    if !row.get::<bool, _>(13) {
        return Err(PublicationRequestError::Authentication);
    }
    let operation = if deleted {
        "atproto_delete"
    } else {
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(map_publication_authorization_error)?;
        let eligible = row.get::<Option<i64>, _>(7).is_some()
            && row.get::<i64, _>(8) == 1
            && row.get::<i64, _>(9) == 0
            && row.get::<i64, _>(10) == 0
            && row.get::<Option<String>, _>(11).is_none()
            && !matches!(
                row.get::<String, _>(12).as_str(),
                "public_thread" | "private_thread"
            );
        if !eligible {
            return Err(PublicationRequestError::Unavailable);
        }
        if remote_uri.is_some() {
            "atproto_update"
        } else {
            "atproto_publish"
        }
    };
    let source_version: i64 = row.get(1);
    let grant_version = row.get::<Option<i64>, _>(7).unwrap_or(0);
    let status = match operation {
        "atproto_delete" => "delete_pending",
        "atproto_update" => "update_pending",
        _ => "pending",
    };
    let updated = sqlx::query(
        "UPDATE atproto_publications SET status=?,safe_error_code=NULL,updated_at=datetime('now')
         WHERE id=? AND source_version=? AND status IN ('failed','cancelled')",
    )
    .bind(status)
    .bind(publication_id)
    .bind(source_version)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(PublicationRequestError::Unavailable);
    }
    let job_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO external_jobs
         (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json)
         VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&job_id)
    .bind(format!("atproto-publication:{publication_id}:{source_version}:retry:{job_id}"))
    .bind(operation)
    .bind(publication_id)
    .bind(source_version)
    .bind(format!("atproto-user:{}:{grant_version}", actor.user_id().as_str()))
    .bind(serde_json::json!({"publication_id": publication_id, "reconcile": true}).to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(AtprotoPublication {
        id: publication_id.to_owned(),
        status: status.into(),
        remote_uri,
        remote_cid: row.get(4),
    })
}

/// Advance an existing publication alongside its canonical source mutation.
/// `putRecord` makes create/update convergence safe with the retained key, and
/// deletion must be attempted even when an earlier create response was lost.
pub async fn schedule_source_mutation(
    connection: &mut sqlx::SqliteConnection,
    message_id: &str,
    source_version: i64,
    deleted: bool,
) -> Result<(), sqlx::Error> {
    let publication = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT p.id,p.user_id,COALESCE(g.grant_version,0) \
         FROM atproto_publications p \
         JOIN messages m ON m.id=p.source_message_id \
         LEFT JOIN atproto_publication_grants g \
           ON g.user_id=p.user_id AND g.channel_id=m.channel_id AND g.enabled=1 \
         WHERE p.source_message_id=? AND p.status!='deleted' \
           AND (? OR p.status!='cancelled')",
    )
    .bind(message_id)
    .bind(deleted)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((publication_id, user_id, grant_version)) = publication else {
        return Ok(());
    };
    let (operation, status) = if deleted {
        ("atproto_delete", "delete_pending")
    } else {
        ("atproto_update", "update_pending")
    };
    sqlx::query(
        "UPDATE atproto_publications \
         SET source_version=?,status=?,safe_error_code=NULL,updated_at=datetime('now') \
         WHERE id=?",
    )
    .bind(source_version)
    .bind(status)
    .bind(&publication_id)
    .execute(&mut *connection)
    .await?;
    let job_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO external_jobs \
         (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) \
         VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&job_id)
    // Match the database trigger's key so callers can safely invoke this
    // helper after a canonical message update without scheduling a duplicate.
    .bind(format!(
        "atproto-publication:{publication_id}:{source_version}"
    ))
    .bind(operation)
    .bind(&publication_id)
    .bind(source_version)
    .bind(format!("atproto-user:{user_id}:{grant_version}"))
    .bind(
        serde_json::json!({
            "publication_id": publication_id,
            "source_mutation": true,
        })
        .to_string(),
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn map_publication_authorization_error(
    error: crate::engine::authorization::AuthorizationError,
) -> PublicationRequestError {
    use crate::auth::authority::AuthError;
    use crate::engine::authorization::AuthorizationError;
    match error {
        AuthorizationError::Unavailable => PublicationRequestError::Unavailable,
        AuthorizationError::Database(error) => PublicationRequestError::Database(error),
        AuthorizationError::Authentication(
            AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_),
        ) => PublicationRequestError::DependencyUnavailable,
        AuthorizationError::Authentication(_) => PublicationRequestError::Authentication,
    }
}

#[cfg(test)]
mod tests;

mod profiles;
pub use profiles::BskyProfileSyncRow;
pub use profiles::StoreBskyProfileParams;
pub use profiles::get_bsky_handle;
pub use profiles::get_bsky_profile_sync;
pub use profiles::get_shared_post;
pub use profiles::insert_shared_post;
pub use profiles::store_bsky_profile_sync;
