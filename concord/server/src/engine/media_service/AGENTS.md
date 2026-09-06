# Media service service

The parent media_service.rs owns shared service types and its public interface; child modules own cohesive domain operations.

Authorize upload/download and claim ownership through the actor service; preserve quotas, rooted media, grant revalidation, and replaced-asset retirement.

Run the media_service unit tests and affected application-policy or transport journeys, plus strict Rust checks.
