# Authorization policy

Actor scope, conversation access, channel policy, visibility, search, and authorization stamps have separate modules; shared types stay in ../authorization.rs.

Visibility and history permissions are distinct. Private threads cannot exceed parent authority. Permission changes invalidate stamps, and a protected projection must be rechecked before delivery.

Run engine::authorization::tests and application-policy. Keep search keyset and stale-stamp regression coverage.
