# Integrations service

The parent integrations.rs owns shared service types and its public interface; child modules own cohesive domain operations.

Keep installation/grant scope, stable bot ownership, idempotency, durable job/outbox creation, and safe credential handling.

Run the integrations unit tests and affected application-policy or transport journeys, plus strict Rust checks.
