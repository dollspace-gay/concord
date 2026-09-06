# Actor-owned domain services

The engine is the application boundary for commands, authorization, durable messaging, replay, organization, moderation, community, and integrations. ChatEngine coordinates sessions and projections.

Never import HTTP or IRC adapters. Preserve actor checks and authorization stamps through reads and delivery. Commit mutation, receipt, and durable event together; external work follows durable acceptance.

Run the affected engine unit tests and python3 scripts/check-actor-service-boundaries.py. Command changes also need the application-policy suite.
