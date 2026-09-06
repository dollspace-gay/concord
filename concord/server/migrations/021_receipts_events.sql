-- Migration 021: idempotent command receipts and durable replay descriptors.

CREATE TABLE entity_versions (
    entity_type TEXT NOT NULL,
    entity_id   TEXT NOT NULL,
    version     INTEGER NOT NULL CHECK(version > 0),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(entity_type, entity_id)
);

INSERT INTO entity_versions(entity_type,entity_id,version)
SELECT 'message', id, entity_version FROM messages;

CREATE TABLE event_log (
    event_sequence        INTEGER PRIMARY KEY AUTOINCREMENT,
    database_generation   TEXT NOT NULL,
    conversation_id       TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    event_kind            TEXT NOT NULL CHECK(length(event_kind) BETWEEN 1 AND 64),
    entity_type           TEXT NOT NULL,
    entity_id             TEXT NOT NULL,
    entity_version        INTEGER NOT NULL CHECK(entity_version > 0),
    authorization_version INTEGER NOT NULL CHECK(authorization_version >= 0),
    actor_id              TEXT NOT NULL,
    descriptor_json       TEXT NOT NULL CHECK(json_valid(descriptor_json)),
    created_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f','now'))
);
CREATE INDEX idx_event_log_generation_sequence
    ON event_log(database_generation,event_sequence);
CREATE INDEX idx_event_log_conversation_sequence
    ON event_log(conversation_id,event_sequence);
CREATE INDEX idx_event_log_created ON event_log(created_at,event_sequence);

CREATE TABLE command_receipts (
    principal_id          TEXT NOT NULL,
    operation_generation  TEXT NOT NULL,
    client_message_id     TEXT NOT NULL,
    request_id            TEXT NOT NULL,
    operation_kind        TEXT NOT NULL CHECK(operation_kind IN (
                              'send','edit','delete','reaction_add','reaction_remove','read'
                          )),
    payload_fingerprint   TEXT NOT NULL CHECK(length(payload_fingerprint) = 64),
    conversation_id       TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    canonical_message_id  TEXT,
    conversation_sequence INTEGER,
    event_sequence        INTEGER NOT NULL,
    entity_version        INTEGER NOT NULL CHECK(entity_version > 0),
    persisted_at          TEXT NOT NULL,
    response_json         TEXT NOT NULL CHECK(json_valid(response_json)),
    PRIMARY KEY(principal_id,operation_generation,client_message_id)
);
CREATE INDEX idx_command_receipts_message
    ON command_receipts(canonical_message_id);

CREATE TABLE delivery_outbox (
    event_sequence INTEGER PRIMARY KEY REFERENCES event_log(event_sequence) ON DELETE CASCADE,
    available_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f','now')),
    attempts       INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    last_error     TEXT,
    claimed_until  TEXT,
    completed_at   TEXT
);
CREATE INDEX idx_delivery_outbox_pending
    ON delivery_outbox(completed_at,available_at,event_sequence);

CREATE TRIGGER retain_event_with_pending_delivery
BEFORE DELETE ON event_log
WHEN EXISTS(
    SELECT 1 FROM delivery_outbox
    WHERE event_sequence = OLD.event_sequence AND completed_at IS NULL
)
BEGIN
    SELECT RAISE(ABORT, 'event has pending durable delivery work');
END;

CREATE TABLE event_retention_state (
    singleton                INTEGER PRIMARY KEY CHECK(singleton = 1),
    retained_from_sequence   INTEGER NOT NULL DEFAULT 0 CHECK(retained_from_sequence >= 0),
    retention_seconds        INTEGER NOT NULL DEFAULT 604800 CHECK(retention_seconds >= 3600),
    dispatcher_high_water    INTEGER NOT NULL DEFAULT 0 CHECK(dispatcher_high_water >= 0),
    updated_at               TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO event_retention_state(singleton) VALUES(1);
