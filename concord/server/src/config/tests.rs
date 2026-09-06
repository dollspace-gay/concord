use super::*;
use uuid::Uuid;

fn fixture() -> PathBuf {
    let root = env::temp_dir().join(format!("concord-config-{}", Uuid::new_v4()));
    ServerConfig::initialize(root.join("concord.toml")).unwrap();
    root
}

fn load_without_env(path: &Path) -> Result<ServerConfig, ConfigError> {
    ServerConfig::load_with_env(path, |_| None)
}

#[test]
fn explicit_initialization_persists_private_secret_and_valid_config() {
    let root = fixture();
    let config_path = root.join("concord.toml");
    let first_secret = fs::read_to_string(root.join("data/secrets/jwt.key")).unwrap();
    let config = load_without_env(&config_path).unwrap();
    assert_eq!(first_secret, config.auth.jwt_secret);
    assert!(first_secret.len() >= 64);
    assert!(ServerConfig::initialize(&config_path).is_err());
    assert_eq!(
        first_secret,
        fs::read_to_string(root.join("data/secrets/jwt.key")).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_invalid_numeric_environment_instead_of_falling_back() {
    let root = fixture();
    let error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
        (name == "MAX_FILE_SIZE_MB").then(|| "many".into())
    })
    .err()
    .expect("invalid number must fail");
    assert!(error.to_string().contains("storage.max_file_size_mb"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_sample_secret_without_echoing_it() {
    let root = fixture();
    let sample = "change-me-to-a-random-secret";
    let error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
        (name == "JWT_SECRET").then(|| sample.into())
    })
    .err()
    .expect("sample secret must fail");
    assert!(error.to_string().contains("auth.jwt_secret"));
    assert!(!error.to_string().contains(sample));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_public_url_prefix_trick_and_partial_tls() {
    let root = fixture();
    let prefix_error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
        (name == "PUBLIC_URL").then(|| "http://localhost.attacker.example".into())
    })
    .err()
    .expect("prefix trick must fail");
    assert!(!is_loopback_public_url("http://localhost.attacker.example"));
    assert!(prefix_error.to_string().contains("auth.public_url"));
    let cert = root.join("certificate.pem");
    fs::write(&cert, "certificate").unwrap();
    let tls_error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
        (name == "IRC_TLS_CERT").then(|| cert.display().to_string())
    })
    .err()
    .expect("partial TLS must fail");
    assert!(tls_error.to_string().contains("irc_tls_cert"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_storage_path() {
    let root = fixture();
    let error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
        (name == "MEDIA_DIR").then(|| root.join("missing").display().to_string())
    })
    .err()
    .expect("missing media dir must fail");
    assert!(error.to_string().contains("storage.media_dir"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_toml_never_echoes_secret_source_text() {
    let root = env::temp_dir().join(format!("concord-config-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("concord.toml");
    let sentinel = "DO-NOT-LOG-THIS-SECRET";
    fs::write(&path, format!("[auth]\njwt_secret = \"{sentinel}\n")).unwrap();

    let error = load_without_env(&path)
        .err()
        .expect("malformed TOML must fail");
    let diagnostics = format!("{error}\n{error:?}");
    assert!(!diagnostics.contains(sentinel));
    assert!(diagnostics.contains("line"));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn stable_id_admin_allowlist_applies_without_username_matching() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE users (id TEXT PRIMARY KEY, username TEXT NOT NULL, is_system_admin INTEGER NOT NULL DEFAULT 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('did:plc:stable','mutable-handle')")
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        !ensure_configured_admin(&pool, "mutable-handle", &["did:plc:stable".into()])
            .await
            .unwrap()
    );
    assert!(
        ensure_configured_admin(&pool, "did:plc:stable", &["did:plc:stable".into()])
            .await
            .unwrap()
    );
    let is_admin: bool =
        sqlx::query_scalar("SELECT is_system_admin FROM users WHERE id='did:plc:stable'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(is_admin);
}
