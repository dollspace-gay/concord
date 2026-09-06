//! Create the disposable million-message dataset and credential inventories used
//! by the dedicated-host load/recovery qualification.

use concord_server::auth::authority::{AuthService, UserId};
use concord_server::db::pool::{create_pool, run_migrations};
use concord_server::engine::integrations::{CreateWebhook, IntegrationService};
use concord_server::engine::write_admission::WriteAdmission;
use concord_server::secrets::SecretVault;
use futures_util::stream::{self, StreamExt};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

const SERVER_ID: &str = "load-server";
const SERVER_ALIAS: &str = "load";

fn env_count(
    name: &str,
    default: usize,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    Ok(std::env::var(name).map_or_else(|_| Ok(default), |value| value.parse())?)
}

fn private_json(
    path: &Path,
    value: &Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}

fn channel_id(index: usize) -> String {
    format!("load-channel-{index:02}")
}

fn channel_name(index: usize) -> String {
    format!("#load{index:02}")
}

fn channel_alias(index: usize) -> String {
    format!("load{index:02}")
}

fn irc_alias(index: usize) -> String {
    format!("#{SERVER_ALIAS}/{}", channel_alias(index))
}

fn assigned_channels(index: usize, channel_count: usize) -> Vec<usize> {
    let stride = (channel_count / 5).max(1);
    (0..channel_count.min(5))
        .map(|offset| (index + offset * stride) % channel_count)
        .collect()
}

async fn insert_identity(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    username: &str,
    role: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
        .bind(id)
        .bind(username)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES(?,?,'nickname')")
        .bind(username)
        .bind(id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,?)")
        .bind(SERVER_ID)
        .bind(id)
        .bind(role)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO user_default_servers(user_id,server_id) VALUES(?,?)")
        .bind(id)
        .bind(SERVER_ID)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn seed_messages(
    pool: &SqlitePool,
    message_count: usize,
    channel_count: usize,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "WITH digits(value) AS (VALUES(0),(1),(2),(3),(4),(5),(6),(7),(8),(9)), \
         numbers(value) AS ( \
           SELECT a.value + 10*b.value + 100*c.value + 1000*d.value + \
                  10000*e.value + 100000*f.value + 1 \
           FROM digits a CROSS JOIN digits b CROSS JOIN digits c \
           CROSS JOIN digits d CROSS JOIN digits e CROSS JOIN digits f \
         ) \
         INSERT INTO messages( \
           id,server_id,channel_id,sender_id,sender_nick,content,created_at, \
           conversation_id,conversation_sequence,content_format \
         ) \
         SELECT printf('load-seed-message-%07d',numbers.value),?,channels.id, \
                'load-web-000','loadweb000', \
                CASE WHEN (numbers.value-1)%50000=0 \
                     THEN 'qualification-stable-search result ' || numbers.value \
                     ELSE 'qualification history message ' || numbers.value END, \
                datetime('2024-01-01','+' || numbers.value || ' seconds'), \
                conversations.id,CAST((numbers.value-1)/? AS INTEGER)+1,'plain' \
         FROM numbers \
         JOIN channels ON channels.id=printf('load-channel-%02d',(numbers.value-1)%?) \
         JOIN conversations ON conversations.channel_id=channels.id \
         WHERE numbers.value<=?",
    )
    .bind(SERVER_ID)
    .bind(channel_count as i64)
    .bind(channel_count as i64)
    .bind(message_count as i64)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE conversations SET next_message_sequence=( \
           SELECT COALESCE(MAX(conversation_sequence),0) FROM messages \
           WHERE messages.conversation_id=conversations.id \
         ) WHERE server_id=?",
    )
    .bind(SERVER_ID)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let database_url = std::env::var("CONCORD_QUAL_SEED_DATABASE_URL")?;
    let jwt_secret = std::env::var("CONCORD_QUAL_SEED_JWT_SECRET")?;
    let external_key_file = PathBuf::from(std::env::var("CONCORD_QUAL_SEED_EXTERNAL_KEY_FILE")?);
    let output_dir = PathBuf::from(std::env::var("CONCORD_QUAL_SEED_OUTPUT_DIR")?);
    let irc_sessions = env_count("CONCORD_QUAL_SEED_IRC_SESSIONS", 800)?;
    let web_sessions = env_count("CONCORD_QUAL_SEED_WEB_SESSIONS", 200)?;
    let channel_count = env_count("CONCORD_QUAL_SEED_CHANNELS", 50)?;
    let message_count = env_count("CONCORD_QUAL_SEED_MESSAGES", 1_000_000)?;
    let profile = std::env::var("CONCORD_QUAL_SEED_PROFILE").unwrap_or_else(|_| "full".into());
    match profile.as_str() {
        "full"
            if irc_sessions == 800
                && web_sessions == 200
                && channel_count == 50
                && message_count >= 1_000_000 => {}
        "full" => {
            return Err("full qualification seed requires exactly 800 IRC sessions, 200 web sessions, 50 channels, and at least one million messages".into());
        }
        "bounded"
            if irc_sessions >= 1
                && web_sessions >= 2
                && channel_count >= 1
                && message_count >= channel_count => {}
        "bounded" => return Err("bounded qualification seed counts are invalid".into()),
        _ => return Err("CONCORD_QUAL_SEED_PROFILE must be full or bounded".into()),
    }
    std::fs::create_dir_all(&output_dir)?;
    if std::fs::read_dir(&output_dir)?.next().is_some() {
        return Err("qualification seed output directory must be empty".into());
    }

    let options = SqliteConnectOptions::from_str(&database_url)?;
    let database_path = options.get_filename();
    if database_path.as_os_str().is_empty() || database_path == Path::new(":memory:") {
        return Err("qualification seed requires a fresh file-backed SQLite database".into());
    }
    let _reservation = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(database_path)
        .map_err(|error| format!("qualification database path must be fresh: {error}"))?;
    let pool = create_pool(&database_url).await?;
    run_migrations(&pool).await?;

    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO users(id,username,is_system_admin) \
         VALUES('load-web-000','loadweb000',1)",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('loadweb000','load-web-000','nickname')")
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO servers(id,name,owner_id) VALUES(?,'Load qualification','load-web-000')",
    )
    .bind(SERVER_ID)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO server_aliases(alias,server_id) VALUES(?,?)")
        .bind(SERVER_ALIAS)
        .bind(SERVER_ID)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES(?,'load-web-000','owner')",
    )
    .bind(SERVER_ID)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO user_default_servers(user_id,server_id) VALUES('load-web-000',?)")
        .bind(SERVER_ID)
        .execute(&mut *transaction)
        .await?;
    for index in 0..channel_count {
        let id = channel_id(index);
        sqlx::query("INSERT INTO channels(id,server_id,name,is_default) VALUES(?,?,?,?)")
            .bind(&id)
            .bind(SERVER_ID)
            .bind(channel_name(index))
            .bind(i64::from(index == 0))
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES(?,?,?)")
            .bind(SERVER_ID)
            .bind(channel_alias(index))
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
    }
    for index in 0..irc_sessions {
        insert_identity(
            &mut transaction,
            &format!("load-irc-{index:04}"),
            &format!("loadirc{index:04}"),
            "member",
        )
        .await?;
    }
    for index in 1..web_sessions {
        insert_identity(
            &mut transaction,
            &format!("load-web-{index:03}"),
            &format!("loadweb{index:03}"),
            "member",
        )
        .await?;
    }
    transaction.commit().await?;

    for web_index in 0..web_sessions {
        for assigned in assigned_channels(irc_sessions + web_index, channel_count) {
            sqlx::query("INSERT INTO channel_members(channel_id,user_id) VALUES(?,?)")
                .bind(channel_id(assigned))
                .bind(format!("load-web-{web_index:03}"))
                .execute(&pool)
                .await?;
        }
    }

    seed_messages(&pool, message_count, channel_count).await?;

    let auth = AuthService::new(pool.clone(), jwt_secret, 24);
    let mut web_inventory = Vec::with_capacity(web_sessions);
    let mut metrics_session = String::new();
    let mut owner_actor = None;
    for web_index in 0..web_sessions {
        let user_id = format!("load-web-{web_index:03}");
        let (cookie, actor) = auth.issue_web_session(&user_id).await?;
        if web_index == 0 {
            metrics_session.clone_from(&cookie);
            owner_actor = Some(actor);
        }
        let assigned = assigned_channels(irc_sessions + web_index, channel_count);
        let mut subscriptions = Vec::with_capacity(assigned.len());
        let mut aliases = Vec::with_capacity(assigned.len());
        for channel_index in assigned {
            let conversation_id: String =
                sqlx::query("SELECT id FROM conversations WHERE channel_id=?")
                    .bind(channel_id(channel_index))
                    .fetch_one(&pool)
                    .await?
                    .get(0);
            subscriptions.push(conversation_id);
            aliases.push(irc_alias(channel_index));
        }
        web_inventory.push(json!({
            "cookie": cookie,
            "subscriptions": subscriptions,
            "channels": aliases,
            "server_id": SERVER_ID,
        }));
    }

    let issued = stream::iter(0..irc_sessions)
        .map(|index| {
            let auth = auth.clone();
            async move {
                let user_id = UserId::from_stored(format!("load-irc-{index:04}"))?;
                let credential = auth
                    .issue_irc_token(&user_id, Some("load qualification"))
                    .await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>((index, credential.secret))
            }
        })
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
    let mut irc_tokens = Vec::with_capacity(irc_sessions);
    for result in issued {
        irc_tokens.push(result?);
    }
    irc_tokens.sort_by_key(|(index, _)| *index);
    let irc_tokens = irc_tokens
        .into_iter()
        .map(|(_, secret)| secret)
        .collect::<Vec<_>>();

    let integrations = IntegrationService::new(
        pool.clone(),
        auth,
        WriteAdmission::new(pool.clone()),
        Arc::new(SecretVault::load(&external_key_file)?),
    );
    let provider_webhook = integrations
        .create_webhook(
            &owner_actor.ok_or("owner actor was not issued")?,
            CreateWebhook {
                server_id: SERVER_ID,
                channel_id: &channel_id(0),
                name: "Qualification provider failure",
                webhook_type: "outgoing",
                url: Some("https://provider-failure.invalid/concord"),
            },
        )
        .await?;

    let channels = (0..channel_count).map(irc_alias).collect::<Vec<_>>();
    private_json(&output_dir.join("irc-tokens.json"), &json!(irc_tokens))?;
    private_json(&output_dir.join("web-sessions.json"), &json!(web_inventory))?;
    private_json(&output_dir.join("channel-inventory.json"), &json!(channels))?;
    private_json(
        &output_dir.join("query-plan.json"),
        &json!([{
            "session_index": 0,
            "server_id": SERVER_ID,
            "channel": channel_name(0),
            "query": "qualification-stable-search",
            "expected_total": message_count.div_ceil(50_000),
            "history_min_count": 50,
            "page_size": 10,
            "interval_seconds": 2.0,
        }]),
    )?;
    private_json(
        &output_dir.join("permission-race-plan.json"),
        &json!({
            "session_index": 1,
            "server_id": SERVER_ID,
            "channel": channel_name(0),
            "denied_statuses": [403, 404],
        }),
    )?;
    private_json(
        &output_dir.join("seed-result.json"),
        &json!({
            "server_id": SERVER_ID,
            "irc_sessions": irc_sessions,
            "web_sessions": web_sessions,
            "channels": channel_count,
            "seeded_messages": message_count,
            "metrics_session": metrics_session,
            "provider_failure_webhook_id": provider_webhook.row.id,
        }),
    )?;
    pool.close().await;
    println!(
        "seeded_messages={message_count} irc_sessions={irc_sessions} web_sessions={web_sessions} channels={channel_count}"
    );
    Ok(())
}
