-- Migration 014: Switch user IDs from UUID to DID
-- Since AT Protocol (Bluesky) is the only auth provider, every user has a DID
-- in oauth_accounts.provider_id. This migration replaces UUID user IDs with DIDs
-- and updates usernames to Bluesky handles.
-- pool.rs wraps migrations with PRAGMA foreign_keys = OFF/ON, so FK changes are safe.

-- Step 1: Build mapping table from old UUID -> new DID + handle
CREATE TEMPORARY TABLE _user_id_map AS
SELECT u.id AS old_id, oa.provider_id AS new_id, oa.bsky_handle AS handle
FROM users u
JOIN oauth_accounts oa ON oa.user_id = u.id AND oa.provider = 'atproto';

-- Step 2: Update all tables referencing users(id)

-- 001_initial.sql tables
UPDATE oauth_accounts SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = oauth_accounts.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE irc_tokens SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = irc_tokens.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE sessions SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = sessions.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE user_nicknames SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = user_nicknames.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 002_servers.sql tables
UPDATE servers SET owner_id = (SELECT new_id FROM _user_id_map WHERE old_id = servers.owner_id) WHERE owner_id IN (SELECT old_id FROM _user_id_map);
UPDATE server_members SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = server_members.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE channel_members SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = channel_members.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 003_messaging_enhancements.sql tables
UPDATE reactions SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = reactions.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE read_states SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = read_states.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 004_media_files.sql tables
UPDATE attachments SET uploader_id = (SELECT new_id FROM _user_id_map WHERE old_id = attachments.uploader_id) WHERE uploader_id IN (SELECT old_id FROM _user_id_map);
UPDATE custom_emoji SET uploader_id = (SELECT new_id FROM _user_id_map WHERE old_id = custom_emoji.uploader_id) WHERE uploader_id IN (SELECT old_id FROM _user_id_map);

-- 007_organization_permissions.sql tables
UPDATE user_roles SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = user_roles.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 008_user_experience.sql tables
UPDATE user_presence SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = user_presence.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE user_profiles SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = user_profiles.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE notification_settings SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = notification_settings.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 009_threads_pinning.sql tables
UPDATE pinned_messages SET pinned_by = (SELECT new_id FROM _user_id_map WHERE old_id = pinned_messages.pinned_by) WHERE pinned_by IN (SELECT old_id FROM _user_id_map);
UPDATE bookmarks SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = bookmarks.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 010_moderation.sql tables
UPDATE bans SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = bans.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE bans SET banned_by = (SELECT new_id FROM _user_id_map WHERE old_id = bans.banned_by) WHERE banned_by IN (SELECT old_id FROM _user_id_map);
UPDATE audit_log SET actor_id = (SELECT new_id FROM _user_id_map WHERE old_id = audit_log.actor_id) WHERE actor_id IN (SELECT old_id FROM _user_id_map);

-- 011_community.sql tables
UPDATE invites SET created_by = (SELECT new_id FROM _user_id_map WHERE old_id = invites.created_by) WHERE created_by IN (SELECT old_id FROM _user_id_map);
UPDATE server_events SET created_by = (SELECT new_id FROM _user_id_map WHERE old_id = server_events.created_by) WHERE created_by IN (SELECT old_id FROM _user_id_map);
UPDATE event_rsvps SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = event_rsvps.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE channel_follows SET created_by = (SELECT new_id FROM _user_id_map WHERE old_id = channel_follows.created_by) WHERE created_by IN (SELECT old_id FROM _user_id_map);
UPDATE server_templates SET created_by = (SELECT new_id FROM _user_id_map WHERE old_id = server_templates.created_by) WHERE created_by IN (SELECT old_id FROM _user_id_map);

-- 012_integrations.sql tables
UPDATE bot_tokens SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = bot_tokens.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE webhooks SET created_by = (SELECT new_id FROM _user_id_map WHERE old_id = webhooks.created_by) WHERE created_by IN (SELECT old_id FROM _user_id_map);
UPDATE slash_commands SET bot_user_id = (SELECT new_id FROM _user_id_map WHERE old_id = slash_commands.bot_user_id) WHERE bot_user_id IN (SELECT old_id FROM _user_id_map);
UPDATE interactions SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = interactions.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);
UPDATE oauth2_apps SET owner_id = (SELECT new_id FROM _user_id_map WHERE old_id = oauth2_apps.owner_id) WHERE owner_id IN (SELECT old_id FROM _user_id_map);
UPDATE oauth2_authorizations SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = oauth2_authorizations.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- 013_atproto_integration.sql tables
UPDATE bsky_shared_posts SET user_id = (SELECT new_id FROM _user_id_map WHERE old_id = bsky_shared_posts.user_id) WHERE user_id IN (SELECT old_id FROM _user_id_map);

-- messages table (sender_id and target_user_id are NOT FK-constrained, just TEXT)
UPDATE messages SET sender_id = (SELECT new_id FROM _user_id_map WHERE old_id = messages.sender_id) WHERE sender_id IN (SELECT old_id FROM _user_id_map);
UPDATE messages SET target_user_id = (SELECT new_id FROM _user_id_map WHERE old_id = messages.target_user_id) WHERE target_user_id IN (SELECT old_id FROM _user_id_map);

-- Step 3: Update the users table itself (PK: id -> DID, username -> handle)
UPDATE users SET
    id = (SELECT new_id FROM _user_id_map WHERE old_id = users.id),
    username = COALESCE(
        (SELECT handle FROM _user_id_map WHERE old_id = users.id),
        users.username
    )
WHERE id IN (SELECT old_id FROM _user_id_map);

-- Step 4: Update primary nicknames to match new handle
UPDATE user_nicknames SET nickname = (
    SELECT u.username FROM users u WHERE u.id = user_nicknames.user_id
) WHERE is_primary = 1 AND user_id IN (SELECT new_id FROM _user_id_map);

-- Cleanup
DROP TABLE IF EXISTS _user_id_map;
