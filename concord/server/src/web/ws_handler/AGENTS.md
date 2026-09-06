# WebSocket commands

protocol.rs owns wire input, connection.rs owns transport lifetime, envelope.rs owns correlation and lifecycle acknowledgements, and dispatch.rs routes commands to domain handlers.

Keep dispatch exhaustive and free of business logic. A ControlFlow::Break from a handler preserves a silent stale-authorization exit; it must not emit a success or error reply. Keep payloads typed and preserve every wire variant.

Run web::ws_handler::tests, contract checks, the architecture guard, and both browser suites. The flat protocol and pure routing table have explicit size exceptions.
