-- Private instance-owned media lifecycle and auditable legacy import state.
ALTER TABLE attachments ADD COLUMN conversation_id TEXT REFERENCES conversations(id);
ALTER TABLE attachments ADD COLUMN media_purpose TEXT NOT NULL DEFAULT 'message'
    CHECK (media_purpose IN ('message','emoji','sticker','user_avatar','user_banner','server_avatar','server_member_avatar'));
ALTER TABLE attachments ADD COLUMN managed_server_id TEXT REFERENCES servers(id) ON DELETE CASCADE;
ALTER TABLE attachments ADD COLUMN managed_user_id TEXT REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE attachments ADD COLUMN media_state TEXT NOT NULL DEFAULT 'legacy_external'
    CHECK (media_state IN ('staging','ready','attached','deleting','deleted','failed','legacy_external'));
ALTER TABLE attachments ADD COLUMN state_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE attachments ADD COLUMN storage_backend TEXT;
ALTER TABLE attachments ADD COLUMN storage_key TEXT;
ALTER TABLE attachments ADD COLUMN sha256 TEXT;
ALTER TABLE attachments ADD COLUMN reserved_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE attachments ADD COLUMN upload_updated_at TEXT;
ALTER TABLE attachments ADD COLUMN ready_at TEXT;
ALTER TABLE attachments ADD COLUMN delete_after TEXT;
ALTER TABLE attachments ADD COLUMN previously_public INTEGER NOT NULL DEFAULT 0;
ALTER TABLE attachments ADD COLUMN import_outcome TEXT;

UPDATE attachments
SET media_state='legacy_external',
    upload_updated_at=created_at,
    previously_public=CASE WHEN blob_url IS NULL THEN 0 ELSE 1 END,
    import_outcome=CASE WHEN blob_url IS NULL THEN 'missing_local_locator' ELSE 'pending' END;

CREATE INDEX idx_attachments_lifecycle ON attachments(media_state, delete_after);
CREATE INDEX idx_attachments_conversation ON attachments(conversation_id);
CREATE UNIQUE INDEX idx_attachments_storage_key ON attachments(storage_key) WHERE storage_key IS NOT NULL;

CREATE TABLE media_import_ledger (
    attachment_id TEXT PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
    previous_url TEXT NOT NULL,
    previous_cid TEXT,
    expected_size INTEGER,
    actual_size INTEGER,
    sha256 TEXT,
    record_uri TEXT,
    reference_outcome TEXT CHECK (reference_outcome IN ('not_checked','confirmed','missing_credentials','missing_data','ambiguous_reference')),
    outcome TEXT NOT NULL CHECK (outcome IN ('pending','importing','imported','download_failed','size_mismatch','missing_credentials','missing_data','ambiguous_reference')),
    detail_code TEXT,
    claim_token TEXT,
    claim_until TEXT,
    attempted_at TEXT,
    completed_at TEXT
);
CREATE INDEX idx_media_import_due ON media_import_ledger(outcome,claim_until,attachment_id);

-- Provider secrets are stored as one authenticated envelope. Plaintext legacy
-- columns remain readable only for the explicit operator migration tool.
ALTER TABLE oauth_accounts ADD COLUMN credential_key_id TEXT;
ALTER TABLE oauth_accounts ADD COLUMN credential_ciphertext TEXT;
ALTER TABLE oauth_accounts ADD COLUMN credential_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE oauth_accounts ADD COLUMN credential_state TEXT NOT NULL DEFAULT 'legacy_plaintext'
    CHECK (credential_state IN ('legacy_plaintext','active','corrupt','key_unavailable','missing_data','revoked'));

INSERT OR IGNORE INTO media_import_ledger(attachment_id,previous_url,previous_cid,expected_size,outcome)
SELECT id,blob_url,blob_cid,file_size,'pending' FROM attachments WHERE blob_url IS NOT NULL;

CREATE TABLE external_jobs (
    id TEXT PRIMARY KEY,
    deduplication_key TEXT NOT NULL UNIQUE,
    operation_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    resource_version INTEGER NOT NULL,
    destination_grant TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending','leased','succeeded','failed','cancelled')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL DEFAULT (datetime('now')),
    lease_owner TEXT,
    lease_token TEXT,
    lease_until TEXT,
    safe_error_code TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_external_jobs_due ON external_jobs(state,next_attempt_at,lease_until);

CREATE TABLE pending_atproto_oauth (
    state_hash TEXT PRIMARY KEY,
    credential_key_id TEXT NOT NULL,
    credential_ciphertext TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending','consumed','expired','corrupt')),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    safe_error_code TEXT
);
CREATE INDEX idx_pending_atproto_oauth_state ON pending_atproto_oauth(state,expires_at);

INSERT OR IGNORE INTO schema_version (version) VALUES (23);
