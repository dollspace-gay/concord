# Replay test coverage

This directory owns tests for concord/web. Keep scenario modules organized by behavior and share setup through the existing fixture owner.

Preserve test names, assertions, feature gates, fixture isolation, and required-suite selectors when moving cases. Test credentials and provider fixtures must stay local and isolated; do not replace real protocol assertions with source-string checks.

Export scenario registration functions; ../replay.spec.ts calls them in the original order. Shared harness helpers and typed fixtures belong in fixtures.ts.

Run npm run test:browser from concord/web. Compare test discovery and order before/after structural changes.
