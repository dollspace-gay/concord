# Egress implementation review inputs

During implementation on 2026-09-06 UTC, the parent reviewed the initial
`server/src/egress.rs` wrapper and returned the following requirements to its
Sol owner. These are review inputs, not claims that the final wrapper or G04
has passed.

- Keep the request builder private so callers cannot bypass bounded response
  consumption or submit a request built by an unrestricted client.
- Disable implicit environment proxies, or implement an explicit trusted proxy
  policy that preserves destination enforcement.
- Bound admission, ports, redirects, response consumption and parsing. Apply
  different credential and redirect policies to public previews and authenticated
  provider operations.
- Prevent credential-bearing redirects to another origin, including custom
  proof headers, and keep token-bearing URLs out of error chains and logs.
- Validate special-purpose IPv6 ranges beyond a broad `2000::/3` membership
  test. The current [IANA IPv6 registry](https://www.iana.org/assignments/iana-ipv6-special-registry/)
  lists `2001:2::/48` for benchmarking, `2001:10::/28` as deprecated ORCHID,
  `3fff::/20` for documentation, and the Teredo/6to4 transition ranges. A public
  egress policy needs explicit treatment of these ranges. The
  [IANA IPv4 registry](https://www.iana.org/assignments/iana-ipv4-special-registry/)
  is the corresponding primary reference for IPv4 exclusions.
- Test real DNS/HTTP/TLS behavior and connection attempts, including rebinding,
  redirect, compression, timeout and credential cases. Three initial pure URL/IP
  tests do not establish the required network boundary.

Both IANA registries were fetched during this review and reported a last update
of 2025-10-09. Provider-library internal transport remains covered by the separate
`provider-transport-inspection.md` input.
