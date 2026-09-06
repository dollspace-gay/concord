# Migration and repair engine

The parent pool.rs is the public entry point. Catalog, introspection, preflight, repair, and migration execution are separate responsibilities.

Run preflight and repair gates before writes; keep snapshots, checksums, foreign-key checks, and schema verification inside the established transaction sequence. Embedded migration paths are relative to the owning source file.

Run db::pool::tests, migration_preflight, and scripts/run-required-suite.sh migration-fixtures.
