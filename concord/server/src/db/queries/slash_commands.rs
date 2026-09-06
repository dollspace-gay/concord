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
mod tests;
