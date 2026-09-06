-- OAuth 2.0 authorization-code consent and normalized grant identity.

ALTER TABLE oauth2_grants ADD COLUMN resource_key TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX oauth2_grants_normalized_resource
    ON oauth2_grants(app_id,user_id,resource_key);

CREATE TABLE oauth2_consent_requests (
    id_hash         TEXT PRIMARY KEY,
    app_id          TEXT NOT NULL REFERENCES oauth2_apps(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id       TEXT REFERENCES servers(id) ON DELETE CASCADE,
    redirect_uri    TEXT NOT NULL,
    scopes          TEXT NOT NULL,
    state           TEXT,
    code_challenge  TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    consumed_at     TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX oauth2_consent_expiry
    ON oauth2_consent_requests(expires_at,consumed_at);

INSERT INTO schema_version(version,applied_at) VALUES (29,datetime('now'));
