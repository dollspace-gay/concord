use super::*;

#[tokio::test]
async fn durable_ready_transition_and_checksum() {
    let (d, p, conversation) = fixture().await;
    let mut u = MediaUpload::start(
        p.clone(),
        d.path(),
        StartMedia {
            owner_id: "u",
            intent: MediaIntent::Message {
                conversation_id: conversation.clone(),
            },
            original_filename: "x.txt",
            content_type: "text/plain",
            max_bytes: 8,
            per_user_bytes: 16,
            total_bytes: 64,
        },
    )
    .await
    .unwrap();
    u.write_chunk(b"hello").await.unwrap();
    let ready = u.finish().await.unwrap();
    assert_eq!(ready.file_size, 5);
    assert_eq!(
        tokio::fs::read(d.path().join(ready.storage_key))
            .await
            .unwrap(),
        b"hello"
    );
    let state: (String, String) =
        sqlx::query_as("SELECT media_state,sha256 FROM attachments WHERE id=?")
            .bind(ready.id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(state.0, "ready");
    assert_eq!(state.1, ready.sha256);
}

#[tokio::test]
async fn oversize_upload_is_failed_on_abort() {
    let before = crate::runtime_metrics::snapshot();
    let upload_index = crate::runtime_metrics::Operation::Upload as usize;
    let (d, p, conversation) = fixture().await;
    let mut u = MediaUpload::start(
        p.clone(),
        d.path(),
        StartMedia {
            owner_id: "u",
            intent: MediaIntent::Message {
                conversation_id: conversation.clone(),
            },
            original_filename: "x",
            content_type: "text/plain",
            max_bytes: 3,
            per_user_bytes: 16,
            total_bytes: 64,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        u.write_chunk(b"four").await,
        Err(MediaError::TooLarge)
    ));
    let id = u.id.clone();
    u.abort().await;
    let state: String = sqlx::query_scalar("SELECT media_state FROM attachments WHERE id=?")
        .bind(id)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(state, "failed");
    let after = crate::runtime_metrics::snapshot();
    assert!(after.failed[upload_index] > before.failed[upload_index]);
}

#[tokio::test]
async fn fragmented_upload_does_not_write_progress_for_every_chunk() {
    let (d, p, conversation) = fixture().await;
    sqlx::query("CREATE TABLE progress_updates(count INTEGER NOT NULL)")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("INSERT INTO progress_updates(count) VALUES(0)")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER count_upload_progress AFTER UPDATE OF upload_updated_at ON attachments BEGIN UPDATE progress_updates SET count=count+1; END")
        .execute(&p)
        .await
        .unwrap();
    let mut upload = MediaUpload::start(
        p.clone(),
        d.path(),
        StartMedia {
            owner_id: "u",
            intent: MediaIntent::Message {
                conversation_id: conversation,
            },
            original_filename: "fragments.bin",
            content_type: "application/octet-stream",
            max_bytes: 2_000,
            per_user_bytes: 4_000,
            total_bytes: 8_000,
        },
    )
    .await
    .unwrap();
    for _ in 0..1_000 {
        upload.write_chunk(b"x").await.unwrap();
    }
    let updates: i64 = sqlx::query_scalar("SELECT count FROM progress_updates")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(updates, 0);
    upload.finish().await.unwrap();
    let updates: i64 = sqlx::query_scalar("SELECT count FROM progress_updates")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(
        updates, 1,
        "only the final fencing update should touch progress"
    );
}

#[tokio::test]
async fn timed_out_upload_abort_releases_reservation_and_staging_file() {
    let (d, p, conversation) = fixture().await;
    let upload = MediaUpload::start(
        p.clone(),
        d.path(),
        StartMedia {
            owner_id: "u",
            intent: MediaIntent::Message {
                conversation_id: conversation,
            },
            original_filename: "timeout.bin",
            content_type: "application/octet-stream",
            max_bytes: 8,
            per_user_bytes: 16,
            total_bytes: 64,
        },
    )
    .await
    .unwrap();
    let id = upload.id.clone();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(1),
            std::future::pending::<()>(),
        )
        .await
        .is_err()
    );
    upload.abort().await;
    let row: (String, i64) =
        sqlx::query_as("SELECT media_state,reserved_bytes FROM attachments WHERE id=?")
            .bind(&id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(row, ("failed".into(), 0));
    assert!(!d.path().join("staging").join(format!("{id}.part")).exists());
}

#[tokio::test]
async fn collection_uses_upload_activity_without_rewriting_creation_time() {
    let (d, p, conversation) = fixture().await;
    let upload = MediaUpload::start(
        p.clone(),
        d.path(),
        StartMedia {
            owner_id: "u",
            intent: MediaIntent::Message {
                conversation_id: conversation.clone(),
            },
            original_filename: "x",
            content_type: "text/plain",
            max_bytes: 8,
            per_user_bytes: 16,
            total_bytes: 64,
        },
    )
    .await
    .unwrap();
    let id = upload.id.clone();
    sqlx::query(
        "UPDATE attachments SET created_at='2000-01-01 00:00:00',upload_updated_at=datetime('now') WHERE id=?",
    )
    .bind(&id)
    .execute(&p)
    .await
    .unwrap();

    assert_eq!(collect_expired(&p, d.path(), 60).await.unwrap(), 0);
    let created_at: String = sqlx::query_scalar("SELECT created_at FROM attachments WHERE id=?")
        .bind(&id)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(created_at, "2000-01-01 00:00:00");

    sqlx::query("UPDATE attachments SET upload_updated_at='2000-01-01 00:00:00' WHERE id=?")
        .bind(&id)
        .execute(&p)
        .await
        .unwrap();
    assert_eq!(collect_expired(&p, d.path(), 60).await.unwrap(), 1);
    let state: String = sqlx::query_scalar("SELECT media_state FROM attachments WHERE id=?")
        .bind(&id)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(state, "failed");
}

#[tokio::test]
async fn reconciles_object_renamed_before_metadata_commit() {
    let (d, p, conversation) = fixture().await;
    let mut upload = MediaUpload::start(
        p.clone(),
        d.path(),
        StartMedia {
            owner_id: "u",
            intent: MediaIntent::Message {
                conversation_id: conversation.clone(),
            },
            original_filename: "x",
            content_type: "text/plain",
            max_bytes: 8,
            per_user_bytes: 16,
            total_bytes: 64,
        },
    )
    .await
    .unwrap();
    upload.write_chunk(b"hello").await.unwrap();
    upload.file.flush().await.unwrap();
    upload.file.sync_all().await.unwrap();
    let id = upload.id.clone();
    let final_path = upload.final_path.clone();
    rooted_rename(
        upload.root.clone(),
        upload.staging_path.clone(),
        final_path.clone(),
    )
    .await
    .unwrap();
    drop(upload.file);
    assert_eq!(reconcile_interrupted(&p, d.path()).await.unwrap(), 1);
    let row: (String, i64, String) =
        sqlx::query_as("SELECT media_state,file_size,sha256 FROM attachments WHERE id=?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(row.0, "ready");
    assert_eq!(row.1, 5);
    assert!(!row.2.is_empty());
}

#[test]
fn storage_keys_cannot_escape_the_media_root() {
    assert!(safe_storage_key("objects/ab/id"));
    for key in [
        "../objects/ab/id",
        "objects/../secret",
        "objects/ab/../../secret",
        "/objects/ab/id",
        "objects\\ab\\id",
        "staging/id.part",
    ] {
        assert!(!safe_storage_key(key), "accepted unsafe key {key:?}");
    }
}

#[tokio::test]
async fn historical_import_preserves_provenance_and_switches_only_verified_bytes() {
    let (d, p, conversation) = fixture().await;
    let id = Uuid::new_v4().to_string();
    let (address, server) = http_fixture(b"legacy bytes".to_vec()).await;
    let previous_url = "http://legacy.test/blob";
    sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_cid,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,? ,?,'legacy_external',?,?,1,'pending')")
        .bind(&id).bind("u").bind(&conversation).bind(&id).bind("old.bin").bind("application/octet-stream").bind(12_i64).bind("cid-old").bind(previous_url).execute(&p).await.unwrap();
    sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,previous_cid,expected_size,outcome,reference_outcome) VALUES(?,?,?,?, 'pending','not_checked')")
        .bind(&id).bind(previous_url).bind("cid-old").bind(12_i64).execute(&p).await.unwrap();
    let client = crate::egress::ControlledHttpClient::fixture(address, 64);
    let report = import_legacy_batch(&p, d.path(), &client, 64, 30, 1)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(
        report,
        ImportReport {
            claimed: 1,
            imported: 1,
            unresolved: 0
        }
    );
    let row: (String, String, String, i64) = sqlx::query_as(
        "SELECT media_state,storage_key,sha256,previously_public FROM attachments WHERE id=?",
    )
    .bind(&id)
    .fetch_one(&p)
    .await
    .unwrap();
    assert_eq!(row.0, "ready");
    assert_eq!(
        tokio::fs::read(d.path().join(&row.1)).await.unwrap(),
        b"legacy bytes"
    );
    assert_eq!(row.2, hex::encode(Sha256::digest(b"legacy bytes")));
    assert_eq!(row.3, 1);
    let inventory = external_reference_inventory(&p).await.unwrap();
    assert_eq!(inventory[0].previous_url, previous_url);
    assert_eq!(inventory[0].previous_cid.as_deref(), Some("cid-old"));
    assert_eq!(inventory[0].outcome, "imported");
}

#[tokio::test]
async fn historical_import_records_size_mismatch_without_switching_locator() {
    let (d, p, conversation) = fixture().await;
    let id = Uuid::new_v4().to_string();
    let (address, server) = http_fixture(b"different".to_vec()).await;
    let previous_url = "http://legacy.test/blob";
    sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,? ,?,'legacy_external',?,1,'pending')")
        .bind(&id).bind("u").bind(&conversation).bind(&id).bind("old.bin").bind("application/octet-stream").bind(99_i64).bind(previous_url).execute(&p).await.unwrap();
    sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,99,'pending')").bind(&id).bind(previous_url).execute(&p).await.unwrap();
    let client = crate::egress::ControlledHttpClient::fixture(address, 64);
    let report = import_legacy_batch(&p, d.path(), &client, 64, 30, 1)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(report.unresolved, 1);
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT media_state,storage_key FROM attachments WHERE id=?")
            .bind(&id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(row, ("legacy_external".into(), None));
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM media_import_ledger WHERE attachment_id=?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(outcome, "size_mismatch");
}

#[tokio::test]
async fn historical_import_streams_above_preview_body_limit() {
    let (d, p, conversation) = fixture().await;
    let id = Uuid::new_v4().to_string();
    let body = vec![0x5a; 3 * 1024 * 1024 + 7];
    let size = body.len() as i64;
    let expected_hash = hex::encode(Sha256::digest(&body));
    let (address, server) = http_fixture(body).await;
    let previous_url = "http://legacy.test/large";
    sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,?,?,'legacy_external',?,1,'pending')")
        .bind(&id).bind("u").bind(&conversation).bind(&id).bind("large.bin").bind("application/octet-stream").bind(size).bind(previous_url).execute(&p).await.unwrap();
    sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,?,'pending')").bind(&id).bind(previous_url).bind(size).execute(&p).await.unwrap();
    let client = crate::egress::ControlledHttpClient::fixture(address, 4 * 1024 * 1024);
    let report = import_legacy_batch(&p, d.path(), &client, 4 * 1024 * 1024, 30, 1)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(report.imported, 1);
    let (key, hash): (String, String) =
        sqlx::query_as("SELECT storage_key,sha256 FROM attachments WHERE id=?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(hash, expected_hash);
    assert_eq!(
        std::fs::metadata(d.path().join(key)).unwrap().len(),
        size as u64
    );
}

#[tokio::test]
async fn fragmented_import_has_bounded_lease_writes() {
    let (d, p, conversation) = fixture().await;
    let id = Uuid::new_v4().to_string();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        for _ in 0..500 {
            stream.write_all(b"x").await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    let previous_url = "http://legacy.test/fragments";
    sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,?,500,'legacy_external',?,1,'pending')")
        .bind(&id).bind("u").bind(&conversation).bind(&id).bind("fragments.bin").bind("application/octet-stream").bind(previous_url).execute(&p).await.unwrap();
    sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,500,'pending')")
        .bind(&id).bind(previous_url).execute(&p).await.unwrap();
    sqlx::query("CREATE TABLE lease_updates(count INTEGER NOT NULL)")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("INSERT INTO lease_updates(count) VALUES(0)")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER count_lease_updates AFTER UPDATE OF claim_until ON media_import_ledger WHEN OLD.outcome='importing' AND NEW.outcome='importing' BEGIN UPDATE lease_updates SET count=count+1; END")
        .execute(&p).await.unwrap();

    let client = crate::egress::ControlledHttpClient::fixture(address, 1_000);
    let report = import_legacy_batch(&p, d.path(), &client, 1_000, 30, 1)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(report.imported, 1);
    let renewals: i64 = sqlx::query_scalar("SELECT count FROM lease_updates")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(
        renewals, 1,
        "fragment count must not determine SQLite write count"
    );
}
