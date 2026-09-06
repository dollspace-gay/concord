# Tests test coverage

This directory owns tests for concord/server/src/irc/connection. Keep scenario modules organized by behavior and share setup through the existing fixture owner.

Preserve test names, assertions, feature gates, fixture isolation, and required-suite selectors when moving cases. Test credentials and provider fixtures must stay local and isolated; do not replace real protocol assertions with source-string checks.

Run the owning test target and compare test discovery before/after structural changes. Rust tests use the server workspace; browser tests use the harness or authenticated runner for this directory.
