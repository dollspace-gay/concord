use sqlx::SqlitePool;

use crate::db::models::{CreateServerEventParams, EventRsvpRow, ServerEventRow};

pub async fn create_event(
    pool: &SqlitePool,
    params: &CreateServerEventParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO server_events (id, server_id, name, description, channel_id, start_time, end_time, image_url, created_by) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(params.id)
    .bind(params.server_id)
    .bind(params.name)
    .bind(params.description)
    .bind(params.channel_id)
    .bind(params.start_time)
    .bind(params.end_time)
    .bind(params.image_url)
    .bind(params.created_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_event(
    pool: &SqlitePool,
    event_id: &str,
) -> Result<Option<ServerEventRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerEventRow>("SELECT * FROM server_events WHERE id = ?")
        .bind(event_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_server_events(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ServerEventRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerEventRow>(
        "SELECT * FROM server_events WHERE server_id = ? ORDER BY start_time ASC",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn update_event_status(
    pool: &SqlitePool,
    event_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE server_events SET status = ?, updated_at = datetime('now') WHERE id = ?")
        .bind(status)
        .bind(event_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_event(pool: &SqlitePool, event_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM server_events WHERE id = ?")
        .bind(event_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_rsvp(
    pool: &SqlitePool,
    event_id: &str,
    user_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO event_rsvps (event_id, user_id, status) VALUES (?, ?, ?) \
         ON CONFLICT(event_id, user_id) DO UPDATE SET status = excluded.status",
    )
    .bind(event_id)
    .bind(user_id)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_rsvp(
    pool: &SqlitePool,
    event_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM event_rsvps WHERE event_id = ? AND user_id = ?")
        .bind(event_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_rsvps(
    pool: &SqlitePool,
    event_id: &str,
) -> Result<Vec<EventRsvpRow>, sqlx::Error> {
    sqlx::query_as::<_, EventRsvpRow>(
        "SELECT * FROM event_rsvps WHERE event_id = ? ORDER BY created_at",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
}

pub async fn get_rsvp_count(pool: &SqlitePool, event_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM event_rsvps WHERE event_id = ?")
        .bind(event_id)
        .fetch_one(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::servers;
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_server(pool: &SqlitePool) {
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
    }

    fn event_params<'a>(id: &'a str) -> CreateServerEventParams<'a> {
        CreateServerEventParams {
            id,
            server_id: "s1",
            name: "Game Night",
            description: Some("Let's play!"),
            channel_id: None,
            start_time: "2027-01-15T20:00:00Z",
            end_time: Some("2027-01-15T23:00:00Z"),
            image_url: None,
            created_by: "u1",
        }
    }

    #[tokio::test]
    async fn test_create_and_get_event() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_event(&pool, &event_params("e1")).await.unwrap();

        let ev = get_event(&pool, "e1").await.unwrap();
        assert!(ev.is_some());
        let e = ev.unwrap();
        assert_eq!(e.name, "Game Night");
        assert_eq!(e.status, "scheduled");
    }

    #[tokio::test]
    async fn test_list_server_events() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_event(&pool, &event_params("e1")).await.unwrap();
        create_event(
            &pool,
            &CreateServerEventParams {
                id: "e2",
                server_id: "s1",
                name: "Movie Night",
                description: None,
                channel_id: None,
                start_time: "2027-02-01T20:00:00Z",
                end_time: None,
                image_url: None,
                created_by: "u1",
            },
        )
        .await
        .unwrap();

        let events = list_server_events(&pool, "s1").await.unwrap();
        assert_eq!(events.len(), 2);
        // Ordered by start_time ASC
        assert_eq!(events[0].name, "Game Night");
        assert_eq!(events[1].name, "Movie Night");
    }

    #[tokio::test]
    async fn test_update_event_status() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_event(&pool, &event_params("e1")).await.unwrap();

        update_event_status(&pool, "e1", "active").await.unwrap();

        let ev = get_event(&pool, "e1").await.unwrap().unwrap();
        assert_eq!(ev.status, "active");
    }

    #[tokio::test]
    async fn test_delete_event() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_event(&pool, &event_params("e1")).await.unwrap();

        delete_event(&pool, "e1").await.unwrap();

        let ev = get_event(&pool, "e1").await.unwrap();
        assert!(ev.is_none());
    }

    #[tokio::test]
    async fn test_rsvp_operations() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_event(&pool, &event_params("e1")).await.unwrap();

        // Create second user
        users::create_with_oauth(
            &pool,
            &CreateOAuthUser {
                user_id: "u2",
                username: "bob",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u2",
                provider: "github",
                provider_id: "gh-u2",
            },
        )
        .await
        .unwrap();

        set_rsvp(&pool, "e1", "u1", "going").await.unwrap();
        set_rsvp(&pool, "e1", "u2", "interested").await.unwrap();

        let rsvps = get_rsvps(&pool, "e1").await.unwrap();
        assert_eq!(rsvps.len(), 2);

        let count = get_rsvp_count(&pool, "e1").await.unwrap();
        assert_eq!(count, 2);

        // Update RSVP (upsert)
        set_rsvp(&pool, "e1", "u2", "going").await.unwrap();
        let rsvps = get_rsvps(&pool, "e1").await.unwrap();
        assert_eq!(rsvps.len(), 2);
        let u2_rsvp = rsvps.iter().find(|r| r.user_id == "u2").unwrap();
        assert_eq!(u2_rsvp.status, "going");

        // Remove RSVP
        remove_rsvp(&pool, "e1", "u1").await.unwrap();
        let count = get_rsvp_count(&pool, "e1").await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent_event() {
        let pool = setup_db().await;
        let ev = get_event(&pool, "nosuch").await.unwrap();
        assert!(ev.is_none());
    }
}
