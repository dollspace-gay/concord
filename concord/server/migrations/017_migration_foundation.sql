-- Migration 017: verifiable migration history and repair provenance.

CREATE TABLE migration_metadata (
    version INTEGER PRIMARY KEY REFERENCES schema_version(version),
    checksum_sha256 TEXT NOT NULL CHECK(length(checksum_sha256) = 64),
    provenance TEXT NOT NULL CHECK(provenance IN ('bundled_script', 'adopted_release_effects')),
    verified_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE database_metadata (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    compatibility_floor INTEGER NOT NULL,
    generation TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE migration_repair_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    migration_version INTEGER NOT NULL,
    repair_kind TEXT NOT NULL,
    object_type TEXT NOT NULL,
    object_id TEXT NOT NULL,
    outcome TEXT NOT NULL,
    details TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE migration_snapshots (
    migration_version INTEGER NOT NULL,
    table_name TEXT NOT NULL,
    row_count INTEGER NOT NULL,
    captured_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (migration_version, table_name)
);
