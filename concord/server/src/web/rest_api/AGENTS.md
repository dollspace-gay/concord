# HTTP domain adapters

The parent rest_api.rs preserves route exports and extractors. Each child owns one HTTP domain, such as accounts, uploads, servers, search, or integrations.

Handlers delegate authorization and mutations to services. Only the approved rooted storage adapters may touch media files. Preserve request/response shapes, status codes, and revalidation after asynchronous work.

Run web::rest_api::tests and scripts/test-actor-service-boundaries.py; update route-level browser coverage when behavior changes.
