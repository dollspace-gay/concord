-- Phase 9: AT Protocol Deep Integration

-- Profile sync fields on oauth_accounts
ALTER TABLE oauth_accounts ADD COLUMN bsky_handle TEXT;
ALTER TABLE oauth_accounts ADD COLUMN bsky_display_name TEXT;
ALTER TABLE oauth_accounts ADD COLUMN bsky_description TEXT;
ALTER TABLE oauth_accounts ADD COLUMN bsky_banner_url TEXT;
ALTER TABLE oauth_accounts ADD COLUMN bsky_followers_count INTEGER;
ALTER TABLE oauth_accounts ADD COLUMN bsky_follows_count INTEGER;
ALTER TABLE oauth_accounts ADD COLUMN last_profile_sync TEXT;

-- Track shared posts to prevent duplicates
CREATE TABLE IF NOT EXISTS bsky_shared_posts (
    id              TEXT PRIMARY KEY,
    message_id      TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    at_uri          TEXT NOT NULL,
    cid             TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(message_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_bsky_shared_user ON bsky_shared_posts(user_id);

-- Opt-in AT Protocol record sync per user
ALTER TABLE users ADD COLUMN atproto_sync_enabled INTEGER NOT NULL DEFAULT 0;
