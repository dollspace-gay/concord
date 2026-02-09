-- Migration 002: Multi-server (guild) support + system admin

-- Version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Servers (guilds)
CREATE TABLE IF NOT EXISTS servers (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    icon_url    TEXT,
    owner_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Server membership
CREATE TABLE IF NOT EXISTS server_members (
    server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL DEFAULT 'member',
    joined_at   TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (server_id, user_id)
);

-- System admin flag on users
ALTER TABLE users ADD COLUMN is_system_admin INTEGER NOT NULL DEFAULT 0;

-- Recreate channels with UUID PK and server_id
CREATE TABLE channels_v2 (
    id           TEXT PRIMARY KEY,
    server_id    TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    topic        TEXT NOT NULL DEFAULT '',
    topic_set_by TEXT,
    topic_set_at TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    is_default   INTEGER NOT NULL DEFAULT 0,
    UNIQUE(server_id, name)
);

INSERT INTO channels_v2 (id, server_id, name, topic, topic_set_by, topic_set_at, created_at, is_default)
    SELECT name, 'default', name, topic, topic_set_by, topic_set_at, created_at, is_default
    FROM channels;

-- Recreate channel_members with channel_id FK
CREATE TABLE channel_members_v2 (
    channel_id   TEXT NOT NULL REFERENCES channels_v2(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role         TEXT NOT NULL DEFAULT 'member',
    joined_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (channel_id, user_id)
);

INSERT INTO channel_members_v2 (channel_id, user_id, role, joined_at)
    SELECT channel_name, user_id, role, joined_at FROM channel_members;

-- Recreate messages with channel_id + server_id
CREATE TABLE messages_v2 (
    id             TEXT PRIMARY KEY,
    server_id      TEXT REFERENCES servers(id) ON DELETE CASCADE,
    channel_id     TEXT REFERENCES channels_v2(id) ON DELETE CASCADE,
    sender_id      TEXT NOT NULL,
    sender_nick    TEXT NOT NULL,
    content        TEXT NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    target_user_id TEXT
);

INSERT INTO messages_v2 (id, server_id, channel_id, sender_id, sender_nick, content, created_at, target_user_id)
    SELECT id, 'default', channel_name, sender_id, sender_nick, content, created_at, target_user_id
    FROM messages;

-- Drop old tables and rename
DROP TABLE IF EXISTS channel_members;
DROP TABLE IF EXISTS messages;
DROP TABLE IF EXISTS channels;

ALTER TABLE channels_v2 RENAME TO channels;
ALTER TABLE channel_members_v2 RENAME TO channel_members;
ALTER TABLE messages_v2 RENAME TO messages;

-- Recreate indexes on new tables
CREATE INDEX IF NOT EXISTS idx_messages_channel_time ON messages(channel_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender_id);
CREATE INDEX IF NOT EXISTS idx_messages_dm ON messages(target_user_id, created_at DESC)
    WHERE target_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_messages_server ON messages(server_id);
CREATE INDEX IF NOT EXISTS idx_channels_server ON channels(server_id);
CREATE INDEX IF NOT EXISTS idx_server_members_user ON server_members(user_id);
CREATE INDEX IF NOT EXISTS idx_server_members_server ON server_members(server_id);

-- Record migration version
INSERT INTO schema_version (version) VALUES (2);
