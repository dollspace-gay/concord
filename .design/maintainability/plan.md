# Concord maintainability refactor

## Objective

Break oversized handwritten source into cohesive modules and directories, with a soft 500-line file budget and local AGENTS.md guidance. Preserve public Rust imports, HTTP/IRC/WebSocket behavior, generated contracts, stored data, authorization, transaction boundaries, cancellation, and error semantics. Work from commit 0e42049; unrelated local configuration remains excluded. Crosslink issue: #14.

## Design

Prefer domain modules with small entry points and explicit dependencies. Rust modules retain existing public paths through narrowly scoped re-exports; service methods move with their responsibility while shared ownership stays with the service. Tests live beside their owning implementation in named scenario modules. Browser components, types, state transitions, and domain actions get separate owners. Qualification tools separate generation, analysis, and orchestration responsibilities.

A credible alternative is a blanket physical split using include files, numbered chunks, or a shared wildcard prelude. That would minimize textual changes but preserve hidden coupling and make ownership harder to understand. It is not the target architecture. Cross-crate decomposition is unnecessary for this refactor and would add public API and build complexity.

## File budget and guidance

500 lines is a review threshold for handwritten implementation and tests. Generated contracts, third-party dependencies, historical evidence, lockfiles, and managed provider tooling are excluded. An indivisible public protocol schema, an exhaustive routing table without business logic, or a coherent integration scenario may exceed the threshold only with a named reason and an explicit maximum in the exception registry. No compressed formatting, warning suppression, or placeholder extraction is permitted to meet the count.

Each owned source directory has an AGENTS.md describing purpose, allowed dependencies, critical invariants, and relevant validation. Child instructions add local information and inherit shared policy rather than copying it. Generated and vendor trees inherit guidance from their owner. Existing root guidance is preserved.

## Delivery increments

1. Inventory and establish architecture/size reporting; refactor central Rust engine and adjacent service modules.
2. Refactor web/IRC adapters, persistence/auth/configuration, and operator surfaces while maintaining entry points.
3. Refactor frontend state, components, and browser tests around domain ownership.
4. Refactor qualification scripts and test suites; update path-aware architecture guards and generated-source consumers.
5. Add local guidance, review the resulting dependency direction, verify test inventory, and execute the complete relevant validation matrix.

Each increment requires compilation or type checking before proceeding. Final checks: Rust formatting and strict all-target/all-feature Clippy; default and all-feature workspace tests run sequentially; exact contract regeneration and fixtures; architecture guard and its negative tests; frontend lint/build and browser harness/authenticated journeys; SQLite crash/sync-failure probes and affected qualification-tool negative tests. Build and installation consumers must include newly introduced directories. No commit or push is requested in this work unit.
