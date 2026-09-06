# IRC transport

Parse and frame untrusted input, translate commands through ChatEngine, and format bounded IRC output. Connection work belongs under connection/.

Preserve incremental framing, CRLF sanitation, canonical aliases, capability negotiation, credential cancellation, and delivery guards. Adapters cannot query domain repositories.

Run irc unit tests, session_authority transport tests, and the architecture guard. Registration or framing changes also need real IRC client qualification.
