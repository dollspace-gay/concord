# Persistence boundary

Own schema installation, row models, and database adapters. Domain services own authorization and command transaction boundaries.

Do not rewrite applied migration files during maintenance. Preserve FULL/WAL durability, exclusion locking, schema fingerprints, integrity checks, and explicit repair gates.

Run migration unit/integration tests and the migration-fixtures suite when schema or migration code changes.
