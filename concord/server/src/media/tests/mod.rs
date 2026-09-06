use super::*;
use tokio::io::AsyncReadExt;

use crate::db::pool::{create_pool, run_migrations};

use tokio::net::TcpListener;

async fn http_fixture(body: Vec<u8>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).await;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
    });
    (address, task)
}

async fn fixture() -> (tempfile::TempDir, SqlitePool, String) {
    let d = tempfile::tempdir().unwrap();
    let p = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&p).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('u','u')")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('s','s','u')")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('c','s','#c')")
        .execute(&p)
        .await
        .unwrap();
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='c'")
            .fetch_one(&p)
            .await
            .unwrap();
    (d, p, conversation)
}

mod behavior;
mod lifecycle;
mod recovery;
mod revocation;
