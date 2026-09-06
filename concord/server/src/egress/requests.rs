use super::{
    ControlledHttpClient, EgressError, EgressRequest, EgressResponse, EgressStream, HeaderMap,
    Method, Origin, RedirectPolicy, Url, validate_destination,
};

impl ControlledHttpClient {
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
}
