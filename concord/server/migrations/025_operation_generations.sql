-- Migration 025: client operation epochs independent from database replay generations.

CREATE TABLE operation_generations (
    generation TEXT PRIMARY KEY CHECK(length(generation) BETWEEN 16 AND 128),
    issued_at   INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL CHECK(expires_at > issued_at)
);

-- Reuse the old database generation for the first epoch so receipts created by
-- earlier builds remain discoverable after upgrading or restoring a backup.
INSERT INTO operation_generations(generation,issued_at,expires_at)
SELECT generation,unixepoch(),unixepoch()+604800
FROM database_metadata WHERE singleton=1;
INSERT INTO operation_generations(generation,issued_at,expires_at)
SELECT lower(hex(randomblob(16))),unixepoch(),unixepoch()+604800
WHERE NOT EXISTS(SELECT 1 FROM operation_generations);

CREATE TABLE operation_generation_state (
    singleton          INTEGER PRIMARY KEY CHECK(singleton=1),
    current_generation TEXT NOT NULL REFERENCES operation_generations(generation)
);
INSERT INTO operation_generation_state(singleton,current_generation)
SELECT 1,generation FROM operation_generations LIMIT 1;

CREATE INDEX idx_operation_generations_expiry
    ON operation_generations(expires_at,generation);

-- A retained receipt wins before epoch validation. A new epoch must never make
-- the same principal/client operation ID eligible for a second canonical write.
CREATE UNIQUE INDEX idx_command_receipts_principal_client
    ON command_receipts(principal_id,client_message_id);

INSERT INTO schema_version (version, applied_at) VALUES (25, datetime('now'));
