# User session service

The parent user_session.rs owns shared service types and its public interface; child modules own cohesive domain operations.

Keep domain authorization, transaction ownership, and observable results intact. Avoid transport dependencies or public visibility expansion.

Run the user_session unit tests and affected application-policy or transport journeys, plus strict Rust checks.
