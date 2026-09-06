use sqlx::SqlitePool;

use crate::db::models::ChannelRow;

/// Create a thread (stored as a channel row with thread-specific fields).
pub struct CreateThreadParams<'a> {
    pub channel_id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub channel_type: &'a str,
    pub parent_message_id: &'a str,
    pub parent_channel_id: &'a str,
    pub creator_user_id: &'a str,
    pub auto_archive_minutes: i32,
}

pub async fn create_thread(
    pool: &SqlitePool,
    params: &CreateThreadParams<'_>,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    create_thread_in(&mut transaction, params).await?;
    transaction.commit().await
}

pub async fn create_thread_in(
    connection: &mut sqlx::SqliteConnection,
    params: &CreateThreadParams<'_>,
) -> Result<(), sqlx::Error> {
    let inserted = sqlx::query(
        "INSERT INTO channels( \
            id,server_id,name,channel_type,thread_parent_message_id,parent_channel_id, \
            thread_auto_archive_minutes,is_private,thread_last_activity_at,thread_archive_due_at, \
            thread_creator_user_id \
         ) SELECT ?,?,?,?,?,?,?,?,datetime('now'),datetime('now','+' || ? || ' minutes'),? \
           WHERE EXISTS(SELECT 1 FROM messages WHERE id=? AND channel_id=? AND server_id=?)",
    )
    .bind(params.channel_id)
    .bind(params.server_id)
    .bind(params.name)
    .bind(params.channel_type)
    .bind(params.parent_message_id)
    .bind(params.parent_channel_id)
    .bind(params.auto_archive_minutes)
    .bind((params.channel_type == "private_thread") as i32)
    .bind(params.auto_archive_minutes)
    .bind(params.creator_user_id)
    .bind(params.parent_message_id)
    .bind(params.parent_channel_id)
    .bind(params.server_id)
    .execute(&mut *connection)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(sqlx::Error::Protocol(
            "thread parent is outside the requested channel/server".into(),
        ));
    }
    if params.channel_type == "private_thread" {
        sqlx::query("INSERT INTO thread_members(thread_id,user_id) VALUES(?,?)")
            .bind(params.channel_id)
            .bind(params.creator_user_id)
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

/// Archive a thread.
pub async fn archive_thread(pool: &SqlitePool, channel_id: &str) -> Result<(), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    set_thread_archived_in(&mut connection, channel_id, true)
        .await
        .map(|_| ())
}

pub async fn set_thread_archived_in(
    connection: &mut sqlx::SqliteConnection,
    channel_id: &str,
    archived: bool,
) -> Result<i64, sqlx::Error> {
    let result = if archived {
        sqlx::query_scalar(
        "UPDATE channels SET archived=1,thread_archive_reason='manual',thread_archive_due_at=NULL \
             ,thread_state_version=thread_state_version+1 \
         WHERE id=? AND channel_type IN ('public_thread','private_thread') \
         RETURNING thread_state_version",
        )
        .bind(channel_id)
        .fetch_optional(&mut *connection)
        .await?
    } else {
        sqlx::query_scalar(
            "UPDATE channels SET archived=0,thread_archive_reason=NULL, \
                 thread_last_activity_at=datetime('now'), \
                 thread_archive_due_at=datetime('now','+' || thread_auto_archive_minutes || ' minutes'), \
                 thread_state_version=thread_state_version+1 \
             WHERE id=? AND channel_type IN ('public_thread','private_thread') \
             RETURNING thread_state_version",
        )
        .bind(channel_id)
        .fetch_optional(&mut *connection)
        .await?
    };
    result.ok_or_else(|| sqlx::Error::Protocol("thread not found".into()))
}

/// Unarchive a thread.
pub async fn unarchive_thread(pool: &SqlitePool, channel_id: &str) -> Result<(), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    set_thread_archived_in(&mut connection, channel_id, false)
        .await
        .map(|_| ())
}

/// Advance thread activity and its durable inactivity deadline inside the
/// caller's canonical message transaction.
pub async fn record_thread_activity(
    connection: &mut sqlx::SqliteConnection,
    channel_id: &str,
    activity_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE channels SET thread_last_activity_at=?, \
             thread_archive_due_at=datetime(?,'+' || thread_auto_archive_minutes || ' minutes'), \
             thread_state_version=thread_state_version+1 \
         WHERE id=? AND channel_type IN ('public_thread','private_thread') AND archived=0",
    )
    .bind(activity_at)
    .bind(activity_at)
    .bind(channel_id)
    .execute(connection)
    .await?;
    Ok(())
}

/// Archive only threads whose persisted deadline is still due. A concurrent
/// accepted message moves the deadline in the same SQLite write order and
/// prevents this conditional update from archiving active work.
pub async fn archive_due_threads(
    connection: &mut sqlx::SqliteConnection,
    limit: i64,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE channels SET archived=1,thread_archive_reason='inactivity',thread_archive_due_at=NULL \
             ,thread_state_version=thread_state_version+1 \
         WHERE id IN (SELECT id FROM channels \
                      WHERE archived=0 \
                        AND channel_type IN ('public_thread','private_thread') \
                        AND thread_archive_due_at<=datetime('now') \
                      ORDER BY thread_archive_due_at,id LIMIT ?) RETURNING id,thread_state_version",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(connection)
    .await
}

/// Get all threads whose parent message lives in the given channel.
pub async fn get_threads_for_channel(
    pool: &SqlitePool,
    parent_channel_id: &str,
    server_id: &str,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT c.* FROM channels c \
         WHERE c.parent_channel_id = ? AND c.server_id = ? \
         AND c.channel_type IN ('public_thread', 'private_thread')",
    )
    .bind(parent_channel_id)
    .bind(server_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
    use crate::db::queries::messages::{self, InsertMessageParams};
    use crate::db::queries::servers;
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_env(pool: &SqlitePool) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: "u1",
                username: "alice",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u1",
                provider: "github",
                provider_id: "gh-u1",
            },
        )
        .await
        .unwrap();
        servers::create_server(pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
        channels::ensure_channel(pool, "c1", "s1", "#general")
            .await
            .unwrap();
        // Create a parent message for threads
        messages::insert_message(
            pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Parent message",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_create_thread() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_thread(
            &pool,
            &CreateThreadParams {
                channel_id: "t1",
                server_id: "s1",
                name: "Discussion",
                channel_type: "public_thread",
                parent_message_id: "m1",
                parent_channel_id: "c1",
                creator_user_id: "u1",
                auto_archive_minutes: 1440,
            },
        )
        .await
        .unwrap();

        let chan = channels::get_channel(&pool, "t1").await.unwrap();
        assert!(chan.is_some());
        let c = chan.unwrap();
        assert_eq!(c.name, "Discussion");
        assert_eq!(c.channel_type, "public_thread");
        assert_eq!(c.thread_parent_message_id, Some("m1".to_string()));
        assert_eq!(c.thread_auto_archive_minutes, 1440);
        assert_eq!(c.archived, 0);
    }

    #[tokio::test]
    async fn test_archive_and_unarchive_thread() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_thread(
            &pool,
            &CreateThreadParams {
                channel_id: "t1",
                server_id: "s1",
                name: "Thread",
                channel_type: "public_thread",
                parent_message_id: "m1",
                parent_channel_id: "c1",
                creator_user_id: "u1",
                auto_archive_minutes: 60,
            },
        )
        .await
        .unwrap();

        archive_thread(&pool, "t1").await.unwrap();
        let chan = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(chan.archived, 1);

        unarchive_thread(&pool, "t1").await.unwrap();
        let chan = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(chan.archived, 0);
    }

    #[tokio::test]
    async fn accepted_activity_moves_deadline_before_auto_archive_sweep() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_thread(
            &pool,
            &CreateThreadParams {
                channel_id: "t1",
                server_id: "s1",
                name: "Thread",
                channel_type: "public_thread",
                parent_message_id: "m1",
                parent_channel_id: "c1",
                creator_user_id: "u1",
                auto_archive_minutes: 60,
            },
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE channels SET thread_archive_due_at=datetime('now','-1 minute') WHERE id='t1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        record_thread_activity(&mut connection, "t1", "2030-01-01 00:00:00")
            .await
            .unwrap();
        drop(connection);

        let mut connection = pool.acquire().await.unwrap();
        assert!(
            archive_due_threads(&mut connection, 100)
                .await
                .unwrap()
                .is_empty()
        );
        drop(connection);
        let row = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(row.archived, 0);
        assert_eq!(
            row.thread_last_activity_at.as_deref(),
            Some("2030-01-01 00:00:00")
        );

        sqlx::query(
            "UPDATE channels SET thread_archive_due_at=datetime('now','-1 minute') WHERE id='t1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        assert_eq!(
            archive_due_threads(&mut connection, 100).await.unwrap(),
            vec![("t1".to_string(), 3)]
        );
        let row = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(row.archived, 1);
        assert_eq!(row.thread_archive_reason.as_deref(), Some("inactivity"));
    }

    #[tokio::test]
    async fn test_get_threads_for_channel() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_thread(
            &pool,
            &CreateThreadParams {
                channel_id: "t1",
                server_id: "s1",
                name: "Thread 1",
                channel_type: "public_thread",
                parent_message_id: "m1",
                parent_channel_id: "c1",
                creator_user_id: "u1",
                auto_archive_minutes: 60,
            },
        )
        .await
        .unwrap();
        create_thread(
            &pool,
            &CreateThreadParams {
                channel_id: "t2",
                server_id: "s1",
                name: "Thread 2",
                channel_type: "private_thread",
                parent_message_id: "m1",
                parent_channel_id: "c1",
                creator_user_id: "u1",
                auto_archive_minutes: 1440,
            },
        )
        .await
        .unwrap();

        let threads = get_threads_for_channel(&pool, "c1", "s1").await.unwrap();
        assert_eq!(threads.len(), 2);
    }

    #[tokio::test]
    async fn test_no_threads_for_channel() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let threads = get_threads_for_channel(&pool, "c1", "s1").await.unwrap();
        assert!(threads.is_empty());
    }

    #[tokio::test]
    async fn test_private_thread() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_thread(
            &pool,
            &CreateThreadParams {
                channel_id: "t1",
                server_id: "s1",
                name: "Secret Thread",
                channel_type: "private_thread",
                parent_message_id: "m1",
                parent_channel_id: "c1",
                creator_user_id: "u1",
                auto_archive_minutes: 60,
            },
        )
        .await
        .unwrap();

        let chan = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(chan.channel_type, "private_thread");
    }
}
