# Moderation service

The parent moderation.rs owns shared service types and its public interface; child modules own cohesive domain operations.

Keep hierarchy checks, reason validation, timeout/ban semantics, audit writes, and post-commit subscription eviction together.

Run the moderation unit tests and affected application-policy or transport journeys, plus strict Rust checks.
