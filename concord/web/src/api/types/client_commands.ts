import type { ChannelPositionInfo } from './permissions';

// Client → Server commands
export type ClientCommand =
  | { type: 'send_message'; server_id: string; channel: string; content: string; reply_to?: string; attachment_ids?: string[]; nonce?: string }
  | { type: 'edit_message'; message_id: string; content: string }
  | { type: 'delete_message'; message_id: string }
  | { type: 'add_reaction'; message_id: string; emoji: string }
  | { type: 'remove_reaction'; message_id: string; emoji: string }
  | { type: 'typing'; server_id: string; channel: string }
  | { type: 'join_channel'; server_id: string; channel: string }
  | { type: 'part_channel'; server_id: string; channel: string; reason?: string }
  | { type: 'set_topic'; server_id: string; channel: string; topic: string }
  | { type: 'fetch_history'; server_id: string; channel: string; before?: string; limit?: number }
  | { type: 'list_channels'; server_id: string }
  | { type: 'get_members'; server_id: string; channel: string }
  | { type: 'list_servers' }
  | { type: 'create_server'; name: string; icon_url?: string }
  | { type: 'join_server'; server_id: string }
  | { type: 'leave_server'; server_id: string }
  | { type: 'create_channel'; server_id: string; name: string; category_id?: string; is_private?: boolean }
  | { type: 'delete_channel'; server_id: string; channel: string }
  | { type: 'delete_server'; server_id: string }
  | { type: 'update_member_role'; server_id: string; user_id: string; role: string }
  | { type: 'mark_read'; server_id: string; channel: string; message_id: string }
  | { type: 'get_unread_counts'; server_id: string }
  | { type: 'list_roles'; server_id: string }
  | { type: 'create_role'; server_id: string; name: string; color?: string; permissions?: number; position?: number }
  | { type: 'update_role'; server_id: string; role_id: string; name?: string; color?: string; permissions?: number; position?: number }
  | { type: 'delete_role'; server_id: string; role_id: string }
  | { type: 'assign_role'; server_id: string; user_id: string; role_id: string }
  | { type: 'remove_role'; server_id: string; user_id: string; role_id: string }
  | { type: 'list_categories'; server_id: string }
  | { type: 'create_category'; server_id: string; name: string }
  | { type: 'update_category'; server_id: string; category_id: string; name?: string; position?: number }
  | { type: 'delete_category'; server_id: string; category_id: string }
  | { type: 'reorder_channels'; server_id: string; channels: ChannelPositionInfo[] }
  | { type: 'set_presence'; status: string; custom_status?: string; status_emoji?: string }
  | { type: 'get_presences'; server_id: string }
  | { type: 'set_server_nickname'; server_id: string; nickname?: string }
  | { type: 'search_messages'; server_id: string; query: string; channel?: string; limit?: number; offset?: number }
  | { type: 'update_notification_settings'; server_id: string; channel_id?: string; level: string; suppress_everyone?: boolean; suppress_roles?: boolean; muted?: boolean; mute_until?: string }
  | { type: 'get_notification_settings'; server_id: string }
  | { type: 'get_user_profile'; user_id: string }
  | { type: 'pin_message'; server_id: string; channel: string; message_id: string }
  | { type: 'unpin_message'; server_id: string; channel: string; message_id: string }
  | { type: 'get_pinned_messages'; server_id: string; channel: string }
  | { type: 'create_thread'; server_id: string; parent_channel: string; name: string; message_id: string; is_private?: boolean }
  | { type: 'archive_thread'; server_id: string; thread_id: string }
  | { type: 'list_threads'; server_id: string; channel: string }
  | { type: 'add_bookmark'; message_id: string; note?: string }
  | { type: 'remove_bookmark'; message_id: string }
  | { type: 'list_bookmarks' }
  | { type: 'kick_member'; server_id: string; user_id: string; reason?: string }
  | { type: 'ban_member'; server_id: string; user_id: string; reason?: string; delete_message_days?: number }
  | { type: 'unban_member'; server_id: string; user_id: string }
  | { type: 'list_bans'; server_id: string }
  | { type: 'timeout_member'; server_id: string; user_id: string; timeout_until?: string; reason?: string }
  | { type: 'set_slow_mode'; server_id: string; channel: string; seconds: number }
  | { type: 'set_nsfw'; server_id: string; channel: string; is_nsfw: boolean }
  | { type: 'bulk_delete_messages'; server_id: string; channel: string; message_ids: string[] }
  | { type: 'get_audit_log'; server_id: string; action_type?: string; limit?: number; before?: string }
  | { type: 'create_automod_rule'; server_id: string; name: string; rule_type: string; config: string; action_type: string; timeout_duration_seconds?: number }
  | { type: 'update_automod_rule'; server_id: string; rule_id: string; name: string; enabled: boolean; config: string; action_type: string; timeout_duration_seconds?: number }
  | { type: 'delete_automod_rule'; server_id: string; rule_id: string }
  | { type: 'list_automod_rules'; server_id: string }
  | { type: 'create_invite'; server_id: string; max_uses?: number; expires_at?: string; channel_id?: string }
  | { type: 'list_invites'; server_id: string }
  | { type: 'delete_invite'; server_id: string; invite_id: string }
  | { type: 'use_invite'; code: string }
  | { type: 'create_event'; server_id: string; name: string; description?: string; channel_id?: string; start_time: string; end_time?: string; image_url?: string }
  | { type: 'list_events'; server_id: string }
  | { type: 'update_event_status'; server_id: string; event_id: string; status: string }
  | { type: 'delete_event'; server_id: string; event_id: string }
  | { type: 'set_rsvp'; server_id: string; event_id: string; status: string }
  | { type: 'remove_rsvp'; server_id: string; event_id: string }
  | { type: 'list_rsvps'; event_id: string }
  | { type: 'update_community_settings'; server_id: string; description?: string; is_discoverable: boolean; welcome_message?: string; rules_text?: string; category?: string }
  | { type: 'get_community_settings'; server_id: string }
  | { type: 'discover_servers'; category?: string }
  | { type: 'accept_rules'; server_id: string }
  | { type: 'set_announcement_channel'; server_id: string; channel: string; is_announcement: boolean }
  | { type: 'follow_channel'; source_channel_id: string; target_channel_id: string }
  | { type: 'unfollow_channel'; follow_id: string }
  | { type: 'list_channel_follows'; channel_id: string }
  | { type: 'create_template'; server_id: string; name: string; description?: string }
  | { type: 'list_templates'; server_id: string }
  | { type: 'delete_template'; server_id: string; template_id: string }
  | { type: 'instantiate_template'; template_id: string; server_name: string }
  // Phase 8: Integrations & Bots
  | { type: 'create_webhook'; server_id: string; channel_id: string; name: string; webhook_type: string; url?: string }
  | { type: 'list_webhooks'; server_id: string }
  | { type: 'update_webhook'; webhook_id: string; name: string; avatar_url?: string }
  | { type: 'delete_webhook'; webhook_id: string }
  | { type: 'create_bot'; username: string }
  | { type: 'list_owned_bots' }
  | { type: 'create_bot_token'; bot_user_id: string; name?: string; scopes?: string }
  | { type: 'list_bot_tokens'; bot_user_id: string }
  | { type: 'delete_bot_token'; token_id: string }
  | { type: 'add_bot_to_server'; bot_user_id: string; server_id: string }
  | { type: 'remove_bot_from_server'; bot_user_id: string; server_id: string }
  | { type: 'register_slash_command'; server_id: string; name: string; description: string; options_json?: string }
  | { type: 'list_slash_commands'; server_id: string }
  | { type: 'delete_slash_command'; command_id: string }
  | { type: 'invoke_slash_command'; server_id: string; channel_id: string; command_name: string; args_json?: string }
  | { type: 'invoke_message_component'; message_id: string; custom_id: string; values?: string[] }
  | { type: 'respond_to_interaction'; interaction_id: string; content?: string; embeds_json?: string; components_json?: string }
  | { type: 'create_oauth2_app'; name: string; description: string; redirect_uris: string; client_type: 'confidential' | 'public'; scopes?: string }
  | { type: 'list_oauth2_apps' }
  | { type: 'delete_oauth2_app'; app_id: string }
  | { type: 'update_server'; server_id: string; name?: string; icon_url?: string }
  | { type: 'set_server_avatar'; server_id: string; avatar_url?: string | null }
  | { type: 'set_vanity_code'; server_id: string; vanity_code?: string | null }
  | { type: 'get_server_limits' };
