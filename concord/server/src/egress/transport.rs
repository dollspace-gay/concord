use super::{
    ControlledHttpClient, EgressError, EgressRequest, EgressStream, HeaderMap, HeaderName,
    LOCATION, Origin, OwnedSemaphorePermit, RedirectPolicy, classify, validate_destination,
};

impl ControlledHttpClient {
    pub(super) async fn send_admitted(
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
