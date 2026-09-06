use sqlx::{SqliteConnection, SqlitePool};

use crate::db::models::{CreateSlashCommandParams, InteractionRow, SlashCommandRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionResponseResult {
    Accepted,
    NotFound,
    WrongApplication,
    Expired,
    AlreadyResponded,
}

pub async fn create_command(
    pool: &SqlitePool,
    p: &CreateSlashCommandParams<'_>,
) -> Result<(), sqlx::Error> {
    let inserted = sqlx::query(
        "INSERT INTO slash_commands (id, bot_user_id, server_id, name, description, options_json)
         SELECT ?, ?, ?, ?, ?, ?
         WHERE NOT EXISTS(
             SELECT 1 FROM slash_commands WHERE server_id IS ? AND name = ? COLLATE NOCASE
         )",
    )
    .bind(p.id)
    .bind(p.bot_user_id)
    .bind(p.server_id)
    .bind(p.name)
    .bind(p.description)
    .bind(p.options_json)
    .bind(p.server_id)
    .bind(p.name)
    .execute(pool)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn get_command(
    pool: &SqlitePool,
    command_id: &str,
) -> Result<Option<SlashCommandRow>, sqlx::Error> {
    sqlx::query_as::<_, SlashCommandRow>("SELECT * FROM slash_commands WHERE id = ?")
        .bind(command_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_commands_for_server(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<SlashCommandRow>, sqlx::Error> {
    sqlx::query_as::<_, SlashCommandRow>(
        "SELECT c.* FROM slash_commands c
         JOIN bot_installations i ON i.bot_user_id=c.bot_user_id AND i.server_id=?
         WHERE (c.server_id=? OR c.server_id IS NULL)
           AND i.state='active' AND i.revoked_at IS NULL
           AND (instr(' '||i.granted_scopes||' ',' commands ')>0
                OR instr(' '||i.granted_scopes||' ',' * ')>0)
         ORDER BY c.name,c.id",
    )
    .bind(server_id)
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn list_commands_for_bot(
    pool: &SqlitePool,
    bot_user_id: &str,
) -> Result<Vec<SlashCommandRow>, sqlx::Error> {
    sqlx::query_as::<_, SlashCommandRow>(
        "SELECT * FROM slash_commands WHERE bot_user_id = ? ORDER BY name",
    )
    .bind(bot_user_id)
    .fetch_all(pool)
    .await
}

pub async fn update_command(
    pool: &SqlitePool,
    command_id: &str,
    description: &str,
    options_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE slash_commands SET description = ?, options_json = ? WHERE id = ?")
        .bind(description)
        .bind(options_json)
        .bind(command_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_command(pool: &SqlitePool, command_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM slash_commands WHERE id = ?")
        .bind(command_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_interaction(
    pool: &SqlitePool,
    p: &crate::db::models::CreateInteractionParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO interactions
         (id,interaction_type,command_id,user_id,server_id,channel_id,data_json,
          application_user_id,expires_at,response_state)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending')",
    )
    .bind(p.id)
    .bind(p.interaction_type)
    .bind(p.command_id)
    .bind(p.user_id)
    .bind(p.server_id)
    .bind(p.channel_id)
    .bind(p.data_json)
    .bind(p.application_user_id)
    .bind(p.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_interaction(
    pool: &SqlitePool,
    interaction_id: &str,
) -> Result<Option<InteractionRow>, sqlx::Error> {
    sqlx::query_as::<_, InteractionRow>("SELECT * FROM interactions WHERE id = ?")
        .bind(interaction_id)
        .fetch_optional(pool)
        .await
}

pub async fn mark_interaction_responded(
    pool: &SqlitePool,
    interaction_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE interactions SET responded = 1 WHERE id = ?")
        .bind(interaction_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Accept the first response from the application that owns an interaction.
/// The guarded update is the replay/expiry authority; the follow-up read only
/// selects a stable rejection reason for the caller.
pub async fn accept_interaction_response(
    connection: &mut SqliteConnection,
    interaction_id: &str,
    application_user_id: &str,
    response_message_id: Option<&str>,
    ephemeral_response_json: Option<&str>,
    response_expires_at: Option<&str>,
) -> Result<InteractionResponseResult, sqlx::Error> {
    let changed = sqlx::query(
        "UPDATE interactions
         SET responded=1,response_state='responded',response_version=response_version+1,
             response_message_id=?,ephemeral_response_json=?,response_expires_at=?,
             responded_at=datetime('now')
         WHERE id=? AND application_user_id=? AND response_state='pending'
           AND expires_at IS NOT NULL AND expires_at>datetime('now')",
    )
    .bind(response_message_id)
    .bind(ephemeral_response_json)
    .bind(response_expires_at)
    .bind(interaction_id)
    .bind(application_user_id)
    .execute(&mut *connection)
    .await?;
    if changed.rows_affected() == 1 {
        return Ok(InteractionResponseResult::Accepted);
    }

    let state: Option<(Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT application_user_id,expires_at,response_state FROM interactions WHERE id=?",
    )
    .bind(interaction_id)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(match state {
        None => InteractionResponseResult::NotFound,
        Some((owner, _, _)) if owner.as_deref() != Some(application_user_id) => {
            InteractionResponseResult::WrongApplication
        }
        Some((_, _, state)) if state != "pending" => InteractionResponseResult::AlreadyResponded,
        Some((_, _, _)) => InteractionResponseResult::Expired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
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
        // Insert bot user directly (create_bot_user references non-existent columns)
        sqlx::query("INSERT INTO users (id, username, is_bot) VALUES ('bot1', 'MyBot', 1)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('s1','bot1','member')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) VALUES('install','bot1','s1','u1','commands','active')")
            .execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_create_and_get_command() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "Pong!",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        let cmd = get_command(&pool, "cmd1").await.unwrap();
        assert!(cmd.is_some());
        let c = cmd.unwrap();
        assert_eq!(c.name, "ping");
        assert_eq!(c.description, "Pong!");
    }

    #[tokio::test]
    async fn test_list_commands_for_server() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "Pong!",
                options_json: "[]",
            },
        )
        .await
        .unwrap();
        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd2",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "help",
                description: "Help!",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        let cmds = list_commands_for_server(&pool, "s1").await.unwrap();
        assert_eq!(cmds.len(), 2);
        // Ordered by name
        assert_eq!(cmds[0].name, "help");
        assert_eq!(cmds[1].name, "ping");
    }

    #[tokio::test]
    async fn server_command_names_are_unambiguous_and_revoked_installs_are_hidden() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot2','OtherBot',1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('s1','bot2','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) VALUES('install2','bot2','s1','u1','commands','active')")
            .execute(&pool).await.unwrap();
        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "first",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "First",
                options_json: "[]",
            },
        )
        .await
        .unwrap();
        assert!(
            create_command(
                &pool,
                &CreateSlashCommandParams {
                    id: "ambiguous",
                    bot_user_id: "bot2",
                    server_id: Some("s1"),
                    name: "PING",
                    description: "Second",
                    options_json: "[]",
                },
            )
            .await
            .is_err()
        );
        assert_eq!(
            list_commands_for_server(&pool, "s1").await.unwrap().len(),
            1
        );
        sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now'),authorization_version=authorization_version+1 WHERE id='install'")
            .execute(&pool).await.unwrap();
        assert!(
            list_commands_for_server(&pool, "s1")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_list_commands_for_bot() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "Pong!",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        let cmds = list_commands_for_bot(&pool, "bot1").await.unwrap();
        assert_eq!(cmds.len(), 1);
    }

    #[tokio::test]
    async fn test_update_command() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "Old desc",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        update_command(&pool, "cmd1", "New desc", "[{\"name\":\"arg\"}]")
            .await
            .unwrap();

        let cmd = get_command(&pool, "cmd1").await.unwrap().unwrap();
        assert_eq!(cmd.description, "New desc");
        assert_eq!(cmd.options_json, "[{\"name\":\"arg\"}]");
    }

    #[tokio::test]
    async fn test_delete_command() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "Pong!",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        delete_command(&pool, "cmd1").await.unwrap();

        let cmd = get_command(&pool, "cmd1").await.unwrap();
        assert!(cmd.is_none());
    }

    #[tokio::test]
    async fn test_create_and_get_interaction() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: Some("s1"),
                name: "ping",
                description: "Pong!",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        create_interaction(
            &pool,
            &crate::db::models::CreateInteractionParams {
                id: "int1",
                interaction_type: "slash_command",
                command_id: Some("cmd1"),
                user_id: "u1",
                server_id: "s1",
                channel_id: "c1",
                data_json: "{}",
                application_user_id: "bot1",
                expires_at: "2999-01-01 00:00:00",
            },
        )
        .await
        .unwrap();

        let interaction = get_interaction(&pool, "int1").await.unwrap();
        assert!(interaction.is_some());
        let i = interaction.unwrap();
        assert_eq!(i.interaction_type, "slash_command");
        assert_eq!(i.responded, 0);

        mark_interaction_responded(&pool, "int1").await.unwrap();
        let i = get_interaction(&pool, "int1").await.unwrap().unwrap();
        assert_eq!(i.responded, 1);
    }

    #[tokio::test]
    async fn test_global_command_appears_in_server_list() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        // Global command (server_id = NULL)
        create_command(
            &pool,
            &CreateSlashCommandParams {
                id: "cmd1",
                bot_user_id: "bot1",
                server_id: None,
                name: "global-cmd",
                description: "Global",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        // list_commands_for_server includes global commands (server_id IS NULL)
        let cmds = list_commands_for_server(&pool, "s1").await.unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "global-cmd");
    }

    #[tokio::test]
    async fn interaction_response_is_owned_expiring_and_first_writer_wins() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        sqlx::query(
            "INSERT INTO interactions
             (id,interaction_type,user_id,server_id,channel_id,data_json,
              application_user_id,expires_at,response_state)
             VALUES('active','button','u1','s1','c1','{}','bot1',datetime('now','+5 minutes'),'pending'),
                   ('expired','button','u1','s1','c1','{}','bot1',datetime('now','-1 second'),'pending')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut transaction = pool.begin().await.unwrap();
        assert_eq!(
            accept_interaction_response(&mut transaction, "active", "u1", None, None, None)
                .await
                .unwrap(),
            InteractionResponseResult::WrongApplication
        );
        assert_eq!(
            accept_interaction_response(&mut transaction, "expired", "bot1", None, None, None)
                .await
                .unwrap(),
            InteractionResponseResult::Expired
        );
        assert_eq!(
            accept_interaction_response(&mut transaction, "active", "bot1", None, None, None)
                .await
                .unwrap(),
            InteractionResponseResult::Accepted
        );
        transaction.rollback().await.unwrap();

        let mut transaction = pool.begin().await.unwrap();
        assert_eq!(
            accept_interaction_response(&mut transaction, "active", "bot1", None, None, None)
                .await
                .unwrap(),
            InteractionResponseResult::Accepted
        );
        transaction.commit().await.unwrap();
        let mut transaction = pool.begin().await.unwrap();
        assert_eq!(
            accept_interaction_response(&mut transaction, "active", "bot1", None, None, None)
                .await
                .unwrap(),
            InteractionResponseResult::AlreadyResponded
        );
        transaction.rollback().await.unwrap();
        let version: i64 =
            sqlx::query_scalar("SELECT response_version FROM interactions WHERE id='active'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(version, 1);
    }
}
