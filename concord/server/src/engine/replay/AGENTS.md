# Durable synchronization

Separate snapshot queries, message/state projections, replay, subscriptions, and cursor cryptography. Public types stay in ../replay.rs.

Retain bounded subscription windows, stable ordering, tombstones, actor binding, expiry, and authorization revalidation. Stale or incompatible cursors must require resynchronization.

Run engine::replay::tests, contract fixtures, and the browser replay tests.
