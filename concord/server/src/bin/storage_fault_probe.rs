use std::io::Write;
use std::time::Duration;

use concord_server::auth::authority::AuthService;
#[cfg(feature = "storage-fault-injection")]
use concord_server::db::pool::{
    arm_storage_sync_fault, create_storage_fault_pool, observed_storage_sync_faults,
};
use concord_server::db::pool::{create_pool, run_migrations};
use concord_server::engine::messaging::{
    ContentFormat, MessagingError, MessagingService, SendMessageCommand,
};
use sqlx::{Row, SqlitePool};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    let command = arguments.get(1).map(String::as_str).unwrap_or("");
    let database = arguments
        .get(2)
        .ok_or_else(|| anyhow::anyhow!("missing database path"))?;
    let url = format!("sqlite://{database}?mode=rwc");
    #[cfg(feature = "storage-fault-injection")]
    let pool = if command == "sync-fault" {
        create_storage_fault_pool(&url).await?
    } else {
        create_pool(&url).await?
    };
    #[cfg(not(feature = "storage-fault-injection"))]
    let pool = create_pool(&url).await?;
    match command {
        "init" => initialize(&pool).await?,
        "send" => {
            let client_id = arguments
                .get(3)
                .ok_or_else(|| anyhow::anyhow!("missing client ID"))?;
            let content = arguments
                .get(4)
                .ok_or_else(|| anyhow::anyhow!("missing content"))?;
            let auth = AuthService::new(pool.clone(), "probe-secret".into(), 1);
            let actor = auth.issue_web_session("user").await?.1;
            let service = MessagingService::new(pool, auth, 4000);
            match service
                .send_channel_message(
                    &actor,
                    SendMessageCommand {
                        request_id: client_id,
                        client_message_id: client_id,
                        operation_generation: None,
                        conversation_id: None,
                        server_id: "server",
                        channel: "#general",
                        content,
                        content_format: ContentFormat::Plain,
                        reply_to_id: None,
                        attachment_ids: &[],
                        mentions: &[],
                    },
                )
                .await
            {
                Ok(receipt) => println!(
                    "{} {} {}",
                    if receipt.replayed {
                        "replayed"
                    } else {
                        "committed"
                    },
                    receipt.message_id,
                    receipt.sequence
                ),
                Err(MessagingError::IdempotencyConflict) => println!("conflict"),
                Err(error) => anyhow::bail!("send failed: {error:?}"),
            }
        }
        #[cfg(feature = "storage-fault-injection")]
        "canonical-crash" => {
            use concord_server::engine::messaging::StorageFaultBarrierStage;
            let stage = match arguments.get(3).map(String::as_str) {
                Some("before") => StorageFaultBarrierStage::BeforeCommit,
                Some("after") => StorageFaultBarrierStage::AfterCommit,
                _ => anyhow::bail!("stage must be before or after"),
            };
            let marker = arguments
                .get(4)
                .ok_or_else(|| anyhow::anyhow!("missing marker"))?;
            let auth = AuthService::new(pool.clone(), "probe-secret".into(), 1);
            let actor = auth.issue_web_session("user").await?.1;
            let service =
                MessagingService::new(pool, auth, 4000).with_storage_fault_barrier(stage, marker);
            let receipt = service
                .send_channel_message(
                    &actor,
                    SendMessageCommand {
                        request_id: "crash-request",
                        client_message_id: "crash-client",
                        operation_generation: None,
                        conversation_id: None,
                        server_id: "server",
                        channel: "#general",
                        content: "crash-window",
                        content_format: ContentFormat::Plain,
                        reply_to_id: None,
                        attachment_ids: &[],
                        mentions: &[],
                    },
                )
                .await?;
            println!("returned {}", receipt.message_id);
        }
        #[cfg(feature = "storage-fault-injection")]
        "sync-fault" => {
            let client_id = arguments
                .get(3)
                .ok_or_else(|| anyhow::anyhow!("missing client ID"))?;
            let content = arguments
                .get(4)
                .ok_or_else(|| anyhow::anyhow!("missing content"))?;
            verify_storage_profile(&pool).await?;
            let auth = AuthService::new(pool.clone(), "probe-secret".into(), 1);
            let actor = auth.issue_web_session("user").await?.1;
            let service = MessagingService::new(pool.clone(), auth, 4000);
            let mut wakeups = service.subscribe_wakeups();
            arm_storage_sync_fault();
            let failure = service
                .send_channel_message(
                    &actor,
                    SendMessageCommand {
                        request_id: client_id,
                        client_message_id: client_id,
                        operation_generation: None,
                        conversation_id: None,
                        server_id: "server",
                        channel: "#general",
                        content,
                        content_format: ContentFormat::Plain,
                        reply_to_id: None,
                        attachment_ids: &[],
                        mentions: &[],
                    },
                )
                .await
                .expect_err("injected xSync failure must reject the command");
            let sqlite_error_code =
                if let MessagingError::Internal(sqlx::Error::Database(error)) = &failure {
                    error
                        .code()
                        .map_or_else(|| "unknown".into(), |code| code.into_owned())
                } else {
                    anyhow::bail!("sync failure returned unexpected command error: {failure:?}");
                };
            if observed_storage_sync_faults() != 1 {
                anyhow::bail!("production SQLite connection did not observe one xSync fault");
            }
            if wakeups.try_recv().is_ok() {
                anyhow::bail!("failed command emitted a durable-delivery wakeup");
            }
            drop(service);
            pool.close().await;

            let reopened = create_pool(&url).await?;
            verify_storage_profile(&reopened).await?;
            verify_integrity(&reopened).await?;
            let reopened_state = canonical_state(&reopened).await?;
            let absent = (0, 0, 0, 0, 0);
            let complete = (1, 1, 1, 1, 1);
            if reopened_state != absent && reopened_state != complete {
                anyhow::bail!(
                    "failed synchronization left partial canonical state: {reopened_state:?}"
                );
            }
            let committed_before_retry = reopened_state == complete;
            let existing_message_id: Option<String> = if committed_before_retry {
                Some(
                    sqlx::query_scalar(
                        "SELECT canonical_message_id FROM command_receipts \
                         WHERE principal_id='user' AND client_message_id=?",
                    )
                    .bind(client_id)
                    .fetch_one(&reopened)
                    .await?,
                )
            } else {
                None
            };

            let retry_auth = AuthService::new(reopened.clone(), "probe-secret".into(), 1);
            let retry_actor = retry_auth.issue_web_session("user").await?.1;
            let retry_service = MessagingService::new(reopened.clone(), retry_auth, 4000);
            let receipt = retry_service
                .send_channel_message(
                    &retry_actor,
                    SendMessageCommand {
                        request_id: client_id,
                        client_message_id: client_id,
                        operation_generation: None,
                        conversation_id: None,
                        server_id: "server",
                        channel: "#general",
                        content,
                        content_format: ContentFormat::Plain,
                        reply_to_id: None,
                        attachment_ids: &[],
                        mentions: &[],
                    },
                )
                .await?;
            if receipt.replayed != committed_before_retry {
                anyhow::bail!(
                    "canonical retry replay state disagreed with reopened transaction outcome"
                );
            }
            if let Some(existing_message_id) = existing_message_id
                && receipt.message_id != existing_message_id
            {
                anyhow::bail!("canonical retry changed the committed message identity");
            }
            let retry_state = canonical_state(&reopened).await?;
            if retry_state != (1, 1, 1, 1, 1) {
                anyhow::bail!("canonical retry was incomplete: {retry_state:?}");
            }
            println!(
                "sync-fault-pass {} {} {} journal_mode=wal synchronous=full \
                 injected=SQLITE_IOERR_FSYNC sqlite_extended_code={} observed={} outcome={} \
                 reopened_state={}/{}/{}/{}/{}",
                receipt.message_id,
                receipt.sequence,
                receipt.persisted_at,
                sqlite_error_code,
                observed_storage_sync_faults(),
                if committed_before_retry {
                    "complete"
                } else {
                    "absent"
                },
                reopened_state.0,
                reopened_state.1,
                reopened_state.2,
                reopened_state.3,
                reopened_state.4,
            );
        }
        "hold-precommit" => {
            let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
            let conversation: String =
                sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                    .fetch_one(&mut *transaction)
                    .await?;
            let sequence: i64 = sqlx::query_scalar(
                "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
                 WHERE id=? RETURNING next_message_sequence",
            )
            .bind(&conversation)
            .fetch_one(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
                                      conversation_id,conversation_sequence,content_format) \
                 VALUES('precommit','server','channel','user','carmilla','partial',?,?,'plain')",
            )
            .bind(conversation)
            .bind(sequence)
            .execute(&mut *transaction)
            .await?;
            println!("PRECOMMIT");
            std::io::stdout().flush()?;
            tokio::time::sleep(Duration::from_secs(30)).await;
            transaction.commit().await?;
        }
        "state" => {
            let row = sqlx::query(
                "SELECT (SELECT COUNT(*) FROM messages), \
                        (SELECT COUNT(*) FROM command_receipts), \
                        (SELECT COUNT(*) FROM event_log), \
                        (SELECT COUNT(*) FROM delivery_outbox), \
                        (SELECT next_message_sequence FROM conversations WHERE channel_id='channel')",
            )
            .fetch_one(&pool)
            .await?;
            println!(
                "{} {} {} {} {}",
                row.get::<i64, _>(0),
                row.get::<i64, _>(1),
                row.get::<i64, _>(2),
                row.get::<i64, _>(3),
                row.get::<i64, _>(4)
            );
        }
        _ => anyhow::bail!("unknown command"),
    }
    Ok(())
}

#[cfg(feature = "storage-fault-injection")]
async fn canonical_state(pool: &SqlitePool) -> anyhow::Result<(i64, i64, i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM messages), \
                (SELECT COUNT(*) FROM command_receipts), \
                (SELECT COUNT(*) FROM event_log), \
                (SELECT COUNT(*) FROM delivery_outbox), \
                (SELECT next_message_sequence FROM conversations WHERE channel_id='channel')",
    )
    .fetch_one(pool)
    .await?)
}

#[cfg(feature = "storage-fault-injection")]
async fn verify_integrity(pool: &SqlitePool) -> anyhow::Result<()> {
    let result: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(pool)
        .await?;
    if result != "ok" {
        anyhow::bail!("reopened database failed integrity_check: {result}");
    }
    Ok(())
}

#[cfg(feature = "storage-fault-injection")]
async fn verify_storage_profile(pool: &SqlitePool) -> anyhow::Result<()> {
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await?;
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(pool)
        .await?;
    if !journal_mode.eq_ignore_ascii_case("wal") || synchronous != 2 {
        anyhow::bail!(
            "unexpected storage profile: journal_mode={journal_mode}, synchronous={synchronous}"
        );
    }
    Ok(())
}

async fn initialize(pool: &SqlitePool) -> anyhow::Result<()> {
    run_migrations(pool).await?;
    sqlx::query("INSERT INTO users(id,username) VALUES('user','carmilla')")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('user','user','canonical_id')",
    )
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','user','owner')",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)",
    )
    .bind(concord_server::engine::permissions::DEFAULT_EVERYONE.bits() as i64)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(pool)
        .await?;
    Ok(())
}
