//! Seed a disposable database and issue real web-session cookies for browser tests.
//!
//! This binary is intentionally separate from the production server CLI. The
//! authenticated browser runner invokes it only against a fresh temporary DB.

use concord_server::auth::authority::{AuthService, UserId};
use concord_server::db::pool::{create_pool, run_migrations};
use concord_server::engine::integrations::{CreateWebhook, IntegrationService};
use concord_server::engine::write_admission::WriteAdmission;
use concord_server::secrets::SecretVault;
use sqlx::sqlite::SqliteConnectOptions;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("CONCORD_FIXTURE_DATABASE_URL")?;
    let jwt_secret = std::env::var("CONCORD_FIXTURE_JWT_SECRET")?;
    let external_key_file = std::env::var("CONCORD_FIXTURE_EXTERNAL_KEY_FILE")?;
    let options = SqliteConnectOptions::from_str(&database_url)?;
    let database_path = options.get_filename();
    if database_path.as_os_str().is_empty() || database_path == std::path::Path::new(":memory:") {
        return Err("browser fixture requires a fresh file-backed SQLite URL".into());
    }
    // Reserve exactly the decoded path SQLx will open. create_new is atomic, so
    // direct fixture invocation cannot overwrite an existing DB through a
    // percent-encoded spelling or an exists/open race.
    let _reservation = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(database_path)
        .map_err(|error| format!("browser fixture database path must be fresh: {error}"))?;
    let pool = create_pool(&database_url).await?;
    run_migrations(&pool).await?;
    let existing_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await?;
    if existing_users != 0 {
        return Err("browser fixture database is not empty".into());
    }

    for (id, username, is_bot) in [
        ("browser-alice", "alice", false),
        ("browser-bob", "bob", false),
        ("browser-helper-bot", "helper-bot", true),
        ("browser-wrong-bot", "wrong-bot", true),
    ] {
        sqlx::query("INSERT OR IGNORE INTO users(id,username,is_bot) VALUES(?,?,?)")
            .bind(id)
            .bind(username)
            .bind(is_bot)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO user_aliases(alias,user_id,alias_kind) VALUES(?,?,'nickname')",
        )
        .bind(username)
        .bind(id)
        .execute(&pool)
        .await?;
    }
    sqlx::query("INSERT OR IGNORE INTO servers(id,name,owner_id) VALUES('browser-server','Browser fixture','browser-alice')")
        .execute(&pool).await?;
    sqlx::query("INSERT OR IGNORE INTO server_members(server_id,user_id,role) VALUES('browser-server','browser-alice','owner'),('browser-server','browser-bob','member')")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('browser-server','browser-helper-bot','member')")
        .execute(&pool).await?;
    sqlx::query("INSERT OR IGNORE INTO channels(id,server_id,name,is_default) VALUES('browser-general','browser-server','#general',1)")
        .execute(&pool).await?;
    sqlx::query("INSERT OR IGNORE INTO channel_members(channel_id,user_id) VALUES('browser-general','browser-alice'),('browser-general','browser-bob')")
        .execute(&pool).await?;
    sqlx::query(
        "INSERT INTO server_aliases(alias,server_id) VALUES('browser-fixture','browser-server')",
    )
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES('browser-server','general','browser-general')")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO user_default_servers(user_id,server_id) VALUES('browser-alice','browser-server'),('browser-bob','browser-server')")
        .execute(&pool).await?;
    let channel_conversation_id: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='browser-general'")
            .fetch_one(&pool)
            .await?;
    let historical_non_uuid_message_id = "legacy/message:alpha;42";
    let historical_padded_message_id = "  padded historical message id  ";
    let historical_long_message_id = format!(" historical:{} ", "界".repeat(180));
    for (id, content, created_at, sequence) in [
        (
            historical_non_uuid_message_id,
            "historical legacy timestamp",
            "2024-01-02 03:04:05.123456",
            1_i64,
        ),
        (
            historical_padded_message_id,
            "historical offset timestamp",
            "2024-01-02T03:04:06.654321-05:00",
            2_i64,
        ),
        (
            historical_long_message_id.as_str(),
            "historical long unicode identifier",
            "2024-01-02T08:04:07.987654321Z",
            3_i64,
        ),
    ] {
        sqlx::query(
            "INSERT INTO messages(\
                 id,server_id,channel_id,sender_id,sender_nick,content,created_at,\
                 conversation_id,conversation_sequence,content_format\
             ) VALUES(?,?,?,?,?,?,?,?,?,'legacy_unknown')",
        )
        .bind(id)
        .bind("browser-server")
        .bind("browser-general")
        .bind("browser-bob")
        .bind("bob")
        .bind(content)
        .bind(created_at)
        .bind(&channel_conversation_id)
        .bind(sequence)
        .execute(&pool)
        .await?;
    }
    sqlx::query("UPDATE conversations SET next_message_sequence=3 WHERE id=?")
        .bind(&channel_conversation_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO conversations(id,kind,next_message_sequence) VALUES('browser-dm','direct',1)",
    )
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO direct_conversation_pairs(conversation_id,lower_user_id,upper_user_id) VALUES('browser-dm','browser-alice','browser-bob')")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO conversation_participants(conversation_id,user_id) VALUES('browser-dm','browser-alice'),('browser-dm','browser-bob')")
        .execute(&pool).await?;
    sqlx::query(
        "INSERT INTO messages(id,sender_id,sender_nick,content,target_user_id,conversation_id,conversation_sequence,content_format) \
         VALUES('10000000-0000-4000-8000-000000000001','browser-bob','bob','offline hello','browser-alice','browser-dm',1,'plain')",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) \
         VALUES('browser-install','browser-helper-bot','browser-server','browser-alice','commands messages','active')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO bot_ownership(bot_user_id,owner_user_id,repair_required) \
         VALUES('browser-helper-bot','browser-alice',0),('browser-wrong-bot','browser-alice',0)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO slash_commands(id,bot_user_id,server_id,name,description,options_json) \
         VALUES('browser-command','browser-helper-bot','browser-server','journey','Exercise bot interactions','[]')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO interactions(id,interaction_type,user_id,server_id,channel_id,data_json, \
          application_user_id,expires_at,response_state) \
         VALUES('browser-expired','slash_command','browser-alice','browser-server','browser-general', \
          '{}','browser-helper-bot',datetime('now','-1 minute'),'pending')",
    )
    .execute(&pool)
    .await?;

    let auth = AuthService::new(pool.clone(), jwt_secret, 1);
    let (alice, alice_actor) = auth.issue_web_session("browser-alice").await?;
    let (alice_revoke, _) = auth.issue_web_session("browser-alice").await?;
    let (bob, _) = auth.issue_web_session("browser-bob").await?;
    let helper_bot_credential = auth
        .issue_bot_token(
            &UserId::from_stored("browser-helper-bot")?,
            "browser fixture",
            "bot commands messages",
        )
        .await?;
    let limited_helper_bot_credential = auth
        .issue_bot_token(
            &UserId::from_stored("browser-helper-bot")?,
            "browser limited fixture",
            "bot commands",
        )
        .await?;
    let wrong_bot_credential = auth
        .issue_bot_token(
            &UserId::from_stored("browser-wrong-bot")?,
            "browser fixture",
            "bot commands",
        )
        .await?;
    let bob_irc = auth
        .issue_irc_token(
            &UserId::from_stored("browser-bob")?,
            Some("browser fixture"),
        )
        .await?
        .secret;
    let alice_irc = auth
        .issue_irc_token(
            &UserId::from_stored("browser-alice")?,
            Some("IRC qualification fixture"),
        )
        .await?
        .secret;
    let integrations = IntegrationService::new(
        pool.clone(),
        auth.clone(),
        WriteAdmission::new(pool.clone()),
        std::sync::Arc::new(SecretVault::load(std::path::Path::new(&external_key_file))?),
    );
    let provider_failure_webhook = integrations
        .create_webhook(
            &alice_actor,
            CreateWebhook {
                server_id: "browser-server",
                channel_id: "browser-general",
                name: "Qualification provider failure",
                webhook_type: "outgoing",
                url: Some("https://provider-failure.invalid/concord"),
            },
        )
        .await?;
    println!(
        "{}",
        serde_json::json!({
            "alice": alice, "alice_revoke": alice_revoke, "alice_irc": alice_irc,
            "bob": bob, "bob_irc": bob_irc,
            "helper_bot": helper_bot_credential.secret,
            "helper_bot_token_id": helper_bot_credential.token_id,
            "limited_helper_bot": limited_helper_bot_credential.secret,
            "wrong_bot": wrong_bot_credential.secret,
            "historical_non_uuid_message_id": historical_non_uuid_message_id,
            "historical_padded_message_id": historical_padded_message_id,
            "historical_long_message_id": historical_long_message_id,
            "browser_conversation_id": channel_conversation_id,
            "provider_failure_webhook_id": provider_failure_webhook.row.id,
        })
    );
    Ok(())
}
