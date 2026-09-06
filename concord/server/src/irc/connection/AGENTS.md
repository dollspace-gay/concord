# IRC connection lifecycle

The parent connection.rs drives deadlines, cancellation, input admission, and disconnect. registration.rs owns registration transitions; writer.rs owns outbound authority checks and byte accounting. Command handlers and event projections remain separate.

Keep the driver responsible for task completion. A registration stop must close the connection; stale protected replies must not escape through the writer. Retain queue descriptor and byte limits.

Run irc::connection::tests, session_authority, and scripts/run-irc-client-qualification.sh when lifecycle behavior changes.
