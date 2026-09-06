use super::*;

#[test]
fn stopped_operator_admin_credential_migration_and_job_recovery_are_audited() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    let loaded = concord_server::config::ServerConfig::load_for_recovery(config).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        concord_server::db::pool::run_migrations(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users(id,username,is_system_admin) VALUES \
             ('did:plc:old','old',1),('did:plc:new','new',0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES \
             ('old-at','did:plc:old','atproto','did:plc:old'), \
             ('new-at','did:plc:new','atproto','did:plc:new')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth_credentials(id,user_id,kind,scopes,expires_at) \
             VALUES('new-session','did:plc:new','web_session','human',unixepoch()+3600)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let access_hash = hex::encode(Sha256::digest(b"delegated-access"));
        let refresh_hash = hex::encode(Sha256::digest(b"delegated-refresh"));
        sqlx::query(
            "INSERT INTO oauth2_apps( \
               id,name,owner_id,client_secret,redirect_uris,scopes,is_public, \
               client_type,credential_state) \
             VALUES('app','App','did:plc:old','','[\"https://app.example/callback\"]', \
                    'identify',1,'public','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_grants( \
               id,app_id,user_id,server_id,resource_key,scopes,state) \
             VALUES('grant','app','did:plc:new',NULL,'','identify','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_tokens( \
               id,grant_id,token_family_id,access_token_hash,refresh_token_hash,scopes, \
               access_expires_at,refresh_expires_at) \
             VALUES('delegated-token','grant','family',?,?,'identify', \
                    datetime('now','+1 hour'),datetime('now','+1 day'))",
        )
        .bind(access_hash)
        .bind(refresh_hash)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_codes( \
               id,code_hash,app_id,user_id,redirect_uri,scopes,code_challenge, \
               code_challenge_method,expires_at) \
             VALUES('code','code-hash','app','did:plc:new','https://app.example/callback', \
                    'identify','challenge','S256',datetime('now','+5 minutes'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_consent_requests( \
               id_hash,app_id,user_id,redirect_uri,scopes,code_challenge,expires_at) \
             VALUES('consent','app','did:plc:new','https://app.example/callback','identify', \
                    'challenge',datetime('now','+5 minutes'))",
        )
        .execute(&pool)
        .await
        .unwrap();
    });

    let original_config = fs::read_to_string(config).unwrap();
    assert!(original_config.contains("admin_user_ids = []"));
    fs::write(
        config,
        original_config.replace("admin_user_ids = []", "admin_user_ids = [\"did:plc:old\"]"),
    )
    .unwrap();
    let refused = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "admin-transfer",
            "--from-user-id",
            "did:plc:old",
            "--to-user-id",
            "did:plc:new",
            "--reason",
            "planned administrator transfer",
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("remove did:plc:old from admin.admin_user_ids")
    );

    fs::write(config, &original_config).unwrap();
    let transfer = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "admin-transfer",
            "--from-user-id",
            "did:plc:old",
            "--to-user-id",
            "did:plc:new",
            "--reason",
            "planned administrator transfer",
        ])
        .output()
        .unwrap();
    assert!(
        transfer.status.success(),
        "{}",
        String::from_utf8_lossy(&transfer.stderr)
    );
    let inventory = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .arg("admin-inventory")
        .output()
        .unwrap();
    assert!(inventory.status.success());
    assert!(String::from_utf8_lossy(&inventory.stdout).contains("did:plc:new"));

    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let admins: (i64, i64) = sqlx::query_as(
            "SELECT \
             (SELECT is_system_admin FROM users WHERE id='did:plc:old'), \
             (SELECT is_system_admin FROM users WHERE id='did:plc:new')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(admins, (0, 1));
        let current = concord_server::config::ServerConfig::load(config).unwrap();
        assert!(
            !concord_server::config::ensure_configured_admin(
                &pool,
                "did:plc:old",
                &current.admin.admin_user_ids,
            )
            .await
            .unwrap()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT is_system_admin FROM users WHERE id='did:plc:old'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );

        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES('server','Server','did:plc:new')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhooks( \
               id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state) \
             VALUES('hook','server','channel','Hook','outgoing','legacy-token', \
                    'https://receiver.example/hook','did:plc:new','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO external_jobs( \
               id,deduplication_key,operation_type,resource_id,resource_version, \
               destination_grant,payload_json,state,attempt_count,safe_error_code) \
             VALUES('job','job-key','webhook_delivery','delivery',1,'webhook:hook:1', \
                    '{\"channel_id\":\"channel\"}','failed',8,'receiver_unavailable')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhook_deliveries( \
               id,webhook_id,external_job_id,delivery_id,event_type,event_version,payload_json,state) \
             VALUES('delivery-row','hook','job','delivery','webhook_test',1, \
                    '{\"channel_id\":\"channel\"}','failed')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO external_jobs( \
               id,deduplication_key,operation_type,resource_id,resource_version, \
               destination_grant,payload_json,state) \
             VALUES('at-job','at-job-key','atproto_publish','publication',1, \
                    'atproto-user:user:1','{}','failed')",
        )
        .execute(&pool)
        .await
        .unwrap();
    });

    for command in ["migration-status", "migration-apply"] {
        let result = fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg(command)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}: {}",
            command,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let jobs = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["jobs-inspect", "--state", "failed", "--limit", "10"])
        .output()
        .unwrap();
    assert!(jobs.status.success());
    let jobs = String::from_utf8(jobs.stdout).unwrap();
    assert!(jobs.contains("\"id\":\"job\""));
    assert!(!jobs.contains("legacy-token"));
    assert!(!jobs.contains("destination_grant"));
    assert!(!jobs.contains("payload_json"));

    let retry = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["job-retry", "job", "--reason", "receiver repaired"])
        .output()
        .unwrap();
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let at_retry = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["job-retry", "at-job", "--reason", "provider repaired"])
        .output()
        .unwrap();
    assert!(!at_retry.status.success());
    assert!(String::from_utf8_lossy(&at_retry.stderr).contains("atproto-publication-reconcile"));

    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_operator_credential_revoke \
             BEFORE UPDATE OF state ON oauth2_grants \
             WHEN OLD.user_id='did:plc:new' AND NEW.state='revoked' \
             BEGIN SELECT RAISE(ABORT,'injected credential revocation failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
    });
    let failed_credentials = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "credential-revoke-all",
            "--user-id",
            "did:plc:new",
            "--reason",
            "fault injection",
        ])
        .output()
        .unwrap();
    assert!(!failed_credentials.status.success());
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let unchanged: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM auth_credentials WHERE user_id='did:plc:new' AND revoked_at IS NULL), \
             (SELECT count(*) FROM oauth2_tokens WHERE grant_id='grant' AND revoked_at IS NULL), \
             (SELECT count(*) FROM oauth2_grants WHERE id='grant' AND state='active'), \
             (SELECT count(*) FROM oauth2_codes WHERE user_id='did:plc:new' AND consumed_at IS NULL), \
             (SELECT count(*) FROM oauth2_consent_requests WHERE user_id='did:plc:new' AND consumed_at IS NULL), \
             (SELECT count(*) FROM operator_audit_log WHERE action_type='credential_revoke_all')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unchanged, (1, 1, 1, 1, 1, 0));
        sqlx::query("DROP TRIGGER fail_operator_credential_revoke")
            .execute(&pool)
            .await
            .unwrap();
    });

    let credentials = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "credential-revoke-all",
            "--user-id",
            "did:plc:new",
            "--reason",
            "lost browser credential",
        ])
        .output()
        .unwrap();
    assert!(credentials.status.success());
    assert!(String::from_utf8_lossy(&credentials.stdout).contains("credentials_revoked=5"));

    runtime.block_on(async {
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode, header};

        let issuer = restarted_issuer(&loaded).await;
        let access = issuer
            .clone()
            .oneshot(
                Request::get("/api/oauth/userinfo")
                    .header(header::AUTHORIZATION, "Bearer delegated-access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(access.status(), StatusCode::UNAUTHORIZED);

        let refresh_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("client_id", "app")
            .append_pair("refresh_token", "delegated-refresh")
            .finish();
        let refresh = issuer
            .oneshot(
                Request::post("/api/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(refresh_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refresh.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(refresh.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("invalid_grant"));
    });

    let recovered = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "admin-recover",
            "--user-id",
            "did:plc:old",
            "--reason",
            "documented local recovery",
        ])
        .output()
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );

    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let job_states: (String, String) = sqlx::query_as(
            "SELECT \
             (SELECT state FROM external_jobs WHERE id='job'), \
             (SELECT state FROM webhook_deliveries WHERE delivery_id='delivery')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(job_states, ("pending".into(), "pending".into()));
        assert!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT revoked_at FROM auth_credentials WHERE id='new-session'",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
            .is_some()
        );
        let delegated: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT g.state,t.revoked_at,c.consumed_at,r.consumed_at \
                 FROM oauth2_grants g JOIN oauth2_tokens t ON t.grant_id=g.id \
                 JOIN oauth2_codes c ON c.user_id=g.user_id \
                 JOIN oauth2_consent_requests r ON r.user_id=g.user_id \
                 WHERE g.id='grant'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(delegated.0, "revoked");
        assert!(delegated.1.is_some());
        assert!(delegated.2.is_some());
        assert!(delegated.3.is_some());
        let mut actions: Vec<String> =
            sqlx::query_scalar("SELECT action_type FROM operator_audit_log")
                .fetch_all(&pool)
                .await
                .unwrap();
        actions.sort();
        assert_eq!(
            actions,
            vec![
                "admin_recovery",
                "admin_transfer",
                "credential_revoke_all",
                "external_job_retry",
            ]
        );
    });
    fs::remove_dir_all(root).unwrap();
}
