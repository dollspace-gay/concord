use super::{
    Arc, ControlledHttpClient, DestinationPolicy, Duration, EgressError, HashMap, HashSet, Mutex,
    Origin, PublicDnsResolver, Resolve, Semaphore, SystemDnsResolver, Url,
};

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

    pub(super) fn host_semaphore(&self, origin: Origin) -> Result<Arc<Semaphore>, EgressError> {
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

    pub(super) fn build<R: Resolve + 'static>(
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
}
