-- Migration 020: canonical conversations and ordered message entities.

CREATE TABLE conversations (
    id                    TEXT PRIMARY KEY,
    kind                  TEXT NOT NULL CHECK(kind IN ('channel', 'direct')),
    server_id             TEXT REFERENCES servers(id) ON DELETE CASCADE,
    channel_id            TEXT REFERENCES channels(id) ON DELETE CASCADE,
    next_message_sequence INTEGER NOT NULL DEFAULT 0 CHECK(next_message_sequence >= 0),
    created_at            TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK((kind = 'channel' AND server_id IS NOT NULL AND channel_id IS NOT NULL)
       OR (kind = 'direct' AND server_id IS NULL AND channel_id IS NULL)),
    UNIQUE(channel_id)
);

INSERT INTO conversations(id,kind,server_id,channel_id)
SELECT 'channel:' || hex(CAST(id AS BLOB)), 'channel', server_id, id
FROM channels;

CREATE TRIGGER create_channel_conversation
AFTER INSERT ON channels
BEGIN
    INSERT INTO conversations(id,kind,server_id,channel_id)
    VALUES('channel:' || hex(CAST(NEW.id AS BLOB)),'channel',NEW.server_id,NEW.id);
END;

ALTER TABLE messages ADD COLUMN conversation_id TEXT REFERENCES conversations(id);
ALTER TABLE messages ADD COLUMN conversation_sequence INTEGER;
ALTER TABLE messages ADD COLUMN content_format TEXT NOT NULL DEFAULT 'legacy_unknown'
    CHECK(content_format IN ('legacy_unknown', 'plain', 'markdown'));
ALTER TABLE messages ADD COLUMN entity_version INTEGER NOT NULL DEFAULT 1
    CHECK(entity_version > 0);

UPDATE messages
SET conversation_id = (
    SELECT id FROM conversations WHERE channel_id = messages.channel_id
)
WHERE channel_id IS NOT NULL
  AND EXISTS(SELECT 1 FROM conversations WHERE channel_id = messages.channel_id);

WITH ordered AS (
    SELECT id,
           ROW_NUMBER() OVER (
               PARTITION BY conversation_id
               ORDER BY created_at, id
           ) AS sequence
    FROM messages
    WHERE conversation_id IS NOT NULL
)
UPDATE messages
SET conversation_sequence = (SELECT sequence FROM ordered WHERE ordered.id = messages.id)
WHERE id IN (SELECT id FROM ordered);

UPDATE conversations
SET next_message_sequence = COALESCE((
    SELECT MAX(conversation_sequence)
    FROM messages
    WHERE messages.conversation_id = conversations.id
), 0);

CREATE UNIQUE INDEX idx_messages_conversation_sequence
    ON messages(conversation_id, conversation_sequence)
    WHERE conversation_id IS NOT NULL AND conversation_sequence IS NOT NULL;
CREATE INDEX idx_messages_conversation_created
    ON messages(conversation_id, created_at, id);

INSERT INTO migration_repair_log(
    migration_version,repair_kind,object_type,object_id,outcome,details
)
SELECT 20,'conversation_backfill','message',id,'preserved_unresolved',
       CASE
           WHEN channel_id IS NULL AND target_user_id IS NOT NULL
               THEN 'historical direct message deferred to migration 022 participant repair'
           WHEN channel_id IS NULL
               THEN 'message has neither a channel nor a direct target'
           ELSE 'message channel does not resolve to a canonical conversation'
       END
FROM messages
WHERE conversation_id IS NULL;

CREATE TABLE message_mentions (
    message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL CHECK(ordinal >= 0),
    mention_kind TEXT NOT NULL CHECK(mention_kind IN ('user', 'role', 'everyone')),
    target_id    TEXT,
    start_byte   INTEGER NOT NULL CHECK(start_byte >= 0),
    end_byte     INTEGER NOT NULL CHECK(end_byte >= start_byte),
    PRIMARY KEY(message_id, ordinal),
    CHECK((mention_kind = 'everyone' AND target_id IS NULL)
       OR (mention_kind != 'everyone' AND target_id IS NOT NULL))
);
CREATE INDEX idx_message_mentions_target
    ON message_mentions(mention_kind, target_id, message_id);

ALTER TABLE read_states ADD COLUMN conversation_sequence INTEGER NOT NULL DEFAULT 0
    CHECK(conversation_sequence >= 0);
UPDATE read_states
SET conversation_sequence = COALESCE((
    SELECT conversation_sequence FROM messages WHERE id = read_states.last_read_message_id
), 0);
