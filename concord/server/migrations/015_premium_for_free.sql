-- Migration 015: Premium-for-Free Features
-- Cross-server emoji settings, custom stickers, per-server avatars, vanity invite URLs

-- Feature 1: Cross-server emoji settings
ALTER TABLE servers ADD COLUMN allow_external_emoji INTEGER NOT NULL DEFAULT 1;
ALTER TABLE servers ADD COLUMN shareable_emoji INTEGER NOT NULL DEFAULT 1;

-- Feature 2: Custom stickers
CREATE TABLE IF NOT EXISTS stickers (
    id          TEXT PRIMARY KEY,
    server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    image_url   TEXT NOT NULL,
    description TEXT,
    uploader_id TEXT NOT NULL REFERENCES users(id),
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(server_id, name)
);
CREATE INDEX IF NOT EXISTS idx_stickers_server ON stickers(server_id);

-- Feature 4: Per-server avatars
ALTER TABLE server_members ADD COLUMN avatar_url TEXT;

-- Feature 5: Vanity invite URLs
ALTER TABLE servers ADD COLUMN vanity_code TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_servers_vanity ON servers(vanity_code) WHERE vanity_code IS NOT NULL;
