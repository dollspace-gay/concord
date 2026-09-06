# AT Protocol OAuth adapter

Keep signing keys, pending state, discovery, login/callback, token exchange, identity resolution, and profile reads in their own modules.

Pending state is single-use and expiring. Use controlled egress, encrypted key/token storage, nonce-aware DPoP, and safe errors; never disclose provider credentials.

Run web::atproto tests and deterministic provider/job fixtures for changes here.
