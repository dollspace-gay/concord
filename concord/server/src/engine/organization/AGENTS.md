# Organization service

The parent organization.rs owns shared service types and its public interface; child modules own cohesive domain operations.

Preserve server ownership, hierarchy, role-grant ceilings, channel override semantics, and projection-version bumps in the same transaction.

Run the organization unit tests and affected application-policy or transport journeys, plus strict Rust checks.
