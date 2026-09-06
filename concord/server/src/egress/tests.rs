use super::destination_policy::{checked_addresses, is_public_ip};
use super::*;
use std::{collections::VecDeque, sync::Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
#[test]
fn address_matrix_and_ports() {
    for a in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.0.0.9",
        "192.168.0.1",
        "198.18.0.1",
        "224.0.0.1",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "100::1",
        "2001:2::1",
        "2001:db8::1",
        "2001:20::1",
        "2002::1",
        "3fff::1",
        "5f00::1",
        "fc00::1",
        "fe80::1",
        "ff00::1",
    ] {
        assert!(!is_public_ip(a.parse().unwrap()), "accepted {a}");
    }
    assert!(is_public_ip("1.1.1.1".parse().unwrap()));
    assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    assert!(
        validate_destination(
            &Url::parse("https://example.com:8443").unwrap(),
            DestinationPolicy::PublicHttps
        )
        .is_err()
    );
}
#[test]
fn mixed_dns_answer_rejected() {
    assert!(
        checked_addresses(vec![
            "1.1.1.1:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap()
        ])
        .is_err()
    );
}
#[derive(Debug)]
struct FixtureResolver(Mutex<VecDeque<SocketAddr>>);
impl Resolve for FixtureResolver {
    fn resolve(&self, _: Name) -> Resolving {
        let a = self.0.lock().unwrap().pop_front();
        Box::pin(async move { Ok(Box::new(vec![a.ok_or("DNS exhausted")?].into_iter()) as Addrs) })
    }
}
fn client(a: Vec<SocketAddr>, max: usize) -> ControlledHttpClient {
    ControlledHttpClient::build(
        2,
        max,
        Duration::from_secs(1),
        Duration::from_secs(2),
        1,
        DestinationPolicy::FixtureHttp,
        Arc::new(FixtureResolver(Mutex::new(a.into()))),
    )
    .unwrap()
}
async fn server(response: Vec<u8>) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut b = [0; 2048];
        let _ = s.read(&mut b).await;
        s.write_all(&response).await.unwrap();
    });
    a
}
#[tokio::test]
async fn chunked_body_is_bounded() {
    let a = server(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n"
            .to_vec(),
    )
    .await;
    let c = client(vec![a], 7);
    let r = c
        .request(
            Method::GET,
            Url::parse("http://fixture.test/data").unwrap(),
            RedirectPolicy::Reject,
        )
        .unwrap();
    assert_eq!(
        c.send(r).await.unwrap_err(),
        EgressError::ResponseTooLarge { limit: 7 }
    );
}
#[tokio::test]
async fn credential_redirect_never_contacts_target() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ta = target.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: http://target.test:{}/x\r\nContent-Length: 0\r\n\r\n",
        ta.port()
    )
    .into_bytes();
    let first = server(response).await;
    let c = client(vec![first, ta], 128);
    let origin = Url::parse(&format!("http://fixture.test:{}/", first.port())).unwrap();
    let r = c
        .request(Method::GET, origin.clone(), RedirectPolicy::FollowSafeGet)
        .unwrap()
        .credentials_for(&origin)
        .unwrap();
    assert_eq!(
        c.send(r).await.unwrap_err(),
        EgressError::InvalidDestination("redirect is forbidden")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn saturated_general_capacity_does_not_starve_reserved_oauth() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut held, _) = listener.accept().await.unwrap();
        let mut request = [0; 2048];
        let _ = held.read(&mut request).await;
        held.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
            .await
            .unwrap();
        let (mut oauth, _) = listener.accept().await.unwrap();
        let _ = oauth.read(&mut request).await;
        oauth
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let general = ControlledHttpClient::fixture_with_inflight(address, 16, 1);
    let oauth = ControlledHttpClient::fixture_with_inflight(address, 16, 1);
    let url = Url::parse("http://fixture.test/resource").unwrap();
    let held = general
        .send_streaming(
            general
                .request(Method::GET, url.clone(), RedirectPolicy::Reject)
                .unwrap(),
        )
        .await
        .unwrap();
    let response = oauth
        .send(
            oauth
                .request(Method::GET, url, RedirectPolicy::Reject)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.body, b"ok");
    drop(held);
}
#[tokio::test]
async fn operator_allowlist_reaches_only_the_exact_private_origin() {
    let address = server(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()).await;
    let origin = Url::parse(&format!("http://{address}/")).unwrap();
    let client =
        ControlledHttpClient::scoped_origins(std::slice::from_ref(&origin), 64, 1).unwrap();
    let allowed = origin.join("/receiver").unwrap();
    let response = client
        .send(
            client
                .request(Method::POST, allowed, RedirectPolicy::Reject)
                .unwrap()
                .body(b"fixture".to_vec()),
        )
        .await
        .unwrap();
    assert_eq!(response.body, b"ok");

    let denied = Url::parse(&format!("http://127.0.0.1:{}/receiver", address.port() + 1)).unwrap();
    assert_eq!(
        client
            .request(Method::POST, denied, RedirectPolicy::Reject)
            .err()
            .unwrap(),
        EgressError::InvalidDestination("origin is not allowlisted")
    );
}
#[test]
fn errors_are_sanitized() {
    for e in [
        EgressError::Connect,
        EgressError::Timeout,
        EgressError::Protocol,
    ] {
        assert!(!e.to_string().contains("http"));
    }
}
#[test]
fn malformed_provider_json_diagnostic_never_echoes_values() {
    #[derive(Debug, serde::Deserialize)]
    struct Expected {
        #[allow(dead_code)]
        count: u64,
    }
    let sentinel = "DO-NOT-LOG-PROVIDER-SECRET";
    let error = parse_provider_json::<Expected>(format!(r#"{{"count":"{sentinel}"}}"#).as_bytes())
        .unwrap_err()
        .to_string();
    assert!(!error.contains(sentinel));
    assert!(error.contains("line"));
    assert!(error.contains("column"));
}
