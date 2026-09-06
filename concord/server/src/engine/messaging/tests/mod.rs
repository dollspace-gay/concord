use super::*;

use crate::db::pool::{create_pool, run_migrations};

use crate::engine::permissions::DEFAULT_EVERYONE;

async fn fixture() -> (SqlitePool, AuthService, Actor, MessagingService) {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('user','carmilla'),('other','laurelai')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES \
         ('user','user','canonical_id'),('other','other','canonical_id'), \
         ('laurelai','other','nickname')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('server','user','owner'),('server','other','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO roles(id,server_id,name,permissions,is_default) \
         VALUES('everyone','server','@everyone',?,1)",
    )
    .bind(DEFAULT_EVERYONE.bits() as i64)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("user").await.unwrap().1;
    let service = MessagingService::new(pool.clone(), auth.clone(), 4000);
    (pool, auth, actor, service)
}

fn command<'a>(
    request_id: &'a str,
    client_message_id: &'a str,
    content: &'a str,
) -> SendMessageCommand<'a> {
    SendMessageCommand {
        request_id,
        client_message_id,
        operation_generation: None,
        conversation_id: None,
        server_id: "server",
        channel: "#general",
        content,
        content_format: ContentFormat::Markdown,
        reply_to_id: None,
        attachment_ids: &[],
        mentions: &[],
    }
}

fn command_in_generation<'a>(
    request_id: &'a str,
    client_message_id: &'a str,
    content: &'a str,
    operation_generation: &'a str,
) -> SendMessageCommand<'a> {
    SendMessageCommand {
        operation_generation: Some(operation_generation),
        ..command(request_id, client_message_id, content)
    }
}

mod behavior;
mod delete_retry_is_canonical_and_new_reaction_on_tombstone_is_rejected;
mod message_event_atomically_enqueues_subscribed_outgoing_webhook;
mod read_state_never_moves_backwards;
mod revocation;
mod validation;
mod webhook_enqueue_failure_rolls_back_canonical_message_and_event;
