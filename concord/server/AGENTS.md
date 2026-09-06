# Server crate

This crate owns authentication, domain services, database state, HTTP/WebSocket/IRC adapters, and operator binaries. Keep adapter dependencies directed toward services and services toward persistence.

Preserve transaction boundaries, actor scope, credential cancellation, bounded queues, and shutdown ownership. Public compatibility re-exports must not widen private APIs. Migrations and generated contract changes need explicit behavior review.

Run cargo fmt --manifest-path concord/Cargo.toml --all -- --check and cargo clippy --manifest-path concord/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings. Run relevant tests, then default and all-feature workspace tests sequentially; feature builds share executable paths.
