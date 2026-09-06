# S0 verification and contract-generator evidence

Date: 2026-09-05

Baseline source before the shared working-tree changes: `cff0df246b91461f959d8dbe9154ba50bba2331c`.
This report describes an uncommitted shared-tree implementation; it does not attribute a green run to that earlier immutable commit.

## Delivered in this slice

- `schemars` 1.2.2 derives JSON Schema directly from the production `ChatEvent`, nested event DTOs, and WebSocket `ClientMessage` Serde types. `contract::WebSocketContract` is only a generation root and does not duplicate fields.
- `generate_contract` emits canonical JSON Schema. Pinned `json-schema-to-typescript` 16.0.0 emits TypeScript declarations and pinned Ajv 8.20.0 supplies generated runtime validators. Generation compiles the validators and checks valid and malformed values.
- The Rust contract fixture passes values through the production Serde implementations, compiles the schema with `jsonschema` 0.33.0, and checks valid and corrupt bidirectional envelopes.
- `scripts/check-contract.sh` fails on generated drift. `scripts/check-contract-fixtures.sh` is the reproducible fixture-and-drift command.
- Required future suites have named package commands and a manual matrix CI workflow. A missing runner exits 2 with an explicit incomplete result, so it cannot be reported as skipped or passed. These interfaces are not suite implementations.
- Push/PR CI runs format, strict Clippy, Rust tests, contract drift/fixtures, frontend build/lint, and a high-severity dependency audit.
- Three existing `cloned_ref_to_slice_refs` findings in `server/src/integration_tests.rs` were corrected with `std::slice::from_ref`. The existing `chat_engine.rs` rustfmt-only finding was corrected without changing behavior.
- `sha2` 0.10 was added for the separately owned migration-checksum implementation while preserving existing Tokio, Tokio-util, SQLx, and feature configuration.

## Commands and observed results

Environment: rustc/cargo 1.96.0, Node 22.23.1, npm 10.9.8.

| Command | Result |
| --- | --- |
| `npm run contracts:check` | PASS after generation; checked-in schema, TS types, and validators match regeneration. |
| `npm run build` | PASS; Vite 7.3.6 built 67 modules. |
| `npm run lint` | PASS after generating `unknown` for unconstrained JSON values. |
| `npm audit --audit-level=high` | PASS; npm reports 0 vulnerabilities after a compatible lockfile refresh. |
| `./scripts/run-required-suite.sh browser-socket` | Expected INCOMPLETE, exit 2: runner is not implemented and qualification cannot pass. |
| `cargo fmt --all -- --check` | Not yet green during concurrent implementation: an IRC-owned terminal blank-line diff and formatting in the newly added migration fixture remained. |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Not yet green during concurrent implementation: the newly added migration fixture had one unused import; the three S0-owned clone findings were fixed. |
| `./scripts/check-contract-fixtures.sh` | PASS; 2 contract tests passed (778 filtered) and generated artifacts matched clean regeneration. |

Generated artifacts are `contract.schema.json`, `contract.ts`, and `validator.ts` under `concord/web/src/api/generated/`. The schema is structural and exhaustive over currently declared enum variants because it derives from the enums. Current fixtures exercise representative valid and invalid variants. G10 still requires a fixture for every command/event/error variant, request IDs, protocol mismatch, and unsupported capabilities as those v2 concepts land.

## Remaining S0/G14 obligations

Application-policy, browser/socket, deterministic-job, historical migration, storage-fault, packaging/restore, load/recovery, and container-smoke runners still need real implementations from their owning stages. The manual qualification workflow deliberately fails each missing matrix entry. Load qualification also needs the design's declared target host and workload metadata instead of treating `ubuntu-latest` as performance evidence. Full G14 and full S0 exit are not claimed by this slice.
