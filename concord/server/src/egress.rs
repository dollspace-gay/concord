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

impl ControlledHttpClient {}

#[cfg(test)]
mod tests;

mod construction;
mod destination_policy;
mod requests;
mod transport;

use destination_policy::PublicDnsResolver;
use destination_policy::SystemDnsResolver;

use destination_policy::validate_destination;
