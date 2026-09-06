-- Feature-integrity state for organization and community workflows.

ALTER TABLE servers ADD COLUMN rules_version INTEGER NOT NULL DEFAULT 1 CHECK(rules_version > 0);
ALTER TABLE server_members ADD COLUMN accepted_rules_version INTEGER NOT NULL DEFAULT 0 CHECK(accepted_rules_version >= 0);
UPDATE server_members
SET accepted_rules_version = CASE
    WHEN rules_accepted <> 0 THEN COALESCE((SELECT rules_version FROM servers WHERE id=server_id), 1)
    ELSE 0
END;

-- Existing opaque snapshots stay legacy until explicitly validated. New writers use v1.
ALTER TABLE server_templates ADD COLUMN format_version INTEGER NOT NULL DEFAULT 0 CHECK(format_version IN (0,1));
ALTER TABLE server_events ADD COLUMN integrity_state TEXT NOT NULL DEFAULT 'legacy_unverified'
    CHECK(integrity_state IN ('active','legacy_unverified','quarantined'));
UPDATE server_events SET integrity_state='active'
WHERE julianday(start_time) IS NOT NULL
  AND (end_time IS NULL OR (julianday(end_time) IS NOT NULL AND julianday(end_time)>julianday(start_time)))
  AND (channel_id IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=server_events.channel_id AND c.server_id=server_events.server_id));
UPDATE server_events SET integrity_state='quarantined' WHERE integrity_state='legacy_unverified';
ALTER TABLE user_presence ADD COLUMN requested_status TEXT NOT NULL DEFAULT 'online'
    CHECK(requested_status IN ('online','idle','dnd','invisible'));
UPDATE user_presence SET requested_status=CASE WHEN status IN ('online','idle','dnd','invisible') THEN status ELSE 'online' END;

ALTER TABLE channels ADD COLUMN thread_last_activity_at TEXT;
ALTER TABLE channels ADD COLUMN thread_archive_due_at TEXT;
ALTER TABLE channels ADD COLUMN thread_archive_reason TEXT
    CHECK(thread_archive_reason IN ('manual','inactivity'));
ALTER TABLE channels ADD COLUMN thread_state_version INTEGER NOT NULL DEFAULT 1
    CHECK(thread_state_version > 0);
UPDATE channels
SET thread_last_activity_at=created_at,
    thread_archive_due_at=datetime(created_at,'+' || thread_auto_archive_minutes || ' minutes')
WHERE channel_type IN ('public_thread','private_thread') AND archived=0;

CREATE TABLE server_folders (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 100),
    color       TEXT,
    position    INTEGER NOT NULL CHECK(position >= 0),
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, position)
);

CREATE TABLE irc_nick_reservations (
    nick_casefold TEXT PRIMARY KEY,
    nickname      TEXT NOT NULL UNIQUE,
    user_id       TEXT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK(length(nickname) BETWEEN 1 AND 32),
    CHECK(nickname NOT GLOB '*[^A-Za-z0-9_.-]*')
);
WITH ranked AS (
    SELECT id,
           'u-' || lower(substr(hex(CAST(id AS BLOB)),1,24)) AS base,
           ROW_NUMBER() OVER(
               PARTITION BY lower(substr(hex(CAST(id AS BLOB)),1,24)) ORDER BY id
           ) AS ordinal
    FROM users
)
INSERT INTO irc_nick_reservations(nick_casefold,nickname,user_id)
SELECT CASE WHEN ordinal=1 THEN base ELSE substr(base,1,27) || '-' || ordinal END,
       CASE WHEN ordinal=1 THEN base ELSE substr(base,1,27) || '-' || ordinal END,
       id
FROM ranked;

CREATE TABLE server_folder_items (
    folder_id   TEXT NOT NULL REFERENCES server_folders(id) ON DELETE CASCADE,
    server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL CHECK(position >= 0),
    PRIMARY KEY(folder_id, server_id),
    UNIQUE(folder_id, position)
);

CREATE TABLE announcement_publications (
    id                  TEXT PRIMARY KEY,
    -- Retained immutable provenance; grants and source messages may later be deleted.
    follow_id           TEXT NOT NULL,
    source_message_id   TEXT NOT NULL,
    target_message_id   TEXT REFERENCES messages(id) ON DELETE SET NULL,
    source_version      INTEGER NOT NULL CHECK(source_version > 0),
    state               TEXT NOT NULL CHECK(state IN ('pending','published','deleted','failed')),
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(follow_id, source_message_id),
    UNIQUE(target_message_id)
);
CREATE INDEX idx_announcement_publications_source ON announcement_publications(source_message_id);

CREATE TRIGGER channel_follow_cancel_pending_publications
BEFORE DELETE ON channel_follows
BEGIN
    UPDATE announcement_publications
       SET state='failed',updated_at=datetime('now')
     WHERE follow_id=OLD.id AND state='pending';
END;

CREATE TRIGGER server_rules_version_on_change
AFTER UPDATE OF rules_text ON servers
WHEN OLD.rules_text IS NOT NEW.rules_text
BEGIN
    UPDATE servers SET rules_version = OLD.rules_version + 1 WHERE id = NEW.id;
END;

CREATE TRIGGER server_event_validate_insert
BEFORE INSERT ON server_events
WHEN julianday(NEW.start_time) IS NULL
  OR (NEW.end_time IS NOT NULL AND (julianday(NEW.end_time) IS NULL OR julianday(NEW.end_time) <= julianday(NEW.start_time)))
  OR (NEW.channel_id IS NOT NULL AND NOT EXISTS(
      SELECT 1 FROM channels c WHERE c.id=NEW.channel_id AND c.server_id=NEW.server_id
  ))
BEGIN
    SELECT RAISE(ABORT, 'invalid server event scope or timestamps');
END;

CREATE TRIGGER server_event_validate_update
BEFORE UPDATE OF server_id,channel_id,start_time,end_time ON server_events
WHEN julianday(NEW.start_time) IS NULL
  OR (NEW.end_time IS NOT NULL AND (julianday(NEW.end_time) IS NULL OR julianday(NEW.end_time) <= julianday(NEW.start_time)))
  OR (NEW.channel_id IS NOT NULL AND NOT EXISTS(
      SELECT 1 FROM channels c WHERE c.id=NEW.channel_id AND c.server_id=NEW.server_id
  ))
BEGIN
    SELECT RAISE(ABORT, 'invalid server event scope or timestamps');
END;

CREATE TRIGGER channel_follow_validate_insert
BEFORE INSERT ON channel_follows
WHEN NEW.source_channel_id = NEW.target_channel_id
  OR NOT EXISTS(SELECT 1 FROM channels WHERE id=NEW.source_channel_id AND is_announcement=1)
BEGIN
    SELECT RAISE(ABORT, 'invalid announcement follow');
END;

INSERT INTO schema_version (version, applied_at) VALUES (24, datetime('now'));
