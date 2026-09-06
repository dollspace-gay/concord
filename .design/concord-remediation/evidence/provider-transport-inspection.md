# Provider transport implementation input

Inspected the pinned local `atproto-oauth 0.13.0` and `atproto-identity 0.13.0` source while the initial Sol implementation slices run. This is source evidence for the future R04/R21 implementation, not a passing egress gate.

The OAuth workflow (`workflow.rs::oauth_init`, `oauth_complete`, `oauth_refresh`) accepts a concrete `reqwest::Client`, creates middleware for DPoP retry, and reads response JSON without a byte bound. `resources.rs::oauth_protected_resource` and `oauth_authorization_server` also read JSON without a body bound. Identity `plc::query`, `web::query`, and HTTP handle resolution use the same concrete-client pattern. Client injection alone therefore does not enforce the design's response-body limit on these calls.

The application currently creates fresh unrestricted clients in `web/atproto.rs`, `web/pds_client.rs`, and profile-sync routes. PDS failures include response bodies and URLs in errors; the caller attempts refresh on any request error. Credential refresh has no per-account serialization/version compare-and-swap. OAuth callback state is not bound to a browser cookie, the consumed pending request does not enforce its stored expiry, and the token subject is not compared to the original resolved DID before using the submitted handle as its username.

Implementation direction: retain the library's independently usable key, JWK, PKCE, JWT, and DPoP primitives and typed metadata where appropriate; move all network exchanges behind bounded, destination-validated application transport. Preserve PAR, PKCE, issuer/subject binding, client assertions and bounded DPoP nonce retries. Verify current protocol requirements and implement controlled fixtures before enabling the replacement. A wrapper that only checks the initial URL or changes the client's DNS resolver cannot count as complete.

Additional preview parsing findings belong to the same slice: `engine/embeds.rs::extract_meta` uses `pos + 500` as a string slicing boundary, and `extract_html_title` indexes the original string using positions in a lowercased copy whose byte length can differ. Fix Unicode boundary handling as well as bounding the network body; cover both with regression inputs.

No provider login, publication, or deletion was performed by this inspection.
