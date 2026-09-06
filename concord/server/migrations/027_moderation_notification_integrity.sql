-- Migration 027: durable moderation identity and canonical notification scopes.

-- Audit records retain the actor identity rendered at action time even if the
-- user account is later removed. The stable actor ID remains queryable but is
-- deliberately no longer an ON DELETE CASCADE foreign key.
DROP INDEX idx_audit_log_server;
DROP INDEX idx_audit_log_actor;
DROP INDEX idx_audit_log_target;
ALTER TABLE audit_log RENAME TO audit_log_legacy_027;

CREATE TABLE audit_log (
    id                      TEXT PRIMARY KEY,
    server_id               TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    actor_id                TEXT NOT NULL,
    actor_username_snapshot TEXT NOT NULL,
    actor_avatar_snapshot   TEXT,
    action_type             TEXT NOT NULL,
    target_type             TEXT,
    target_id               TEXT,
    reason                  TEXT,
    changes                 TEXT,
    created_at              TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO audit_log(
    id,server_id,actor_id,actor_username_snapshot,actor_avatar_snapshot,
    action_type,target_type,target_id,reason,changes,created_at
)
SELECT a.id,a.server_id,a.actor_id,COALESCE(u.username,a.actor_id),u.avatar_url,
       a.action_type,a.target_type,a.target_id,a.reason,a.changes,a.created_at
FROM audit_log_legacy_027 a
LEFT JOIN users u ON u.id=a.actor_id;

DROP TABLE audit_log_legacy_027;
CREATE INDEX idx_audit_log_server ON audit_log(server_id, created_at DESC);
CREATE INDEX idx_audit_log_actor ON audit_log(actor_id, created_at DESC);
CREATE INDEX idx_audit_log_target ON audit_log(target_type, target_id);

-- SQLite considers NULL values distinct in an ordinary composite UNIQUE index,
-- so each scope shape needs its own partial uniqueness constraint.
DROP INDEX idx_notification_scope;
CREATE UNIQUE INDEX idx_notification_global_scope
    ON notification_settings(user_id)
    WHERE server_id IS NULL AND channel_id IS NULL;
CREATE UNIQUE INDEX idx_notification_server_scope
    ON notification_settings(user_id,server_id)
    WHERE server_id IS NOT NULL AND channel_id IS NULL;
CREATE UNIQUE INDEX idx_notification_channel_scope
    ON notification_settings(user_id,channel_id)
    WHERE server_id IS NOT NULL AND channel_id IS NOT NULL;

CREATE TRIGGER notification_scope_insert_guard
BEFORE INSERT ON notification_settings
WHEN (NEW.channel_id IS NOT NULL AND NEW.server_id IS NULL)
  OR (NEW.channel_id IS NOT NULL AND NOT EXISTS(
      SELECT 1 FROM channels c
      WHERE c.id=NEW.channel_id AND c.server_id=NEW.server_id
  ))
BEGIN
    SELECT RAISE(ABORT,'invalid notification scope');
END;

CREATE TRIGGER notification_scope_update_guard
BEFORE UPDATE OF server_id,channel_id ON notification_settings
WHEN (NEW.channel_id IS NOT NULL AND NEW.server_id IS NULL)
  OR (NEW.channel_id IS NOT NULL AND NOT EXISTS(
      SELECT 1 FROM channels c
      WHERE c.id=NEW.channel_id AND c.server_id=NEW.server_id
  ))
BEGIN
    SELECT RAISE(ABORT,'invalid notification scope');
END;

-- Thread authorship must be the actor who created the thread, rather than the
-- author of its parent message. Legacy rows intentionally remain unknown so
-- they cannot acquire guessed ownership authority.
ALTER TABLE channels ADD COLUMN thread_creator_user_id TEXT REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE channels ADD COLUMN thread_tags_version INTEGER NOT NULL DEFAULT 1;
CREATE INDEX idx_channels_thread_creator ON channels(thread_creator_user_id,channel_type);

-- A ban commits access revocation promptly while message cleanup advances in
-- bounded, restart-safe batches. Progress remains inspectable even after the
-- job completes.
CREATE TABLE moderation_cleanup_jobs (
    id              TEXT PRIMARY KEY,
    ban_id          TEXT NOT NULL UNIQUE,
    server_id       TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL,
    actor_id        TEXT NOT NULL,
    cutoff_at       TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending'
                    CHECK(state IN ('pending','completed')),
    deleted_count   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_moderation_cleanup_pending
    ON moderation_cleanup_jobs(state,created_at,id);
CREATE TABLE moderation_cleanup_scopes (
    job_id              TEXT NOT NULL REFERENCES moderation_cleanup_jobs(id) ON DELETE CASCADE,
    conversation_id     TEXT NOT NULL,
    through_sequence    INTEGER NOT NULL CHECK(through_sequence >= 0),
    PRIMARY KEY(job_id,conversation_id)
);

INSERT INTO migration_repair_log(
    migration_version,repair_kind,object_type,object_id,outcome,details
)
SELECT 27,'legacy_thread_creator','thread',id,'unresolved',
       'legacy schema did not persist the actor who created the thread; moderator authority is required for ownership-sensitive changes'
FROM channels
WHERE channel_type IN ('public_thread','private_thread');

INSERT OR IGNORE INTO schema_version(version) VALUES(27);
