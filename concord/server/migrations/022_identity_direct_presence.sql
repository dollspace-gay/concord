-- Migration 022: stable user aliases and durable direct-message policy state.

CREATE TABLE user_aliases (
    alias TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    alias_kind TEXT NOT NULL CHECK(alias_kind IN ('legacy_id','canonical_id','nickname')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT OR IGNORE INTO user_aliases(alias,user_id,alias_kind)
SELECT id,id,'canonical_id' FROM users;
INSERT OR IGNORE INTO user_aliases(alias,user_id,alias_kind)
SELECT un.nickname,un.user_id,'nickname'
FROM user_nicknames un
WHERE NOT EXISTS(
    SELECT 1 FROM user_nicknames other
    WHERE lower(other.nickname)=lower(un.nickname) AND other.user_id<>un.user_id
);

CREATE TABLE server_aliases (
    alias TEXT PRIMARY KEY COLLATE NOCASE,
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    is_canonical INTEGER NOT NULL DEFAULT 1 CHECK(is_canonical IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
WITH ranked AS (
    SELECT id,lower(substr(hex(CAST(id AS BLOB)),1,20)) base,
           ROW_NUMBER() OVER(
               PARTITION BY lower(substr(hex(CAST(id AS BLOB)),1,20)) ORDER BY id
           ) ordinal
    FROM servers
)
INSERT INTO server_aliases(alias,server_id)
SELECT 's-' || base || '-' || printf('%x',ordinal),id FROM ranked;
INSERT OR IGNORE INTO server_aliases(alias,server_id,is_canonical)
SELECT lower(replace(name,' ','-')),id,0 FROM servers candidate
WHERE lower(replace(name,' ','-'))<>''
  AND lower(replace(name,' ','-')) NOT GLOB '*[^a-z0-9-]*'
  AND (SELECT COUNT(*) FROM servers other
       WHERE lower(replace(other.name,' ','-'))=lower(replace(candidate.name,' ','-')))=1;

CREATE TABLE channel_aliases (
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    alias TEXT NOT NULL COLLATE NOCASE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    is_canonical INTEGER NOT NULL DEFAULT 1 CHECK(is_canonical IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(server_id,alias),
    UNIQUE(channel_id,alias)
);
INSERT INTO channel_aliases(server_id,alias,channel_id)
SELECT server_id,ltrim(lower(name),'#'),id FROM channels candidate
WHERE ltrim(name,'#')<>''
  AND lower(ltrim(name,'#')) NOT GLOB '*[^a-z0-9_-]*'
  AND (SELECT COUNT(*) FROM channels other
       WHERE other.server_id=candidate.server_id
         AND lower(ltrim(other.name,'#'))=lower(ltrim(candidate.name,'#')))=1;

CREATE TABLE user_default_servers (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE conversation_participants (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TEXT NOT NULL DEFAULT (datetime('now')),
    left_at TEXT,
    PRIMARY KEY(conversation_id,user_id)
);

CREATE TABLE direct_conversation_pairs (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    lower_user_id TEXT NOT NULL REFERENCES users(id),
    upper_user_id TEXT NOT NULL REFERENCES users(id),
    CHECK(lower_user_id < upper_user_id),
    UNIQUE(lower_user_id,upper_user_id)
);

CREATE TABLE user_blocks (
    blocker_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(blocker_user_id,blocked_user_id),
    CHECK(blocker_user_id <> blocked_user_id)
);

CREATE TABLE direct_message_preferences (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    allow_from TEXT NOT NULL DEFAULT 'shared_server'
        CHECK(allow_from IN ('everyone','shared_server','none')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TEMP TABLE _legacy_direct_pairs AS
SELECT DISTINCT
       CASE WHEN sender_id < target_user_id THEN sender_id ELSE target_user_id END lower_user_id,
       CASE WHEN sender_id < target_user_id THEN target_user_id ELSE sender_id END upper_user_id
FROM messages
WHERE conversation_id IS NULL AND channel_id IS NULL
  AND sender_id IS NOT NULL AND target_user_id IS NOT NULL
  AND sender_id<>target_user_id
  AND EXISTS(SELECT 1 FROM users WHERE id=messages.sender_id)
  AND EXISTS(SELECT 1 FROM users WHERE id=messages.target_user_id);

INSERT INTO conversations(id,kind)
SELECT 'direct:' || hex(CAST(lower_user_id AS BLOB)) || ':' || hex(CAST(upper_user_id AS BLOB)),
       'direct'
FROM _legacy_direct_pairs;
INSERT INTO direct_conversation_pairs(conversation_id,lower_user_id,upper_user_id)
SELECT 'direct:' || hex(CAST(lower_user_id AS BLOB)) || ':' || hex(CAST(upper_user_id AS BLOB)),
       lower_user_id,upper_user_id
FROM _legacy_direct_pairs;
INSERT INTO conversation_participants(conversation_id,user_id)
SELECT conversation_id,lower_user_id FROM direct_conversation_pairs
UNION ALL
SELECT conversation_id,upper_user_id FROM direct_conversation_pairs;

UPDATE messages
SET conversation_id=(
        SELECT pair.conversation_id FROM direct_conversation_pairs pair
        WHERE pair.lower_user_id=CASE WHEN messages.sender_id < messages.target_user_id
                                     THEN messages.sender_id ELSE messages.target_user_id END
          AND pair.upper_user_id=CASE WHEN messages.sender_id < messages.target_user_id
                                     THEN messages.target_user_id ELSE messages.sender_id END
    )
WHERE conversation_id IS NULL AND channel_id IS NULL
  AND sender_id IS NOT NULL AND target_user_id IS NOT NULL;

WITH ordered AS (
    SELECT id,ROW_NUMBER() OVER(PARTITION BY conversation_id ORDER BY created_at,id) sequence
    FROM messages WHERE conversation_id LIKE 'direct:%' AND conversation_sequence IS NULL
)
UPDATE messages
SET conversation_sequence=(SELECT sequence FROM ordered WHERE ordered.id=messages.id)
WHERE id IN (SELECT id FROM ordered);
UPDATE conversations
SET next_message_sequence=COALESCE((
    SELECT MAX(conversation_sequence) FROM messages
    WHERE messages.conversation_id=conversations.id
),0)
WHERE kind='direct';

INSERT INTO migration_repair_log(
    migration_version,repair_kind,object_type,object_id,outcome,details
)
SELECT 22,'direct_participant_backfill','message',id,'preserved_unresolved',
       'direct target is missing, self-directed, ambiguous, or does not resolve to active users'
FROM messages
WHERE conversation_id IS NULL AND channel_id IS NULL;
DROP TABLE _legacy_direct_pairs;
