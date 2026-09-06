# Canonical durable commands

The parent messaging.rs owns command types and shared service state. Children own send, edit, delete, reaction, read, announcement, receipt, policy, and outbox responsibilities.

One admitted transaction must authorize and commit message state, idempotency receipt, entity version, and durable event together. Preserve fault barriers and cancellation behavior. Never duplicate sends in an adapter.

Run engine::messaging::tests and storage-faults, including process crash and sync-failure cases.
