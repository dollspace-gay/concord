use super::*;

use crate::db::pool::{create_pool, run_migrations};

use crate::engine::messaging::{
    ContentFormat, EntityCommand, MessagingService, ReactionCommand, ReadCommand,
    SendMessageCommand,
};

use crate::engine::permissions::DEFAULT_EVERYONE;

async fn fixture() -> (
    SqlitePool,
    AuthService,
    Actor,
    String,
    MessagingService,
    ReplayService,
) {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    for (id, name) in [("user", "carmilla"), ("other", "laura")] {
        sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
            .bind(id)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) \
         VALUES('server','user','owner'),('server','other','member')",
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
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let actor = auth.issue_web_session("user").await.unwrap().1;
    let messaging = MessagingService::new(pool.clone(), auth.clone(), 4000);
    let replay = ReplayService::new(pool.clone(), auth.clone(), "persistent-secret");
    (pool, auth, actor, conversation, messaging, replay)
}

async fn send(
    messaging: &MessagingService,
    actor: &Actor,
    client_id: &str,
    content: &str,
) -> crate::engine::messaging::CommandReceipt {
    messaging
        .send_channel_message(
            actor,
            SendMessageCommand {
                request_id: client_id,
                client_message_id: client_id,
                operation_generation: None,
                conversation_id: None,
                server_id: "server",
                channel: "#general",
                content,
                content_format: ContentFormat::Markdown,
                reply_to_id: None,
                attachment_ids: &[],
                mentions: &[],
            },
        )
        .await
        .unwrap()
}

mod authorization;
mod behavior;
mod lifecycle;
mod messaging;
mod queries;
mod recovery;
mod revocation;
