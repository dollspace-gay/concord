# Dependency audit status

This record describes the dependency graph after the September 6, 2026 dependency update. No advisories are suppressed. `cargo audit` reports two vulnerability matches for Hickory and one unsoundness warning for LRU, detailed below. `npm audit` reports zero vulnerabilities.

## Updated dependencies

All direct Cargo and npm requirements were checked against the latest stable registry releases, and both lockfiles were refreshed. Major upgrades include SQLx 0.9, Argon2 0.6, rand 0.10, reqwest 0.13, chacha20poly1305 0.11, SHA-2 0.11, TOML 1.1, Vite 8, ESLint 10, and TypeScript 7. The new SQLx graph removes the optional vulnerable RSA package; updated transitive packages also remove `core2` and the yanked `spin` 0.9.8 release.

Two compatibility constraints are intentional:

- `libsqlite3-sys` uses 0.37.0, the newest release permitted by SQLx 0.9's `<0.38.0` constraint. The fault-injection VFS must link the same SQLite library as SQLx.
- TypeScript 7.0.2 supplies the native compiler through `@typescript/native`. The `typescript` import aliases `@typescript/typescript6` 6.0.2 because typescript-eslint 8.69 supports the JavaScript compiler API through TypeScript 6.0. This follows the [TypeScript team's side-by-side installation instructions](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/) and [typescript-eslint's supported versions](https://typescript-eslint.io/users/dependency-versions/). No peer-dependency override is used.

Concord still uses `jsonwebtoken` 11 with its AWS-LC backend and explicitly HS256 tokens. AT Protocol 0.14.5 remains the latest stable parent release. The existing Rust 1.96 and Node 22.23.1 environment supports the updated dependencies.

## Residual audit entries

- `RUSTSEC-2026-0118` and `RUSTSEC-2026-0119` remain recorded for `hickory-proto` 0.25.2. `atproto-oauth` 0.14.5 enables `atproto-identity`'s defaults through its own dependency declaration even when Concord disables the OAuth crate's default features. The DNSSEC feature implicated by 0118 is absent. Concord does not instantiate `HickoryDnsTxtResolver` or any AT identity resolver; handle and DID network resolution uses the egress-checked HTTP requests in `web/atproto.rs`. Hickory DNS parsing and encoding therefore have no runtime entry point in Concord. The current upstream 0.14.5 release still constrains this dependency to 0.25.
- `RUSTSEC-2026-0253` remains recorded for `lru` 0.16.4. It is enabled by the same upstream `atproto-identity` default-feature declaration. Concord does not import or instantiate `storage_lru` or `LruDidDocumentStorage`, so the affected `LruCache::pop` implementation has no runtime entry point. The current upstream AT identity release constrains `lru` to 0.16, below the advisory's patched 0.18.2 release.

Re-run `cargo audit` after each AT Protocol parent upgrade. Remove a residual entry only when the lockfile no longer contains it or its upstream advisory is withdrawn.
