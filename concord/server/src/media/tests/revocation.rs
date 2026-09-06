use super::*;

#[tokio::test]
async fn expired_import_attempt_cannot_overwrite_newer_verified_bytes() {
    let (d, p, conversation) = fixture().await;
    let id = Uuid::new_v4().to_string();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let first_headers_sent = Arc::new(tokio::sync::Notify::new());
    let release_first_body = Arc::new(tokio::sync::Notify::new());
    let server = {
        let first_headers_sent = first_headers_sent.clone();
        let release_first_body = release_first_body.clone();
        tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = first.read(&mut request).await;
            first
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            first_headers_sent.notify_one();

            let (mut second, _) = listener.accept().await.unwrap();
            let _ = second.read(&mut request).await;
            second
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nnewer",
                )
                .await
                .unwrap();

            release_first_body.notified().await;
            first.write_all(b"older").await.unwrap();
        })
    };
    let previous_url = "http://legacy.test/raced";
    sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,?,5,'legacy_external',?,1,'pending')")
        .bind(&id).bind("u").bind(&conversation).bind(&id).bind("raced.bin").bind("application/octet-stream").bind(previous_url).execute(&p).await.unwrap();
    sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,5,'pending')")
        .bind(&id).bind(previous_url).execute(&p).await.unwrap();
    let client = crate::egress::ControlledHttpClient::fixture(address, 64);

    let stale = {
        let p = p.clone();
        let root = d.path().to_owned();
        let client = client.clone();
        tokio::spawn(async move { import_legacy_batch(&p, &root, &client, 64, 1, 1).await })
    };
    first_headers_sent.notified().await;
    sqlx::query("UPDATE media_import_ledger SET claim_until=datetime('now','-1 second') WHERE attachment_id=? AND outcome='importing'")
        .bind(&id)
        .execute(&p)
        .await
        .unwrap();
    let winner_client = crate::egress::ControlledHttpClient::fixture(address, 64);
    let winner = import_legacy_batch(&p, d.path(), &winner_client, 64, 30, 1)
        .await
        .unwrap();
    assert_eq!(winner.imported, 1);
    release_first_body.notify_one();
    let stale = stale.await.unwrap().unwrap();
    server.await.unwrap();
    assert_eq!(stale.unresolved, 1);

    let (key, hash): (String, String) =
        sqlx::query_as("SELECT storage_key,sha256 FROM attachments WHERE id=?")
            .bind(&id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(
        tokio::fs::read(d.path().join(&key)).await.unwrap(),
        b"newer"
    );
    assert_eq!(hash, hex::encode(Sha256::digest(b"newer")));
    let object_count = std::fs::read_dir(d.path().join("objects"))
        .unwrap()
        .map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap().count())
        .sum::<usize>();
    assert_eq!(object_count, 1, "the stale attempt object must be removed");
}
