-- Migration 019: persisted authorization versions and thread visibility boundaries.

ALTER TABLE servers ADD COLUMN authorization_version INTEGER NOT NULL DEFAULT 1
    CHECK (authorization_version > 0);
ALTER TABLE channels ADD COLUMN authorization_version INTEGER NOT NULL DEFAULT 1
    CHECK (authorization_version > 0);
ALTER TABLE channels ADD COLUMN parent_channel_id TEXT REFERENCES channels(id) ON DELETE CASCADE;
ALTER TABLE channels ADD COLUMN visibility_repair_required INTEGER NOT NULL DEFAULT 0
    CHECK (visibility_repair_required IN (0, 1));

CREATE TABLE channel_visibility_grants (
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    target_type TEXT NOT NULL CHECK (target_type IN ('user', 'role')),
    target_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (channel_id, target_type, target_id)
);
CREATE INDEX idx_visibility_grants_target
    ON channel_visibility_grants(target_type, target_id, channel_id);

CREATE TABLE thread_members (
    thread_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (thread_id, user_id)
);
CREATE INDEX idx_thread_members_user ON thread_members(user_id, thread_id);

INSERT OR IGNORE INTO channel_visibility_grants(channel_id, target_type, target_id)
SELECT channel_id, target_type, target_id
FROM channel_permission_overrides
WHERE (allow_bits & 1) != 0 AND (deny_bits & 1) = 0;

INSERT INTO migration_repair_log
    (migration_version, repair_kind, object_type, object_id, outcome, details)
SELECT 19, 'legacy_visibility_override', 'channel', channel_id, 'adopted',
       'explicit VIEW_CHANNELS allow adopted as persisted visibility grant'
FROM channel_permission_overrides
WHERE (allow_bits & 1) != 0 AND (deny_bits & 1) = 0;

UPDATE channels
SET parent_channel_id = (
    SELECT m.channel_id FROM messages m
    WHERE m.id = channels.thread_parent_message_id
      AND m.server_id = channels.server_id
)
WHERE channel_type IN ('public_thread', 'private_thread');

UPDATE channels SET is_private = 1 WHERE channel_type = 'private_thread';

INSERT OR IGNORE INTO thread_members(thread_id, user_id, joined_at)
SELECT c.id, cm.user_id, cm.joined_at
FROM channels c
JOIN channel_members cm ON cm.channel_id = c.id
WHERE c.channel_type = 'private_thread';

INSERT INTO migration_repair_log
    (migration_version, repair_kind, object_type, object_id, outcome, details)
SELECT 19, 'legacy_thread_membership', 'thread', c.id, 'adopted',
       'legacy persisted channel membership adopted as explicit private-thread membership'
FROM channels c
WHERE c.channel_type = 'private_thread'
  AND EXISTS (SELECT 1 FROM thread_members tm WHERE tm.thread_id = c.id);

UPDATE channels
SET visibility_repair_required = 1
WHERE channel_type IN ('public_thread', 'private_thread')
  AND parent_channel_id IS NULL;

UPDATE channels
SET visibility_repair_required = 1
WHERE channel_type = 'private_thread'
  AND NOT EXISTS (SELECT 1 FROM thread_members tm WHERE tm.thread_id = channels.id);

UPDATE channels
SET visibility_repair_required = 1
WHERE is_private = 1
  AND NOT EXISTS (
      SELECT 1 FROM channel_visibility_grants g WHERE g.channel_id = channels.id
  )
  AND channel_type != 'private_thread';

CREATE INDEX idx_channels_parent_channel ON channels(parent_channel_id, channel_type);
CREATE INDEX idx_channels_authorization_version ON channels(server_id, authorization_version);

CREATE TRIGGER authorization_server_members_insert AFTER INSERT ON server_members BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = new.server_id;
END;
CREATE TRIGGER authorization_server_members_update AFTER UPDATE ON server_members BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (old.server_id, new.server_id);
END;
CREATE TRIGGER authorization_server_members_delete AFTER DELETE ON server_members BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = old.server_id;
END;
CREATE TRIGGER authorization_roles_insert AFTER INSERT ON roles BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = new.server_id;
END;
CREATE TRIGGER authorization_roles_update AFTER UPDATE ON roles BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (old.server_id, new.server_id);
END;
CREATE TRIGGER authorization_roles_delete AFTER DELETE ON roles BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = old.server_id;
END;
CREATE TRIGGER authorization_user_roles_insert AFTER INSERT ON user_roles BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = new.server_id;
END;
CREATE TRIGGER authorization_user_roles_update AFTER UPDATE ON user_roles BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (old.server_id, new.server_id);
END;
CREATE TRIGGER authorization_user_roles_delete AFTER DELETE ON user_roles BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = old.server_id;
END;
CREATE TRIGGER authorization_bans_insert AFTER INSERT ON bans BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = new.server_id;
END;
CREATE TRIGGER authorization_bans_update AFTER UPDATE ON bans BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (old.server_id, new.server_id);
END;
CREATE TRIGGER authorization_bans_delete AFTER DELETE ON bans BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = old.server_id;
END;
CREATE TRIGGER authorization_overrides_insert AFTER INSERT ON channel_permission_overrides BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = new.channel_id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = (SELECT server_id FROM channels WHERE id = new.channel_id);
END;
CREATE TRIGGER authorization_overrides_update AFTER UPDATE ON channel_permission_overrides BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id IN (old.channel_id, new.channel_id);
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (SELECT server_id FROM channels WHERE id IN (old.channel_id, new.channel_id));
END;
CREATE TRIGGER authorization_overrides_delete AFTER DELETE ON channel_permission_overrides BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = old.channel_id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = (SELECT server_id FROM channels WHERE id = old.channel_id);
END;
CREATE TRIGGER authorization_visibility_insert AFTER INSERT ON channel_visibility_grants BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = new.channel_id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = (SELECT server_id FROM channels WHERE id = new.channel_id);
END;
CREATE TRIGGER authorization_visibility_update AFTER UPDATE ON channel_visibility_grants BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id IN (old.channel_id, new.channel_id);
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (SELECT server_id FROM channels WHERE id IN (old.channel_id, new.channel_id));
END;
CREATE TRIGGER authorization_visibility_delete AFTER DELETE ON channel_visibility_grants BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = old.channel_id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = (SELECT server_id FROM channels WHERE id = old.channel_id);
END;
CREATE TRIGGER authorization_thread_members_insert AFTER INSERT ON thread_members BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = new.thread_id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = (SELECT server_id FROM channels WHERE id = new.thread_id);
END;
CREATE TRIGGER authorization_thread_members_update AFTER UPDATE ON thread_members BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id IN (old.thread_id, new.thread_id);
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (SELECT server_id FROM channels WHERE id IN (old.thread_id, new.thread_id));
END;
CREATE TRIGGER authorization_thread_members_delete AFTER DELETE ON thread_members BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = old.thread_id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = (SELECT server_id FROM channels WHERE id = old.thread_id);
END;
CREATE TRIGGER authorization_channels_visibility_update
AFTER UPDATE OF is_private, channel_type, parent_channel_id, visibility_repair_required ON channels BEGIN
    UPDATE channels SET authorization_version = authorization_version + 1 WHERE id = new.id;
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id IN (old.server_id, new.server_id);
END;
CREATE TRIGGER authorization_server_owner_update
AFTER UPDATE OF owner_id ON servers BEGIN
    UPDATE servers SET authorization_version = authorization_version + 1 WHERE id = new.id;
END;
