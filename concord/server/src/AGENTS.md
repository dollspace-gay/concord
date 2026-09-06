# Server source

Entry points compose services; domain behavior belongs in engine/, storage in db/, and protocol adaptation in web/ or irc/. Binary recovery paths live in bin/.

Keep errors structured and safe for their caller. Keep asynchronous work bounded and owned by a shutdown or cancellation path. Maintain exact public module paths when moving implementation.

Use the server crate checks and python3 scripts/check-actor-service-boundaries.py from the repository root.
