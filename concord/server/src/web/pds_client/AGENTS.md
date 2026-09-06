# PDS request client

Separate authenticated requests, blob uploads, record writes, and token refresh. Preserve the parent public client interface.

Refresh coordination is account-scoped. Preserve token/nonces across retries and use controlled egress on every attempt. External publication must remain tied to its durable job/grant.

Run web::pds_client::tests and deterministic-jobs.
