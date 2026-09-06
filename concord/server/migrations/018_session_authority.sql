-- Migration 018: durable shared credential and session authority

ALTER TABLE users ADD COLUMN disabled_at TEXT;

CREATE TABLE auth_credentials (
    id               TEXT PRIMARY KEY,
    user_id          TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('web_session', 'irc_token', 'bot_token')),
    token_id         TEXT UNIQUE,
    secret_hash      TEXT,
    scopes           TEXT NOT NULL,
    expires_at       INTEGER,
    revoked_at       INTEGER,
    version          INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    legacy_source_id TEXT,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch()),
    last_used_at     INTEGER,
    CHECK ((kind = 'web_session' AND secret_hash IS NULL AND expires_at IS NOT NULL)
        OR (kind != 'web_session' AND secret_hash IS NOT NULL))
);

CREATE INDEX idx_auth_credentials_user_active
    ON auth_credentials(user_id, revoked_at, expires_at);
CREATE INDEX idx_auth_credentials_kind_legacy
    ON auth_credentials(kind, legacy_source_id);
CREATE TRIGGER auth_credentials_versioned_authority_update
BEFORE UPDATE OF scopes, expires_at, revoked_at ON auth_credentials
WHEN NEW.version <= OLD.version
BEGIN
    SELECT RAISE(ABORT, 'credential authority changes require a version increment');
END;
CREATE TRIGGER auth_credentials_immutable_identity
BEFORE UPDATE OF id, user_id, kind, token_id, secret_hash ON auth_credentials
BEGIN
    SELECT RAISE(ABORT, 'credential identity is immutable');
END;

-- Existing IRC and bot secrets remain valid until rotation, but are explicitly
-- marked as legacy because their raw value cannot reveal an indexed token ID.
INSERT INTO auth_credentials
    (id, user_id, kind, secret_hash, scopes, legacy_source_id, created_at, last_used_at)
SELECT 'irc:' || id, user_id, 'irc_token', token_hash, 'irc', id,
       unixepoch(created_at), unixepoch(last_used)
FROM irc_tokens;

INSERT INTO auth_credentials
    (id, user_id, kind, secret_hash, scopes, legacy_source_id, created_at, last_used_at)
SELECT 'bot:' || id, user_id, 'bot_token', token_hash, scopes, id,
       unixepoch(created_at), unixepoch(last_used)
FROM bot_tokens;

ALTER TABLE irc_tokens ADD COLUMN credential_id TEXT REFERENCES auth_credentials(id);
ALTER TABLE irc_tokens ADD COLUMN token_id TEXT;
CREATE UNIQUE INDEX idx_irc_tokens_token_id ON irc_tokens(token_id) WHERE token_id IS NOT NULL;
UPDATE irc_tokens SET credential_id = 'irc:' || id;

ALTER TABLE bot_tokens ADD COLUMN credential_id TEXT REFERENCES auth_credentials(id);
ALTER TABLE bot_tokens ADD COLUMN token_id TEXT;
CREATE UNIQUE INDEX idx_bot_tokens_token_id ON bot_tokens(token_id) WHERE token_id IS NOT NULL;
UPDATE bot_tokens SET credential_id = 'bot:' || id;

CREATE TABLE bot_ownership (
    bot_user_id     TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    owner_user_id   TEXT REFERENCES users(id) ON DELETE RESTRICT,
    repair_required INTEGER NOT NULL DEFAULT 0 CHECK (repair_required IN (0, 1)),
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    CHECK ((owner_user_id IS NULL AND repair_required = 1)
        OR (owner_user_id IS NOT NULL AND repair_required = 0))
);
CREATE INDEX idx_bot_ownership_owner ON bot_ownership(owner_user_id);
INSERT INTO bot_ownership(bot_user_id, owner_user_id, repair_required)
SELECT id, NULL, 1 FROM users WHERE is_bot = 1;
