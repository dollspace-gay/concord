use sqlx::SqlitePool;

use crate::db::models::{CreateSlashCommandParams, InteractionRow, SlashCommandRow};

pub async fn create_command(
    pool: &SqlitePool,
    p: &CreateSlashCommandParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO slash_commands (id, bot_user_id, server_id, name, description, options_json)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(p.id)
    .bind(p.bot_user_id)
    .bind(p.server_id)
    .bind(p.name)
    .bind(p.description)
    .bind(p.options_json)
    .execute(pool)
    .await?;
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
        "SELECT * FROM slash_commands WHERE server_id = ? OR server_id IS NULL ORDER BY name",
    )
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
        "INSERT INTO interactions (id, interaction_type, command_id, user_id, server_id, channel_id, data_json)
         VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(p.id)
    .bind(p.interaction_type)
    .bind(p.command_id)
    .bind(p.user_id)
    .bind(p.server_id)
    .bind(p.channel_id)
    .bind(p.data_json)
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
}
