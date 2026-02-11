-- Migration 012: Integrations & Bots (Phase 8)
-- Adds webhooks, bot accounts, slash commands, message components, OAuth2 apps, rich embeds

-- Bot flag on users
ALTER TABLE users ADD COLUMN is_bot INTEGER NOT NULL DEFAULT 0;

-- Bot tokens for API authentication
CREATE TABLE IF NOT EXISTS bot_tokens (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL DEFAULT 'Default',
    scopes      TEXT NOT NULL DEFAULT 'bot',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    last_used   TEXT
);
CREATE INDEX IF NOT EXISTS idx_bot_tokens_user ON bot_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_bot_tokens_hash ON bot_tokens(token_hash);

-- Webhooks (incoming and outgoing)
CREATE TABLE IF NOT EXISTS webhooks (
    id              TEXT PRIMARY KEY,
    server_id       TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    channel_id      TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    avatar_url      TEXT,
    webhook_type    TEXT NOT NULL CHECK(webhook_type IN ('incoming', 'outgoing')),
    token           TEXT NOT NULL UNIQUE,
    url             TEXT,
    created_by      TEXT NOT NULL REFERENCES users(id),
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_webhooks_server ON webhooks(server_id);
CREATE INDEX IF NOT EXISTS idx_webhooks_channel ON webhooks(channel_id);
CREATE INDEX IF NOT EXISTS idx_webhooks_token ON webhooks(token);

-- Outgoing webhook event subscriptions
CREATE TABLE IF NOT EXISTS webhook_events (
    id          TEXT PRIMARY KEY,
    webhook_id  TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event_type  TEXT NOT NULL,
    UNIQUE(webhook_id, event_type)
);

-- Slash commands registered by bots
CREATE TABLE IF NOT EXISTS slash_commands (
    id              TEXT PRIMARY KEY,
    bot_user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id       TEXT REFERENCES servers(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    options_json    TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(bot_user_id, server_id, name)
);
CREATE INDEX IF NOT EXISTS idx_slash_commands_server ON slash_commands(server_id);
CREATE INDEX IF NOT EXISTS idx_slash_commands_bot ON slash_commands(bot_user_id);

-- Interaction log (slash command invocations, button clicks, etc.)
CREATE TABLE IF NOT EXISTS interactions (
    id              TEXT PRIMARY KEY,
    interaction_type TEXT NOT NULL CHECK(interaction_type IN ('slash_command', 'button', 'select_menu', 'modal_submit')),
    command_id      TEXT REFERENCES slash_commands(id) ON DELETE SET NULL,
    user_id         TEXT NOT NULL REFERENCES users(id),
    server_id       TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    channel_id      TEXT NOT NULL,
    data_json       TEXT NOT NULL DEFAULT '{}',
    responded       INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_interactions_server ON interactions(server_id);

-- Message components stored alongside messages
ALTER TABLE messages ADD COLUMN components_json TEXT;

-- OAuth2 applications (third-party integrations)
CREATE TABLE IF NOT EXISTS oauth2_apps (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    icon_url        TEXT,
    owner_id        TEXT NOT NULL REFERENCES users(id),
    client_secret   TEXT NOT NULL,
    redirect_uris   TEXT NOT NULL DEFAULT '[]',
    scopes          TEXT NOT NULL DEFAULT 'identify',
    is_public       INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_oauth2_apps_owner ON oauth2_apps(owner_id);

-- OAuth2 authorization grants
CREATE TABLE IF NOT EXISTS oauth2_authorizations (
    id              TEXT PRIMARY KEY,
    app_id          TEXT NOT NULL REFERENCES oauth2_apps(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id       TEXT REFERENCES servers(id) ON DELETE CASCADE,
    scopes          TEXT NOT NULL,
    access_token    TEXT NOT NULL UNIQUE,
    refresh_token   TEXT UNIQUE,
    expires_at      TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(app_id, user_id, server_id)
);
CREATE INDEX IF NOT EXISTS idx_oauth2_auth_user ON oauth2_authorizations(user_id);
CREATE INDEX IF NOT EXISTS idx_oauth2_auth_token ON oauth2_authorizations(access_token);

-- Rich embeds on messages (structured embed data for bots)
ALTER TABLE messages ADD COLUMN rich_embeds_json TEXT;
