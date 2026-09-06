use super::*;
use atproto_identity::key::{KeyType, generate_key};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn accepted_mutation_with_lost_response_is_uncertain_and_not_authentication() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = std::sync::Arc::new(AtomicUsize::new(0));
    let observed = accepted.clone();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let read = socket.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST "));
        observed.fetch_add(1, Ordering::SeqCst);
        // The remote accepted the mutation and then lost the response.
    });
    let transport = crate::egress::ControlledHttpClient::fixture(address, 1024);
    let key = generate_key(KeyType::P256Private).unwrap();
    let result = do_dpop_request(&DpopRequest {
        transport: &transport,
        key: &key,
        access_token: "access-token",
        method: "POST",
        url: "http://fixture.test/xrpc/com.atproto.repo.createRecord",
        body: Some(br#"{"repo":"did:example:alice"}"#),
        content_type: "application/json",
    })
    .await;
    assert!(matches!(result, Err(PdsRequestError::Uncertain(_))));
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refresh_coordinator_serializes_the_same_account() {
    let first = account_refresh_lock("did:example:alice").unwrap();
    let second = account_refresh_lock("did:example:alice").unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    let held = first.lock().await;
    let waiting = tokio::spawn(async move {
        let _guard = second.lock().await;
    });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    drop(held);
    waiting.await.unwrap();
}

#[tokio::test]
async fn refresh_uses_bound_origin_and_yields_to_concurrent_reauthentication() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0_u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(bytes).unwrap();
        assert!(request.starts_with("POST /bound-token HTTP/1.1"));
        request_seen_tx.send(()).unwrap();
        respond_rx.await.unwrap();
        let body = r#"{"access_token":"stale-refresh-access","refresh_token":"stale-refresh-token","expires_in":3600}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });

    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    users::create_with_oauth(
        &pool,
        &users::CreateOAuthUser {
            user_id: "alice",
            username: "alice",
            email: None,
            avatar_url: None,
            oauth_id: "oauth-alice",
            provider: "atproto",
            provider_id: "did:example:alice",
        },
    )
    .await
    .unwrap();
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([37_u8; 32])).unwrap();
    let vault = crate::secrets::SecretVault::load(key_file.path()).unwrap();
    let old_dpop = generate_key(KeyType::P256Private).unwrap();
    let old_dpop_json = serde_json::to_string(&jwk::generate(&old_dpop).unwrap()).unwrap();
    let old = users::AtprotoCredentials {
        did: "did:example:alice".into(),
        access_token: "old-access".into(),
        refresh_token: "old-refresh".into(),
        dpop_private_key: old_dpop_json,
        pds_url: "https://old-pds.example".into(),
        authorization_issuer: "http://fixture.test".into(),
        token_endpoint: "http://fixture.test/bound-token".into(),
        token_expires_at: "2026-01-01T00:00:00Z".into(),
        credential_version: 0,
    };
    users::store_atproto_credentials_encrypted(&pool, &vault, "alice", &old)
        .await
        .unwrap();
    let old = users::get_atproto_credentials_encrypted(&pool, &vault, "alice")
        .await
        .unwrap()
        .unwrap();
    let transport = crate::egress::ControlledHttpClient::fixture(address, 4096);
    let signing_key = generate_key(KeyType::P256Private).unwrap();
    let session = PdsSession {
        transport: &transport,
        pool: &pool,
        vault: &vault,
        user_id: "alice",
        signing_key: &signing_key,
        client_id: "https://concord.example/oauth/client-metadata.json",
        redirect_uri: "https://concord.example/oauth/atproto/callback",
    };
    let refreshed = refresh_access_token(&session, &old, &old_dpop);
    let replace = async {
        request_seen_rx.await.unwrap();
        let new_dpop = generate_key(KeyType::P256Private).unwrap();
        let replacement = users::AtprotoCredentials {
            did: "did:example:alice".into(),
            access_token: "reauth-access".into(),
            refresh_token: "reauth-refresh".into(),
            dpop_private_key: serde_json::to_string(&jwk::generate(&new_dpop).unwrap()).unwrap(),
            pds_url: "https://new-pds.example".into(),
            authorization_issuer: "https://new-issuer.example".into(),
            token_endpoint: "https://new-issuer.example/token".into(),
            token_expires_at: "2027-01-01T00:00:00Z".into(),
            credential_version: 0,
        };
        users::store_atproto_credentials_encrypted(&pool, &vault, "alice", &replacement)
            .await
            .unwrap();
        respond_tx.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(refreshed, replace);
    let result = result.unwrap();
    assert_eq!(result.access_token, "reauth-access");
    assert_eq!(result.refresh_token, "reauth-refresh");
    assert_eq!(result.pds_url, "https://new-pds.example");
    assert_eq!(result.authorization_issuer, "https://new-issuer.example");
    assert_eq!(result.token_endpoint, "https://new-issuer.example/token");
    assert_ne!(result.dpop_private_key, old.dpop_private_key);
    server.await.unwrap();
}
