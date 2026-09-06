//! Controlled outbound HTTP with validated destinations and resource bounds.
use reqwest::{
    Method, StatusCode, Url,
    dns::{Addrs, Name, Resolve, Resolving},
    header::{HeaderMap, HeaderName, HeaderValue, LOCATION},
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedirectPolicy {
    Reject,
    FollowSafeGet,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Origin {
    scheme: String,
    host: String,
    port: u16,
}
impl Origin {
    fn from_url(url: &Url) -> Result<Self, EgressError> {
        Ok(Self {
            scheme: url.scheme().into(),
            host: url
                .host_str()
                .ok_or(EgressError::InvalidDestination("host is required"))?
                .to_ascii_lowercase(),
            port: url
                .port_or_known_default()
                .ok_or(EgressError::InvalidDestination("port is required"))?,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum EgressError {
    InvalidDestination(&'static str),
    InvalidRequest(&'static str),
    AdmissionTimeout,
    Connect,
    Timeout,
    Protocol,
    ResponseTooLarge { limit: usize },
}
impl fmt::Display for EgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDestination(r) => write!(f, "invalid egress destination: {r}"),
            Self::InvalidRequest(r) => write!(f, "invalid egress request: {r}"),
            Self::AdmissionTimeout => f.write_str("egress admission timed out"),
            Self::Connect => f.write_str("controlled request connection failed"),
            Self::Timeout => f.write_str("controlled request timed out"),
            Self::Protocol => f.write_str("controlled request protocol failure"),
            Self::ResponseTooLarge { limit } => {
                write!(f, "response exceeds the {limit}-byte limit")
            }
        }
    }
}
impl std::error::Error for EgressError {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ProviderJsonError {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ProviderJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid provider JSON at line {}, column {}",
            self.line, self.column
        )
    }
}

impl std::error::Error for ProviderJsonError {}

pub fn parse_provider_json<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, ProviderJsonError> {
    serde_json::from_slice(bytes).map_err(|error| ProviderJsonError {
        line: error.line(),
        column: error.column(),
    })
}

/// Process-wide outbound capacity classes. Provider login and operator recovery
/// retain capacity even when user-triggered previews saturate their own budget.
#[derive(Clone)]
pub struct EgressServices {
    pub general: ControlledHttpClient,
    pub imports: ControlledHttpClient,
    pub oauth: ControlledHttpClient,
    pub admin: ControlledHttpClient,
    profile_sync_endpoint: Url,
}

impl EgressServices {
    pub fn internet() -> Result<Self, EgressError> {
        Self::internet_with_admin_origins(&[])
    }

    pub fn internet_with_admin_origins(admin_origins: &[String]) -> Result<Self, EgressError> {
        let client = |max_response_bytes, max_in_flight| {
            ControlledHttpClient::with_limits(
                5,
                max_response_bytes,
                Duration::from_secs(10),
                Duration::from_secs(30),
                max_in_flight,
            )
        };
        Ok(Self {
            general: client(2 * 1024 * 1024, 16)?,
            imports: client(128 * 1024 * 1024, 4)?,
            oauth: client(2 * 1024 * 1024, 4)?,
            admin: if admin_origins.is_empty() {
                client(2 * 1024 * 1024, 2)?
            } else {
                let parsed = admin_origins
                    .iter()
                    .map(|origin| {
                        let url = Url::parse(origin).map_err(|_| {
                            EgressError::InvalidDestination("invalid allowlisted origin")
                        })?;
                        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
                            return Err(EgressError::InvalidDestination(
                                "allowlist entry must be an origin",
                            ));
                        }
                        Ok(url)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                ControlledHttpClient::scoped_origins(&parsed, 2 * 1024 * 1024, 2)?
            },
            profile_sync_endpoint: Url::parse(
                "https://public.api.bsky.app/xrpc/app.bsky.actor.getProfile",
            )
            .expect("static Bluesky profile endpoint is valid"),
        })
    }

    pub fn profile_sync_endpoint(&self) -> &Url {
        &self.profile_sync_endpoint
    }

    /// Build egress services pinned to a controlled HTTP provider fixture.
    #[cfg(feature = "browser-fixtures")]
    #[doc(hidden)]
    pub fn profile_fixture(address: SocketAddr) -> Self {
        let client = ControlledHttpClient::fixture(address, 2 * 1024 * 1024);
        Self {
            general: client.clone(),
            imports: client.clone(),
            oauth: client.clone(),
            admin: client,
            profile_sync_endpoint: Url::parse(&format!(
                "http://provider.fixture:{}/xrpc/app.bsky.actor.getProfile",
                address.port()
            ))
            .expect("fixture profile endpoint is valid"),
        }
    }
}
fn classify(e: &reqwest::Error) -> EgressError {
    if e.is_timeout() {
        EgressError::Timeout
    } else if e.is_connect() {
        EgressError::Connect
    } else {
        EgressError::Protocol
    }
}

#[derive(Debug)]
pub struct EgressResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub final_url: Url,
    pub body: Vec<u8>,
}

pub struct EgressStream {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub final_url: Url,
    response: reqwest::Response,
    max_response_bytes: usize,
    received: usize,
    deadline: tokio::time::Instant,
    _permit: OwnedSemaphorePermit,
    _host_permit: OwnedSemaphorePermit,
}
impl EgressStream {
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, EgressError> {
        let chunk = tokio::time::timeout_at(self.deadline, self.response.chunk())
            .await
            .map_err(|_| EgressError::Timeout)?
            .map_err(|error| classify(&error))?;
        let Some(chunk) = chunk else {
            return Ok(None);
        };
        self.received =
            self.received
                .checked_add(chunk.len())
                .ok_or(EgressError::ResponseTooLarge {
                    limit: self.max_response_bytes,
                })?;
        if self.received > self.max_response_bytes {
            return Err(EgressError::ResponseTooLarge {
                limit: self.max_response_bytes,
            });
        }
        Ok(Some(chunk.to_vec()))
    }
}

/// Opaque request. The underlying Reqwest builder cannot escape this module.
pub struct EgressRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
    redirect: RedirectPolicy,
    credential_origin: Option<Origin>,
}
impl EgressRequest {
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }
    /// Restricts credentials to the exact initial origin and disables redirects.
    pub fn credentials_for(mut self, origin: &Url) -> Result<Self, EgressError> {
        self.credential_origin = Some(Origin::from_url(origin)?);
        Ok(self)
    }
}

#[derive(Clone)]
enum DestinationPolicy {
    PublicHttps,
    ScopedOrigins(Arc<HashSet<Origin>>),
    #[cfg(any(test, feature = "browser-fixtures"))]
    FixtureHttp,
}

#[derive(Clone)]
pub struct ControlledHttpClient {
    client: reqwest::Client,
    max_redirects: usize,
    max_response_bytes: usize,
    admission_timeout: Duration,
    operation_timeout: Duration,
    permits: Arc<Semaphore>,
    per_host_limit: usize,
    host_permits: Arc<Mutex<HashMap<Origin, Arc<Semaphore>>>>,
    policy: DestinationPolicy,
}
impl ControlledHttpClient {
    pub fn scoped_origins(
        origins: &[Url],
        max_response_bytes: usize,
        max_in_flight: usize,
    ) -> Result<Self, EgressError> {
        if origins.is_empty() {
            return Err(EgressError::InvalidRequest("allowlist is empty"));
        }
        let allowed = origins
            .iter()
            .map(Origin::from_url)
            .collect::<Result<HashSet<_>, _>>()?;
        Self::build(
            0,
            max_response_bytes,
            Duration::from_secs(5),
            Duration::from_secs(30),
            max_in_flight,
            DestinationPolicy::ScopedOrigins(Arc::new(allowed)),
            Arc::new(SystemDnsResolver),
        )
    }
    fn host_semaphore(&self, origin: Origin) -> Result<Arc<Semaphore>, EgressError> {
        let mut permits = self
            .host_permits
            .lock()
            .map_err(|_| EgressError::Protocol)?;
        if let Some(existing) = permits.get(&origin) {
            return Ok(existing.clone());
        }
        if permits.len() >= 256 {
            return Err(EgressError::AdmissionTimeout);
        }
        let created = Arc::new(Semaphore::new(self.per_host_limit));
        permits.insert(origin, created.clone());
        Ok(created)
    }
    pub fn internet() -> Result<Self, EgressError> {
        Self::with_limits(
            5,
            2 * 1024 * 1024,
            Duration::from_secs(10),
            Duration::from_secs(30),
            32,
        )
    }
    pub fn with_limits(
        max_redirects: usize,
        max_response_bytes: usize,
        connect_timeout: Duration,
        request_timeout: Duration,
        max_in_flight: usize,
    ) -> Result<Self, EgressError> {
        if max_response_bytes == 0 || max_in_flight == 0 {
            return Err(EgressError::InvalidRequest("limits must be non-zero"));
        }
        Self::build(
            max_redirects,
            max_response_bytes,
            connect_timeout,
            request_timeout,
            max_in_flight,
            DestinationPolicy::PublicHttps,
            Arc::new(PublicDnsResolver),
        )
    }
    fn build<R: Resolve + 'static>(
        max_redirects: usize,
        max_response_bytes: usize,
        connect_timeout: Duration,
        request_timeout: Duration,
        max_in_flight: usize,
        policy: DestinationPolicy,
        resolver: Arc<R>,
    ) -> Result<Self, EgressError> {
        let client = reqwest::Client::builder()
            .dns_resolver(resolver)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|_| EgressError::Protocol)?;
        Ok(Self {
            client,
            max_redirects,
            max_response_bytes,
            admission_timeout: connect_timeout,
            operation_timeout: request_timeout,
            permits: Arc::new(Semaphore::new(max_in_flight)),
            per_host_limit: max_in_flight.min(4),
            host_permits: Arc::new(Mutex::new(HashMap::new())),
            policy,
        })
    }
    pub fn request(
        &self,
        method: Method,
        url: Url,
        redirect: RedirectPolicy,
    ) -> Result<EgressRequest, EgressError> {
        validate_destination(&url, self.policy.clone())?;
        if redirect == RedirectPolicy::FollowSafeGet
            && method != Method::GET
            && method != Method::HEAD
        {
            return Err(EgressError::InvalidRequest(
                "only GET or HEAD may follow redirects",
            ));
        }
        Ok(EgressRequest {
            method,
            url,
            headers: HeaderMap::new(),
            body: None,
            redirect,
            credential_origin: None,
        })
    }
    pub async fn send(&self, request: EgressRequest) -> Result<EgressResponse, EgressError> {
        let mut stream = self.send_streaming(request).await?;
        let mut body = Vec::new();
        while let Some(chunk) = stream.next_chunk().await? {
            body.extend_from_slice(&chunk);
        }
        Ok(EgressResponse {
            status: stream.status,
            headers: stream.headers,
            final_url: stream.final_url,
            body,
        })
    }
    pub(crate) async fn send_with_preflight<F, Fut>(
        &self,
        request: EgressRequest,
        preflight: F,
    ) -> Result<EgressResponse, EgressError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), EgressError>>,
    {
        let deadline = tokio::time::Instant::now() + self.operation_timeout;
        let admission_deadline = deadline.min(tokio::time::Instant::now() + self.admission_timeout);
        let permit =
            tokio::time::timeout_at(admission_deadline, self.permits.clone().acquire_owned())
                .await
                .map_err(|_| EgressError::AdmissionTimeout)?
                .map_err(|_| EgressError::AdmissionTimeout)?;
        let origin = Origin::from_url(&request.url)?;
        let host = self.host_semaphore(origin)?;
        let host_permit = tokio::time::timeout_at(admission_deadline, host.acquire_owned())
            .await
            .map_err(|_| EgressError::AdmissionTimeout)?
            .map_err(|_| EgressError::AdmissionTimeout)?;
        tokio::time::timeout_at(deadline, preflight())
            .await
            .map_err(|_| EgressError::Timeout)??;
        let mut stream = self
            .send_admitted(request, permit, host_permit, deadline)
            .await?;
        let mut body = Vec::new();
        while let Some(chunk) = stream.next_chunk().await? {
            body.extend_from_slice(&chunk);
        }
        Ok(EgressResponse {
            status: stream.status,
            headers: stream.headers,
            final_url: stream.final_url,
            body,
        })
    }
    pub async fn send_streaming(
        &self,
        request: EgressRequest,
    ) -> Result<EgressStream, EgressError> {
        let deadline = tokio::time::Instant::now() + self.operation_timeout;
        let admission_deadline = deadline.min(tokio::time::Instant::now() + self.admission_timeout);
        let permit =
            tokio::time::timeout_at(admission_deadline, self.permits.clone().acquire_owned())
                .await
                .map_err(|_| EgressError::AdmissionTimeout)?
                .map_err(|_| EgressError::AdmissionTimeout)?;
        let origin = Origin::from_url(&request.url)?;
        let host = self.host_semaphore(origin)?;
        let host_permit = tokio::time::timeout_at(admission_deadline, host.acquire_owned())
            .await
            .map_err(|_| EgressError::AdmissionTimeout)?
            .map_err(|_| EgressError::AdmissionTimeout)?;
        self.send_admitted(request, permit, host_permit, deadline)
            .await
    }
    async fn send_admitted(
        &self,
        mut request: EgressRequest,
        permit: OwnedSemaphorePermit,
        mut host_permit: OwnedSemaphorePermit,
        deadline: tokio::time::Instant,
    ) -> Result<EgressStream, EgressError> {
        let mut admitted_origin = Origin::from_url(&request.url)?;
        for hop in 0..=self.max_redirects {
            validate_destination(&request.url, self.policy.clone())?;
            if request
                .credential_origin
                .as_ref()
                .is_some_and(|o| Origin::from_url(&request.url).as_ref() != Ok(o))
            {
                return Err(EgressError::InvalidDestination("credential origin changed"));
            }
            let mut builder = self
                .client
                .request(request.method.clone(), request.url.clone())
                .headers(request.headers.clone());
            if let Some(body) = request.body.clone() {
                builder = builder.body(body);
            }
            let response = tokio::time::timeout_at(deadline, builder.send())
                .await
                .map_err(|_| EgressError::Timeout)?
                .map_err(|e| classify(&e))?;
            if response.status().is_redirection() {
                if request.redirect == RedirectPolicy::Reject || request.credential_origin.is_some()
                {
                    return Err(EgressError::InvalidDestination("redirect is forbidden"));
                }
                if hop == self.max_redirects {
                    return Err(EgressError::InvalidDestination("redirect limit exceeded"));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .ok_or(EgressError::Protocol)?
                    .to_str()
                    .map_err(|_| EgressError::Protocol)?;
                let next = request
                    .url
                    .join(location)
                    .map_err(|_| EgressError::Protocol)?;
                validate_destination(&next, self.policy.clone())?;
                let next_origin = Origin::from_url(&next)?;
                if next_origin != admitted_origin {
                    drop(response);
                    let next_host = self.host_semaphore(next_origin.clone())?;
                    let redirect_admission =
                        deadline.min(tokio::time::Instant::now() + self.admission_timeout);
                    host_permit =
                        tokio::time::timeout_at(redirect_admission, next_host.acquire_owned())
                            .await
                            .map_err(|_| EgressError::AdmissionTimeout)?
                            .map_err(|_| EgressError::AdmissionTimeout)?;
                    admitted_origin = next_origin;
                }
                request.url = next;
                let mut safe_headers = HeaderMap::new();
                for name in ["accept", "accept-language", "user-agent"] {
                    if let Some(value) = request.headers.get(name) {
                        safe_headers.insert(HeaderName::from_static(name), value.clone());
                    }
                }
                request.headers = safe_headers;
                request.body = None;
                continue;
            }
            if response
                .content_length()
                .is_some_and(|n| n > self.max_response_bytes as u64)
            {
                return Err(EgressError::ResponseTooLarge {
                    limit: self.max_response_bytes,
                });
            }
            let status = response.status();
            let headers = response.headers().clone();
            let final_url = request.url;
            return Ok(EgressStream {
                status,
                headers,
                final_url,
                response,
                max_response_bytes: self.max_response_bytes,
                received: 0,
                deadline,
                _permit: permit,
                _host_permit: host_permit,
            });
        }
        Err(EgressError::InvalidDestination("redirect limit exceeded"))
    }
}

fn validate_destination(url: &Url, policy: DestinationPolicy) -> Result<(), EgressError> {
    match policy {
        DestinationPolicy::PublicHttps if url.scheme() != "https" => {
            return Err(EgressError::InvalidDestination("HTTPS is required"));
        }
        DestinationPolicy::ScopedOrigins(ref origins)
            if !origins.contains(&Origin::from_url(url)?) =>
        {
            return Err(EgressError::InvalidDestination("origin is not allowlisted"));
        }
        #[cfg(any(test, feature = "browser-fixtures"))]
        DestinationPolicy::FixtureHttp if url.scheme() != "http" => {
            return Err(EgressError::InvalidDestination("fixture HTTP is required"));
        }
        _ => {}
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EgressError::InvalidDestination(
            "URL credentials are forbidden",
        ));
    }
    let port = url
        .port_or_known_default()
        .ok_or(EgressError::InvalidDestination("port is required"))?;
    if matches!(policy, DestinationPolicy::PublicHttps) && port != 443 {
        return Err(EgressError::InvalidDestination("port is not allowed"));
    }
    if matches!(policy, DestinationPolicy::ScopedOrigins(_)) {
        return Ok(());
    }
    match url
        .host()
        .ok_or(EgressError::InvalidDestination("host is required"))?
    {
        url::Host::Ipv4(a) if !is_public_ip(a.into()) => {
            Err(EgressError::InvalidDestination("non-public address"))
        }
        url::Host::Ipv6(a) if !is_public_ip(a.into()) => {
            Err(EgressError::InvalidDestination("non-public address"))
        }
        _ => Ok(()),
    }
}

#[derive(Debug)]
struct PublicDnsResolver;
impl Resolve for PublicDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            checked_addresses(tokio::net::lookup_host((host.as_str(), 0)).await?.collect())
        })
    }
}

#[derive(Debug)]
struct SystemDnsResolver;
impl Resolve for SystemDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addresses.is_empty() {
                return Err("DNS returned no addresses".into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[cfg(any(test, feature = "browser-fixtures"))]
#[derive(Debug)]
struct FixedTestResolver(SocketAddr);
#[cfg(any(test, feature = "browser-fixtures"))]
impl Resolve for FixedTestResolver {
    fn resolve(&self, _: Name) -> Resolving {
        let address = self.0;
        Box::pin(async move { Ok(Box::new(vec![address].into_iter()) as Addrs) })
    }
}

#[cfg(any(test, feature = "browser-fixtures"))]
impl ControlledHttpClient {
    pub(crate) fn fixture(address: SocketAddr, max_response_bytes: usize) -> Self {
        Self::fixture_with_inflight(address, max_response_bytes, 2)
    }

    fn fixture_with_inflight(
        address: SocketAddr,
        max_response_bytes: usize,
        max_in_flight: usize,
    ) -> Self {
        Self::build(
            max_in_flight,
            max_response_bytes,
            Duration::from_secs(1),
            Duration::from_secs(2),
            2,
            DestinationPolicy::FixtureHttp,
            Arc::new(FixedTestResolver(address)),
        )
        .expect("valid fixture client")
    }
}
fn checked_addresses(
    addresses: Vec<SocketAddr>,
) -> Result<Addrs, Box<dyn std::error::Error + Send + Sync>> {
    if addresses.is_empty() {
        return Err("DNS returned no addresses".into());
    }
    if addresses.iter().any(|a| !is_public_ip(a.ip())) {
        return Err("DNS returned a non-public address".into());
    }
    Ok(Box::new(addresses.into_iter()) as Addrs)
}
fn in_prefix(value: u128, network: u128, bits: u8) -> bool {
    value >> (128 - bits) == network >> (128 - bits)
}
fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(a) => is_public_ipv4(a),
        IpAddr::V6(a) => is_public_ipv6(a),
    }
}
fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    let denied = [
        (0x00000000, 8),
        (0x0a000000, 8),
        (0x64400000, 10),
        (0x7f000000, 8),
        (0xa9fe0000, 16),
        (0xac100000, 12),
        (0xc0000000, 24),
        (0xc0000200, 24),
        (0xc0586300, 24),
        (0xc0a80000, 16),
        (0xc6120000, 15),
        (0xc6336400, 24),
        (0xcb007100, 24),
        (0xe0000000, 4),
        (0xf0000000, 4),
    ];
    !denied
        .iter()
        .any(|&(network, bits)| value >> (32 - bits) == network >> (32 - bits))
}
fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(v4) = address.to_ipv4_mapped() {
        return is_public_ipv4(v4);
    }
    let value = u128::from(address);
    let denied = [
        (0, 128),
        (1, 128),
        (0x01000000000000000000000000000000, 64),
        (0x20010000000000000000000000000000, 23),
        (0x20010002000000000000000000000000, 48),
        (0x20010db8000000000000000000000000, 32),
        (0x20010020000000000000000000000000, 28),
        (0x20020000000000000000000000000000, 16),
        (0x3fff0000000000000000000000000000, 20),
        (0x5f000000000000000000000000000000, 16),
        (0xfc000000000000000000000000000000, 7),
        (0xfe800000000000000000000000000000, 10),
        (0xff000000000000000000000000000000, 8),
    ];
    (value >> 125) == 1 && !denied.iter().any(|&(n, b)| in_prefix(value, n, b))
}

#[cfg(test)]
mod tests {
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
            Box::pin(
                async move { Ok(Box::new(vec![a.ok_or("DNS exhausted")?].into_iter()) as Addrs) },
            )
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
        let a=server(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n".to_vec()).await;
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

        let denied =
            Url::parse(&format!("http://127.0.0.1:{}/receiver", address.port() + 1)).unwrap();
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
        let error =
            parse_provider_json::<Expected>(format!(r#"{{"count":"{sentinel}"}}"#).as_bytes())
                .unwrap_err()
                .to_string();
        assert!(!error.contains(sentinel));
        assert!(error.contains("line"));
        assert!(error.contains("column"));
    }
}
