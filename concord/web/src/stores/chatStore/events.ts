import type { ChatEvent as ServerEvent } from '../../api/generated/contract';
import type { ChatStoreContext } from './types';

import { handleBookmarksEvents } from './events/bookmarks';
import { handleCategoriesEvents } from './events/categories';
import { handleCommandsEvents } from './events/commands';
import { handleCommunityEvents } from './events/community';
import { handleForumTagsEvents } from './events/forumTags';
import { handleHistoryEvents } from './events/history';
import { handleIntegrationsEvents } from './events/integrations';
import { handleMembershipEvents } from './events/membership';
import { handleMessagesEvents } from './events/messages';
import { handleModerationEvents } from './events/moderation';
import { handlePinsEvents } from './events/pins';
import { handleProfilesEvents } from './events/profiles';
import { handleRolesEvents } from './events/roles';
import { handleSearchEvents } from './events/search';
import { handleServerProfileEvents } from './events/serverProfile';
import { handleServersEvents } from './events/servers';
import { handleSyncEvents } from './events/sync';
import { handleThreadsEvents } from './events/threads';

export function handleChatEvent(context: ChatStoreContext, event: ServerEvent) {
  switch (event.type) {
    case 'sync_snapshot':
    case 'replay_batch':
    case 'durable_event':
    case 'resync_required':
      return handleSyncEvents(context, event);
    case 'command_error':
    case 'command_committed':
      return handleCommandsEvents(context, event);
    case 'message':
    case 'message_edit':
    case 'message_delete':
    case 'message_ack':
    case 'message_embed':
    case 'reaction_add':
    case 'reaction_remove':
      return handleMessagesEvents(context, event);
    case 'typing_start':
    case 'join':
    case 'part':
    case 'quit':
    case 'names':
    case 'topic_change':
    case 'channel_list':
      return handleMembershipEvents(context, event);
    case 'history':
      return handleHistoryEvents(context, event);
    case 'server_list':
    case 'direct_conversation_list':
    case 'unread_counts':
      return handleServersEvents(context, event);
    case 'role_list':
    case 'role_update':
    case 'role_delete':
    case 'member_role_update':
    case 'channel_permission_override_list':
      return handleRolesEvents(context, event);
    case 'category_list':
    case 'category_update':
    case 'category_delete':
    case 'channel_reorder':
      return handleCategoriesEvents(context, event);
    case 'presence_update':
    case 'presence_list':
    case 'user_profile':
    case 'own_presence':
    case 'server_nickname_update':
      return handleProfilesEvents(context, event);
    case 'notification_settings':
    case 'search_results':
      return handleSearchEvents(context, event);
    case 'message_pin':
    case 'message_unpin':
    case 'pinned_messages':
      return handlePinsEvents(context, event);
    case 'thread_create':
    case 'thread_update':
    case 'thread_list':
      return handleThreadsEvents(context, event);
    case 'forum_tag_list':
    case 'forum_tag_update':
    case 'forum_tag_delete':
    case 'thread_tag_update':
      return handleForumTagsEvents(context, event);
    case 'bookmark_list':
    case 'bookmark_add':
    case 'bookmark_remove':
      return handleBookmarksEvents(context, event);
    case 'member_kick':
    case 'member_ban':
    case 'member_unban':
    case 'member_timeout':
    case 'slow_mode_update':
    case 'nsfw_update':
    case 'bulk_message_delete':
    case 'audit_log_entries':
    case 'ban_list':
    case 'automod_rule_list':
    case 'automod_rule_update':
    case 'automod_rule_delete':
      return handleModerationEvents(context, event);
    case 'invite_list':
    case 'invite_create':
    case 'invite_delete':
    case 'event_list':
    case 'event_update':
    case 'event_delete':
    case 'event_rsvp_list':
    case 'server_community':
    case 'discover_servers':
    case 'channel_follow_list':
    case 'channel_follow_create':
    case 'channel_follow_delete':
    case 'template_list':
    case 'template_update':
    case 'template_delete':
    case 'template_instantiated':
      return handleCommunityEvents(context, event);
    case 'webhook_list':
    case 'webhook_update':
    case 'webhook_delete':
    case 'slash_command_list':
    case 'slash_command_update':
    case 'slash_command_delete':
    case 'interaction_create':
    case 'interaction_response':
    case 'interaction_invoked':
    case 'lifecycle_command_succeeded':
    case 'bot_token_list':
    case 'bot_account_list':
    case 'bot_credential_created':
    case 'o_auth2_app_list':
    case 'o_auth2_app_update':
      return handleIntegrationsEvents(context, event);
    case 'server_avatar_update':
    case 'server_limits':
    case 'error':
      return handleServerProfileEvents(context, event);
  }
}
