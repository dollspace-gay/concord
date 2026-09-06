use std::io;
use std::sync::Arc;
use std::time::Duration;

use concord_server::auth::authority::AuthService;
use concord_server::engine::chat_engine::ChatEngine;
use concord_server::irc::connection::handle_irc_connection_until;
use concord_server::irc::framing::{IrcLineDecoder, MAX_LINE_LENGTH};
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio_util::sync::CancellationToken;

async fn write_chunks(mut stream: DuplexStream, chunks: Vec<Vec<u8>>) {
    for chunk in chunks {
        stream.write_all(&chunk).await.unwrap();
        tokio::task::yield_now().await;
    }
    stream.shutdown().await.unwrap();
}

#[tokio::test]
async fn representative_registration_decodes_at_every_split_point() {
    let commands = b"PASS secret\r\nNICK carmilla\r\nUSER carmilla 0 * :Carmilla\r\n";

    for split in 0..=commands.len() {
        let (mut server, mut client) = tokio::io::duplex(1);
        let first = commands[..split].to_vec();
        let second = commands[split..].to_vec();
        let (first_consumed_tx, first_consumed_rx) = tokio::sync::oneshot::channel();
        let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
        let writer = tokio::spawn(async move {
            client.write_all(&first).await.unwrap();
            first_consumed_tx.send(()).unwrap();
            continue_rx.await.unwrap();
            client.write_all(&second).await.unwrap();
            client.shutdown().await.unwrap();
        });
        let mut decoder = IrcLineDecoder::new();

        let release = tokio::spawn(async move {
            first_consumed_rx.await.unwrap();
            tokio::task::yield_now().await;
            continue_tx.send(()).unwrap();
        });

        for expected in [
            "PASS secret",
            "NICK carmilla",
            "USER carmilla 0 * :Carmilla",
        ] {
            let line = tokio::time::timeout(Duration::from_secs(1), decoder.read_line(&mut server))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(line.as_deref(), Some(expected), "split {split}");
        }
        release.await.unwrap();
        writer.await.unwrap();
    }
}

#[tokio::test]
async fn split_utf8_and_multiple_lines_in_one_read_are_preserved() {
    let input = "NICK émilie\r\nUSER émilie 0 * :Émilie 🌙\r\n".as_bytes();
    let split = input.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
    let (mut server, client) = tokio::io::duplex(128);
    let writer = tokio::spawn(write_chunks(
        client,
        vec![input[..split].to_vec(), input[split..].to_vec()],
    ));
    let mut decoder = IrcLineDecoder::new();

    assert_eq!(
        decoder.read_line(&mut server).await.unwrap().as_deref(),
        Some("NICK émilie")
    );
    assert_eq!(
        decoder.read_line(&mut server).await.unwrap().as_deref(),
        Some("USER émilie 0 * :Émilie 🌙")
    );
    assert_eq!(decoder.read_line(&mut server).await.unwrap(), None);
    writer.await.unwrap();
}

#[tokio::test]
async fn clean_eof_and_partial_eof_are_distinct() {
    let (mut clean_server, clean_client) = tokio::io::duplex(16);
    drop(clean_client);
    assert_eq!(
        IrcLineDecoder::new()
            .read_line(&mut clean_server)
            .await
            .unwrap(),
        None
    );

    let (mut partial_server, mut partial_client) = tokio::io::duplex(16);
    partial_client.write_all(b"NI").await.unwrap();
    partial_client.shutdown().await.unwrap();
    let error = IrcLineDecoder::new()
        .read_line(&mut partial_server)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn exact_limit_is_accepted_and_one_byte_over_is_rejected() {
    let accepted = format!("{}\r\n", "a".repeat(MAX_LINE_LENGTH - 2));
    let (mut accepted_server, client) = tokio::io::duplex(MAX_LINE_LENGTH * 2);
    let writer = tokio::spawn(write_chunks(client, vec![accepted.as_bytes().to_vec()]));
    let decoded = IrcLineDecoder::new()
        .read_line(&mut accepted_server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decoded.len(), MAX_LINE_LENGTH - 2);
    writer.await.unwrap();

    let rejected = format!("{}\r\n", "a".repeat(MAX_LINE_LENGTH - 1));
    let (mut rejected_server, client) = tokio::io::duplex(MAX_LINE_LENGTH * 2);
    let writer = tokio::spawn(write_chunks(client, vec![rejected.into_bytes()]));
    let error = IrcLineDecoder::new()
        .read_line(&mut rejected_server)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    writer.await.unwrap();
}

#[tokio::test]
async fn malformed_line_does_not_consume_the_next_complete_line() {
    let (mut server, client) = tokio::io::duplex(64);
    let writer = tokio::spawn(write_chunks(
        client,
        vec![b"NICK \xff\r\nPING :still-here\r\n".to_vec()],
    ));
    let mut decoder = IrcLineDecoder::new();

    assert_eq!(
        decoder.read_line(&mut server).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        decoder.read_line(&mut server).await.unwrap().as_deref(),
        Some("PING :still-here")
    );
    writer.await.unwrap();
}

#[tokio::test]
async fn pending_read_is_cancellation_safe() {
    let (mut server, mut client) = tokio::io::duplex(1);
    let prefix = tokio::spawn(async move {
        client.write_all(b"NI").await.unwrap();
        client
    });
    let mut decoder = IrcLineDecoder::new();

    let cancelled =
        tokio::time::timeout(Duration::from_millis(10), decoder.read_line(&mut server)).await;
    assert!(cancelled.is_err());
    let mut client = prefix.await.unwrap();
    let suffix = tokio::spawn(async move {
        client.write_all(b"CK carmilla\r\n").await.unwrap();
    });

    assert_eq!(
        decoder.read_line(&mut server).await.unwrap().as_deref(),
        Some("NICK carmilla")
    );
    suffix.await.unwrap();
}

#[tokio::test]
async fn cancellation_reaps_connection_when_client_stops_reading() {
    let (server, mut client) = tokio::io::duplex(32);
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let db = concord_server::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    concord_server::db::pool::run_migrations(&db).await.unwrap();
    let auth = AuthService::new(db.clone(), "test-secret".into(), 1);
    let engine = Arc::new(ChatEngine::new(
        db.clone(),
        auth.clone(),
        "test-secret",
        4000,
        100,
    ));
    let task = tokio::spawn(handle_irc_connection_until(
        server,
        "duplex-test".into(),
        engine,
        db,
        auth,
        task_cancel,
    ));

    // Generate output and deliberately leave it unread so the writer blocks.
    client
        .write_all(b"CAP LS\r\nCAP LS\r\nCAP LS\r\nCAP LS\r\n")
        .await
        .unwrap();
    tokio::task::yield_now().await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("connection task leaked behind stalled writer")
        .unwrap();
}
