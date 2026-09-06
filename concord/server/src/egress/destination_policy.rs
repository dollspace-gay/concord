use super::{
    Addrs, DestinationPolicy, EgressError, IpAddr, Ipv4Addr, Ipv6Addr, Name, Origin, Resolve,
    Resolving, SocketAddr, Url,
};
#[cfg(any(test, feature = "browser-fixtures"))]
use super::{Arc, ControlledHttpClient, Duration};

pub(super) fn validate_destination(
    url: &Url,
    policy: DestinationPolicy,
) -> Result<(), EgressError> {
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
pub(super) struct PublicDnsResolver;

impl Resolve for PublicDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            checked_addresses(tokio::net::lookup_host((host.as_str(), 0)).await?.collect())
        })
    }
}

#[derive(Debug)]
pub(super) struct SystemDnsResolver;

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
pub(super) struct FixedTestResolver(SocketAddr);

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

    pub(super) fn fixture_with_inflight(
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

pub(super) fn checked_addresses(
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

pub(super) fn in_prefix(value: u128, network: u128, bits: u8) -> bool {
    value >> (128 - bits) == network >> (128 - bits)
}

pub(super) fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(a) => is_public_ipv4(a),
        IpAddr::V6(a) => is_public_ipv6(a),
    }
}

pub(super) fn is_public_ipv4(address: Ipv4Addr) -> bool {
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

pub(super) fn is_public_ipv6(address: Ipv6Addr) -> bool {
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
