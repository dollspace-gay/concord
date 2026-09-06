# Credential authority

Own credential issue, verification, live leases, revocation, and secret storage. Consumers receive actor identity through the authority API.

Revalidate durable credential state before privileged use. Never log tokens, secret material, or private provider payloads; keep expensive hashing admitted and off the async executor.

Run the session_authority integration target and auth unit tests for changes here.
