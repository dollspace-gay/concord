# Authority implementation

The parent authority.rs owns shared credential types and state. Child modules implement session/token issuance, actor validation, live cancellation, and secret verification.

Use pub(super) only for coordination between these children. Revocation must update durable state and notify every matching live lease without affecting other credentials.

Run cargo test --manifest-path concord/Cargo.toml -p concord-server --test session_authority and the default/all-feature compile checks.
