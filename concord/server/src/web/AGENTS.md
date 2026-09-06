# Web transport and provider adapters

HTTP and WebSocket routes translate authenticated requests to actor-owned services. Provider clients use controlled egress and durable account grants.

Do not move repository access into REST or command handlers. Preserve request correlation, status/error semantics, origin checks, token secrecy, and outbound destination restrictions.

Run web unit tests, the architecture guard, contract checks, and authenticated browser journeys for affected routes.
