-- Migration 026: durable integration credentials, grants, interactions, and OAuth lifecycle.

-- A bot identity exists independently from its installation in a server.
CREATE TABLE bot_installations (
    id                  TEXT PRIMARY KEY,
    bot_user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id           TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    installed_by        TEXT NOT NULL REFERENCES users(id),
    granted_scopes      TEXT NOT NULL,
    authorization_version INTEGER NOT NULL DEFAULT 1 CHECK(authorization_version > 0),
    state               TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','revoked')),
    installed_at        TEXT NOT NULL DEFAULT (datetime('now')),
    revoked_at          TEXT,
    UNIQUE(bot_user_id,server_id)
);
CREATE INDEX idx_bot_installations_server ON bot_installations(server_id,state,bot_user_id);

-- Authorization stamps carry the enclosing server version. Installation grant
-- changes must therefore invalidate that version even when server_members is
-- unchanged (for example, an in-place scope reduction on reinstall).
CREATE TRIGGER bot_installations_authorization_insert
AFTER INSERT ON bot_installations
BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1
    WHERE id = NEW.server_id;
END;

CREATE TRIGGER bot_installations_authorization_update
AFTER UPDATE OF server_id, granted_scopes, state, authorization_version ON bot_installations
WHEN OLD.server_id != NEW.server_id
  OR OLD.granted_scopes != NEW.granted_scopes
  OR OLD.state != NEW.state
  OR OLD.authorization_version != NEW.authorization_version
BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1
    WHERE id IN (OLD.server_id, NEW.server_id);
END;

CREATE TRIGGER bot_installations_authorization_delete
AFTER DELETE ON bot_installations
BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1
    WHERE id = OLD.server_id;
END;

ALTER TABLE bot_tokens ADD COLUMN token_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE bot_tokens ADD COLUMN expires_at TEXT;
ALTER TABLE bot_tokens ADD COLUMN revoked_at TEXT;
CREATE UNIQUE INDEX idx_bot_tokens_credential_id
    ON bot_tokens(credential_id) WHERE credential_id IS NOT NULL;

-- Existing plaintext webhook credentials remain explicitly quarantined until
-- the operator secret migration replaces them with hashes/envelopes.
ALTER TABLE webhooks ADD COLUMN credential_id TEXT;
ALTER TABLE webhooks ADD COLUMN principal_user_id TEXT REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE webhooks ADD COLUMN incoming_token_hash TEXT;
ALTER TABLE webhooks ADD COLUMN signing_key_id TEXT;
ALTER TABLE webhooks ADD COLUMN signing_ciphertext TEXT;
ALTER TABLE webhooks ADD COLUMN credential_state TEXT NOT NULL DEFAULT 'legacy_plaintext';
ALTER TABLE webhooks ADD COLUMN grant_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE webhooks ADD COLUMN revoked_at TEXT;
ALTER TABLE webhooks ADD COLUMN last_delivery_at TEXT;
ALTER TABLE webhooks ADD COLUMN last_safe_error_code TEXT;
CREATE UNIQUE INDEX idx_webhooks_credential_id
    ON webhooks(credential_id) WHERE credential_id IS NOT NULL;

CREATE TABLE webhook_deliveries (
    id                  TEXT PRIMARY KEY,
    webhook_id          TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event_sequence      INTEGER REFERENCES event_log(event_sequence) ON DELETE SET NULL,
    external_job_id     TEXT UNIQUE REFERENCES external_jobs(id) ON DELETE SET NULL,
    delivery_id         TEXT NOT NULL UNIQUE,
    event_type          TEXT NOT NULL,
    event_version       INTEGER NOT NULL CHECK(event_version > 0),
    payload_json        TEXT NOT NULL,
    state               TEXT NOT NULL DEFAULT 'pending'
                            CHECK(state IN ('pending','leased','delivered','failed','cancelled')),
    attempt_count       INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count >= 0),
    last_status         INTEGER,
    safe_error_code     TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    delivered_at        TEXT
);
CREATE INDEX idx_webhook_deliveries_status
    ON webhook_deliveries(webhook_id,state,created_at,id);

-- Invocation ownership and reply capability are durable, one-use state.
ALTER TABLE interactions ADD COLUMN application_user_id TEXT REFERENCES users(id);
ALTER TABLE interactions ADD COLUMN expires_at TEXT;
ALTER TABLE interactions ADD COLUMN response_state TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE interactions ADD COLUMN response_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE interactions ADD COLUMN reply_credential_id TEXT;
ALTER TABLE interactions ADD COLUMN reply_token_hash TEXT;
ALTER TABLE interactions ADD COLUMN response_message_id TEXT REFERENCES messages(id) ON DELETE SET NULL;
ALTER TABLE interactions ADD COLUMN ephemeral_response_json TEXT;
ALTER TABLE interactions ADD COLUMN response_expires_at TEXT;
ALTER TABLE interactions ADD COLUMN responded_at TEXT;
CREATE UNIQUE INDEX idx_interactions_reply_credential
    ON interactions(reply_credential_id) WHERE reply_credential_id IS NOT NULL;
CREATE INDEX idx_interactions_expiry ON interactions(response_state,expires_at,id);

-- New OAuth clients use hashed confidential credentials. Legacy plaintext is
-- retained only as an explicit migration state and is never a valid new-client default.
ALTER TABLE oauth2_apps ADD COLUMN client_type TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE oauth2_apps ADD COLUMN secret_credential_id TEXT;
ALTER TABLE oauth2_apps ADD COLUMN client_secret_hash TEXT;
ALTER TABLE oauth2_apps ADD COLUMN credential_state TEXT NOT NULL DEFAULT 'legacy_plaintext';
ALTER TABLE oauth2_apps ADD COLUMN revoked_at TEXT;
CREATE UNIQUE INDEX idx_oauth2_apps_secret_credential
    ON oauth2_apps(secret_credential_id) WHERE secret_credential_id IS NOT NULL;

CREATE TABLE oauth2_codes (
    id                  TEXT PRIMARY KEY,
    code_hash           TEXT NOT NULL UNIQUE,
    app_id              TEXT NOT NULL REFERENCES oauth2_apps(id) ON DELETE CASCADE,
    user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id           TEXT REFERENCES servers(id) ON DELETE CASCADE,
    redirect_uri        TEXT NOT NULL,
    scopes              TEXT NOT NULL,
    code_challenge      TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL CHECK(code_challenge_method='S256'),
    expires_at          TEXT NOT NULL,
    consumed_at         TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_oauth2_codes_expiry ON oauth2_codes(expires_at,consumed_at);

CREATE TABLE oauth2_grants (
    id                  TEXT PRIMARY KEY,
    app_id              TEXT NOT NULL REFERENCES oauth2_apps(id) ON DELETE CASCADE,
    user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id           TEXT REFERENCES servers(id) ON DELETE CASCADE,
    scopes              TEXT NOT NULL,
    grant_version       INTEGER NOT NULL DEFAULT 1 CHECK(grant_version > 0),
    state               TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','revoked')),
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    revoked_at          TEXT,
    UNIQUE(app_id,user_id,server_id)
);

CREATE TABLE oauth2_tokens (
    id                  TEXT PRIMARY KEY,
    grant_id            TEXT NOT NULL REFERENCES oauth2_grants(id) ON DELETE CASCADE,
    token_family_id     TEXT NOT NULL,
    access_token_hash   TEXT NOT NULL UNIQUE,
    refresh_token_hash  TEXT UNIQUE,
    scopes              TEXT NOT NULL,
    issued_at           TEXT NOT NULL DEFAULT (datetime('now')),
    access_expires_at   TEXT NOT NULL,
    refresh_expires_at  TEXT,
    rotated_to_id       TEXT REFERENCES oauth2_tokens(id),
    revoked_at          TEXT,
    reuse_detected_at   TEXT
);
CREATE INDEX idx_oauth2_tokens_family ON oauth2_tokens(token_family_id,issued_at);
CREATE INDEX idx_oauth2_tokens_grant ON oauth2_tokens(grant_id,revoked_at);

ALTER TABLE channels ADD COLUMN atproto_publication_enabled INTEGER NOT NULL DEFAULT 0;
CREATE TABLE atproto_publication_grants (
    user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id          TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    enabled             INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    grant_version       INTEGER NOT NULL DEFAULT 1 CHECK(grant_version > 0),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(user_id,channel_id)
);
CREATE TABLE atproto_publications (
    id                  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL,
    source_message_id   TEXT NOT NULL,
    source_version      INTEGER NOT NULL CHECK(source_version > 0),
    destination         TEXT NOT NULL,
    collection          TEXT NOT NULL,
    record_key          TEXT NOT NULL,
    remote_uri          TEXT,
    remote_cid          TEXT,
    status              TEXT NOT NULL DEFAULT 'pending'
                            CHECK(status IN ('pending','published','update_pending','delete_pending','deleted','failed','cancelled')),
    safe_error_code     TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id,source_message_id,destination),
    UNIQUE(user_id,collection,record_key)
);
CREATE INDEX idx_atproto_publications_status
    ON atproto_publications(status,updated_at,id);
CREATE TRIGGER atproto_publication_source_changed
AFTER UPDATE OF content,deleted_at,entity_version ON messages
WHEN EXISTS(SELECT 1 FROM atproto_publications p WHERE p.source_message_id=NEW.id)
BEGIN
    UPDATE atproto_publications
       SET source_version=NEW.entity_version,
           status=CASE WHEN NEW.deleted_at IS NULL THEN 'update_pending' ELSE 'delete_pending' END,
           updated_at=datetime('now')
     WHERE source_message_id=NEW.id AND status NOT IN ('deleted','cancelled');
    INSERT OR IGNORE INTO external_jobs
        (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json)
    SELECT lower(hex(randomblob(16))),
           'atproto-publication:' || p.id || ':' || NEW.entity_version,
           CASE WHEN NEW.deleted_at IS NULL THEN 'atproto_update' ELSE 'atproto_delete' END,
           p.id,NEW.entity_version,
           'atproto-user:' || p.user_id || ':' || COALESCE((
               SELECT g.grant_version FROM atproto_publication_grants g
                WHERE g.user_id=p.user_id AND g.channel_id=NEW.channel_id
           ),0),
           json_object('publication_id',p.id)
      FROM atproto_publications p
     WHERE p.source_message_id=NEW.id AND p.status IN ('update_pending','delete_pending');
END;

CREATE TABLE credential_rotation_state (
    singleton           INTEGER PRIMARY KEY CHECK(singleton=1),
    old_key_id          TEXT NOT NULL,
    new_key_id          TEXT NOT NULL,
    old_key_backup      TEXT NOT NULL,
    durable_replacement TEXT NOT NULL,
    phase               TEXT NOT NULL CHECK(phase IN ('prepared','database_committed','activated')),
    started_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE credential_rotation_history (
    old_key_id          TEXT NOT NULL,
    new_key_id          TEXT NOT NULL,
    old_key_backup      TEXT NOT NULL,
    durable_replacement TEXT NOT NULL,
    started_at          TEXT NOT NULL,
    activated_at        TEXT NOT NULL,
    PRIMARY KEY(old_key_id,new_key_id)
);

INSERT INTO schema_version(version,applied_at) VALUES (26,datetime('now'));
