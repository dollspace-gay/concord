# Concord remediation baseline evidence

This record supports the [remediation design](../../concord-remediation.md). It records review evidence from 2026-09-05 against commit `cff0df246b91461f959d8dbe9154ba50bba2331c`. The design turn rechecked the current commit, relevant source/callers/schema, worktree status, and the source hashes. No production application source, migrations, lockfiles, configuration, or stored community data changed during the review/design work.

## Scope and limits

- Read the Rust workspace, both transport adapters, database/schema, AT Protocol/media integration, React state and core components, and packaging/documentation.
- Original checks ran locally with Rust 1.96.0 and Node 22.23.1. The reviewed backend test suite passed 779 tests; the frontend built and linted. Dependencies were installed with `npm ci --ignore-scripts` for the local review.
- Two probes used synthetic data, an in-memory SQLite database, and a temporary loopback listener. The IRC probe copied the exact private read helper rather than changing its visibility in the application.
- No live OAuth/PDS account, production deployment, destructive external cleanup, visual browser usability test, or production-scale load test was exercised. Source-derived concerns are labeled separately in the design.
- The existing unrelated worktree changes in `.claude/settings.json`, `.gitignore`, `.mcp.json`, `.codex/`, `.crosslink/`, and `AGENTS.md` were preserved. The design adds only `.design/` artifacts. No commit, push, implementation agent, or release was performed.

## Existing checks

| Command and location | Observed result |
| --- | --- |
| `cargo test --workspace --locked` from `concord/` | 779 passed, zero failed/ignored; binary/doc-test targets had zero tests. |
| `cargo fmt --all -- --check` from `concord/` | Failed on formatting differences in the existing source. |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` from `concord/` | Failed on three `cloned_ref_to_slice_refs` findings in `server/src/integration_tests.rs`, at baseline lines 1600, 2000, and 2012. |
| `npm run build` from `concord/web/` | TypeScript and Vite passed; Vite transformed 67 modules. |
| `npm run lint` from `concord/web/` after dependencies were installed | Passed. The earlier attempt without installed dependencies failed to find ESLint and is not attributed to the code. |
| `npm audit --json` from `concord/web/` | Reported ten dependency findings: two low, two moderate, six high. Reachability/production exploitability was not assessed. |

These command results belong to this baseline; they are not new acceptance results for the proposed design. Compiler warnings and formatting issues are much smaller than the demonstrated behavioral defects, and all categories remain tracked by the design.

## Application probe

Source: [concord-review-probe.rs](concord-review-probe.rs). Output: [concord-review-probe.txt](concord-review-probe.txt).

The probe links the compiled Concord library and its dependencies, creates owner/member/outsider identities, a server and private channel, an explicit VIEW_CHANNELS deny for the member, and one synthetic message. It exercises actual HTTP/WebSocket routes on an ephemeral loopback port, then calls the production engine to characterize session replacement and forced insert failure.

```text
REST private-channel search: status=200 OK, leaked_secret=true
Revoked token REST /api/me: 401 Unauthorized
Revoked token WebSocket upgrade: 101 Switching Protocols
Nonmember WebSocket search: leaked_secret=true
Web + IRC same identity: original_session_survives=false
Injected INSERT failure: acknowledged=true, persisted_rows=0
```

The token is generated with a synthetic local test secret, revoked in the running application's blocklist, and never printed. The SQLite failure comes from a test-only trigger rejecting inserts. No actual community message or credential is in these artifacts. The assertions intentionally characterize the vulnerable baseline; a future acceptance test must assert the opposite outcomes.

## IRC framing probe

Source: [concord-review-irc.rs](concord-review-irc.rs). Output: [concord-review-irc.txt](concord-review-irc.txt).

The probe copies `read_bounded_line` from `concord/server/src/irc/connection.rs`, links the same compiled Tokio dependency with its full feature set, and uses an in-process duplex stream. One case provides a complete `NICK owner\r\n` command. The other writes `NI` followed by the remainder after a short delay. An external process timeout kills a non-yielding read so the test cannot hang the review runtime.

```text
complete: returncode=0, read 12 bytes
fragment: timed out after 2 seconds (process killed)
```

This establishes the helper's fragmented-input failure. It is not a claim that the entire IRC registration/TLS workflow was exercised by this probe; those are required future gates.

## Reproduction and provenance

The [source manifest](source-manifest.json) hashes the reviewed Rust sources, migrations, frontend sources, principal manifests/lockfiles, README and packaging/config example, plus the copied probe artifacts. It excludes live databases, credentials, dependency/build directories, and unrelated local configuration. A mismatch means the old observations must be re-evaluated before being attached to a new head.

The application probe was compiled with `rustc --edition=2024`, dependency search in the workspace's `target/debug/deps`, and explicit `--extern` entries for the compiled `concord_server`, `tokio`, `sqlx`, `axum`, `reqwest`, `tokio_tungstenite`, `futures_util`, `serde_json`, and `uuid` libraries. Host/build-only dependency variants cannot be mixed with the runtime variants; the original review corrected that harness linkage before obtaining the results above. The IRC probe links only the matching full-feature Tokio library.

These are preserved source experiments, not a portable supported test runner. S0 incorporates their scenarios into the project's real Cargo/socket harness, eliminating artifact-hash linkage and making failure/cleanup deadlines explicit. Keep the distinction between a reproduced baseline defect and a passing repaired regression.

External protocol/design constraints were checked against the primary sources linked next to their use in the remediation document: AT Protocol blobs/OAuth, SQLite durability/foreign keys/backup, IRCv3 server-time, OAuth security best practice, OWASP SSRF guidance, and the existing Reqwest client API. No external source was treated as authority to operate user accounts or change project scope.
