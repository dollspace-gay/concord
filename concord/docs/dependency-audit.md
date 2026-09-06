# Dependency audit status

This record describes the dependency graph after the September 2026 remediation pass. It does not suppress advisories. `cargo audit` remains expected to report the residual entries below until their upstream dependency constraints change.

## Updated dependencies

The lockfile now selects patched releases of `aws-lc-sys` (0.45.0), `crossbeam-epoch` (0.9.21), `h2` (0.4.19), `quinn-proto` (0.11.17), `rustls-webpki` (0.103.15), `anyhow` (1.0.104), `event-listener` (5.4.2), and the 0.8/0.9 `rand` lines. Concord uses `jsonwebtoken` 11 with its AWS-LC backend because its tokens are explicitly HS256; the RustCrypto RSA backend is not enabled. The AT Protocol crates are updated to 0.14.5, which also removes the old `serde_ipld_dagcbor`/`ipld-core` dependency chain. IRC TLS PEM loading now uses the maintained `rustls-pki-types` API.

## Residual audit entries

- `RUSTSEC-2026-0118` and `RUSTSEC-2026-0119` remain recorded for `hickory-proto` 0.25.2. `atproto-oauth` 0.14.5 enables `atproto-identity`'s defaults through its own dependency declaration even when Concord disables the OAuth crate's default features. The DNSSEC feature implicated by 0118 is absent. Concord does not instantiate `HickoryDnsTxtResolver` or any AT identity resolver; handle and DID network resolution uses the egress-checked HTTP requests in `web/atproto.rs`. Hickory DNS parsing and encoding therefore have no runtime entry point in Concord. The current upstream 0.14.5 release still constrains this dependency to 0.25.
- `RUSTSEC-2026-0253` remains recorded for `lru` 0.16.4. It is enabled by the same upstream `atproto-identity` default-feature declaration. Concord does not import or instantiate `storage_lru` or `LruDidDocumentStorage`, so the affected `LruCache::pop` implementation has no runtime entry point. The current upstream AT identity release constrains `lru` to 0.16, below the advisory's patched 0.18.2 release.
- `RUSTSEC-2023-0071` remains present as an optional `sqlx-mysql`/`sqlx-macros-core` lockfile package. `cargo tree -i rsa --target all` has no path from any Concord target. Concord enables only SQLx SQLite, and `jsonwebtoken` uses AWS-LC with HS256, so RSA private-key operations are not compiled into a Concord target.
- `RUSTSEC-2026-0105` records `core2` 0.4.0 as unmaintained and yanked. It remains under `atproto-dasl -> cid/multihash` in the latest AT Protocol 0.14.5 release. This is a maintenance warning rather than a reported vulnerability; no compatible maintained replacement is exposed by that parent release.
- `spin` 0.9.8 remains yanked through current `flume`/SQLx SQLite and other published dependencies. There is no associated RustSec vulnerability and no compatible update in the current parent releases.

Re-run `cargo audit` after each AT Protocol or SQLx parent upgrade. Remove an entry from this record only when the lockfile no longer contains it or its upstream advisory is withdrawn.
