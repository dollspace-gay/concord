# Maintainability refactor result

Verdict: **ready for review**. Issue #14 is implemented in the working tree against 0e42049. No commit or push was requested or performed.

## Delivered structure

The 14,156-line ChatEngine file now has a small module root, explicit public re-exports, and cohesive children for session orchestration, commands, projections, and service access. Domain services, authorization, persistence, OAuth/AT Protocol, media, IRC registration/writing, WebSocket routing/handlers, and operator commands have corresponding module owners. Scenario tests live beside those owners. Existing public import paths and production authority/transaction boundaries are preserved.

The browser store now has a compatibility facade with typed action factories, event families, projection/synchronization helpers, and shared lifecycle state. Components separate composition, message rendering, and domain tabs; API types have a stable barrel and domain modules. Browser test entry points register the original scenarios in their original order while implementations live in behavior files.

Qualification entry points now delegate to generator and analyzer packages. The generator fingerprint includes every local generator source and its provenance implementation. The architecture guard follows production module declarations recursively and checks inherited repository aliases; test-only modules remain excluded.

## File policy

The source inventory changed from 222 to 763 handwritten source files, largely by extracting domain implementations and scenario tests. Files above 500 lines decreased from **57 to 4**. All **115 owned source directories** have local AGENTS.md guidance. Generated contracts, dependencies, build outputs, historical evidence, and lockfiles are outside the source budget.

The CI check requires either a file at or below 500 lines or a reviewed exception with a reason and fixed maximum. It also rejects missing guidance, stale exceptions, and unnecessary exceptions. The four exceptions are:

| File | Lines | Reason |
| --- | ---: | --- |
| engine/events/protocol.rs | 693 | Flat public event wire enum |
| web/ws_handler/protocol.rs | 659 | Flat client wire enum and serde defaults |
| web/ws_handler/dispatch.rs | 980 | Exhaustive typed routing for 124 commands |
| engine/chat_engine/legacy_send.rs | 556 | Test-only legacy compatibility oracle |

Paths in this table are relative to concord/server/src. Details and growth caps are in size-exceptions.json. The soft limit is a review rule, not permission to compress code or scatter one operation across arbitrary chunks.

## Validation

All final checks passed:

- Rust formatting and locked strict all-target/all-feature Clippy.
- Default workspace tests: 1,072 passed. All-feature workspace tests: 1,075 passed, including crash/storage-fault and recovery targets. The pre/post refactor all-feature test inventory retains all 1,075 cases.
- Exact generated-contract comparison and four Rust contract tests; payload coverage includes 124 client and 105 server variants with minimal and edge shapes.
- Frontend lint and build; 51 browser harness tests; 20 authenticated real-server browser journeys.
- Recursive actor-service boundary check: 59 HTTP handlers/helpers and all 124 WebSocket operations; 16 mutation/regression tests. Both moved exact application-policy test selectors execute one passing test each.
- Maintainability policy: seven negative/regression tests. Qualification provenance/import checks: two tests covering inert imports, every generator source mutation, added modules, and ignored bytecode. Analyzer: 43 negative cases. Telemetry analyzer self-test passed.
- Bounded local load/recovery smoke: eight accepted messages, 40/40 expected deliveries, no duplicate deliveries or missing messages; reconnect, history/search, abusive-client recovery, permission race, concurrent uploads/provider failure, and restart probes passed their smoke requirements.
- Clean source install/update smoke: locked fresh release build of server/operator and browser assets, source/artifact fingerprints, static/TLS/operator layout, and installation/update verification passed.
- Final Git whitespace check.

The first authenticated run failed at private bot-response visibility after grouped test imports changed registration order. Registration now preserves the original order, and the complete 20-case rerun passed without changing product behavior or weakening assertions. This establishes the final passing result; it does not independently prove that order was the only possible contributor to the first failure.

Rust function-body comparison against the original source found only import/path and formatting differences outside the deliberately extracted IRC connection loop and WebSocket dispatcher. Those two control-flow changes preserve cancellation, credential lifetime, silent early returns, and correlated error/success behavior and are covered by the Rust/socket/browser checks.

This validates the structural change and bounded local behavior. It does not claim a new dedicated-host one-hour load qualification. The frontend build still emits its existing Tailwind source-map and bundle-size warnings.

## Evidence

Local logs and machine-readable inventories are under /home/doll/.cache/concord-maintainability:

- rust-tests-default.log, rust-tests-all.log, clippy2.log
- web-lint3.log, web-build-final.log, browser-tests2.log, authenticated-browser2.log
- contracts-final.log, boundary-tests3.log, selectors-final.log, maintainability-final.json
- load-recovery-evidence/20260906T181809Z-3757324
- authenticated-evidence/20260906T181917Z-3761830
- source-install-evidence/20260906T182122Z-3771952

Unrelated local configuration, managed integration files, and historical evidence remain outside this refactor. Review and commit are the next steps.
