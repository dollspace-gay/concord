# Applied schema history

Each numbered SQL file is an ordered migration registered by db/pool/catalog.rs. Treat deployed migrations as immutable.

A new schema change must include its preflight, compatibility, repair, and integrity implications. Preserve checksums and fail closed on unrecognized schema drift.

Run migration-fixtures and db::pool tests; structural source refactors must not modify these SQL files.
