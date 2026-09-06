// Generated from the production Rust Serde DTOs. Do not edit.

/**
 * Client-to-server WebSocket message types.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ClientMessage".
 */
export type ClientMessage =
  | {
      command: ClientMessage;
      request_id: string;
      type: "lifecycle_command";
    }
  | {
      cursor?: string | null;
      limit?: number | null;
      protocol_version: number;
      request_id: string;
      subscriptions: string[];
      type: "sync";
    }
  | {
      attachment_ids?: string[] | null;
      channel: string;
      client_message_id?: string | null;
      content: string;
      content_format?: "plain" | "markdown";
      conversation_id?: string | null;
      mentions?: MessageMention[];
      nonce?: string | null;
      operation_generation: string;
      reply_to?: string | null;
      request_id?: string | null;
      server_id?: string;
      type: "send_message";
    }
  | {
      attachment_ids?: string[] | null;
      client_message_id?: string | null;
      content: string;
      content_format?: "plain" | "markdown";
      nonce?: string | null;
      operation_generation: string;
      recipient: string;
      reply_to?: string | null;
      request_id?: string | null;
      type: "send_direct_message";
    }
  | {
      type: "list_direct_conversations";
    }
  | {
      channel: string;
      server_id?: string;
      type: "join_channel";
    }
  | {
      channel: string;
      reason?: string | null;
      server_id?: string;
      type: "part_channel";
    }
  | {
      channel: string;
      server_id?: string;
      topic: string;
      type: "set_topic";
    }
  | {
      before?: string | null;
      channel: string;
      limit?: number | null;
      server_id?: string;
      type: "fetch_history";
    }
  | {
      server_id?: string;
      type: "list_channels";
    }
  | {
      channel: string;
      server_id?: string;
      type: "get_members";
    }
  | {
      type: "list_servers";
    }
  | {
      icon_url?: string | null;
      name: string;
      type: "create_server";
    }
  | {
      server_id: string;
      type: "join_server";
    }
  | {
      server_id: string;
      type: "leave_server";
    }
  | {
      category_id?: string | null;
      channel_type?: string | null;
      is_private?: boolean | null;
      name: string;
      server_id: string;
      type: "create_channel";
    }
  | {
      channel: string;
      server_id: string;
      type: "delete_channel";
    }
  | {
      server_id: string;
      type: "delete_server";
    }
  | {
      icon_url?: string | null;
      name?: string | null;
      server_id: string;
      type: "update_server";
    }
  | {
      role: string;
      server_id: string;
      type: "update_member_role";
      user_id: string;
    }
  | {
      client_message_id?: string | null;
      content: string;
      content_format?: "plain" | "markdown";
      mentions?: MessageMention[];
      message_id: string;
      operation_generation: string;
      request_id?: string | null;
      type: "edit_message";
    }
  | {
      client_message_id?: string | null;
      message_id: string;
      operation_generation: string;
      request_id?: string | null;
      type: "delete_message";
    }
  | {
      client_message_id?: string | null;
      emoji: string;
      message_id: string;
      operation_generation: string;
      request_id?: string | null;
      type: "add_reaction";
    }
  | {
      client_message_id?: string | null;
      emoji: string;
      message_id: string;
      operation_generation: string;
      request_id?: string | null;
      type: "remove_reaction";
    }
  | {
      channel: string;
      server_id?: string;
      type: "typing";
    }
  | {
      channel: string;
      client_message_id?: string | null;
      conversation_id?: string | null;
      message_id: string;
      operation_generation: string;
      request_id?: string | null;
      server_id?: string;
      type: "mark_read";
    }
  | {
      server_id?: string;
      type: "get_unread_counts";
    }
  | {
      server_id: string;
      type: "list_roles";
    }
  | {
      color?: string | null;
      name: string;
      permissions?: number | null;
      server_id: string;
      type: "create_role";
    }
  | {
      color?: string | null;
      name: string;
      permissions: number;
      role_id: string;
      server_id: string;
      type: "update_role";
    }
  | {
      role_id: string;
      server_id: string;
      type: "delete_role";
    }
  | {
      role_id: string;
      server_id: string;
      type: "assign_role";
      user_id: string;
    }
  | {
      role_id: string;
      server_id: string;
      type: "remove_role";
      user_id: string;
    }
  | {
      channel_id: string;
      server_id: string;
      type: "list_channel_permission_overrides";
    }
  | {
      allow_bits: number;
      channel_id: string;
      deny_bits: number;
      server_id: string;
      target_id: string;
      target_type: string;
      type: "set_channel_permission_override";
    }
  | {
      channel_id: string;
      server_id: string;
      target_id: string;
      target_type: string;
      type: "delete_channel_permission_override";
    }
  | {
      server_id: string;
      type: "list_categories";
    }
  | {
      name: string;
      server_id: string;
      type: "create_category";
    }
  | {
      category_id: string;
      name: string;
      server_id: string;
      type: "update_category";
    }
  | {
      category_id: string;
      server_id: string;
      type: "delete_category";
    }
  | {
      channels: ChannelPositionInfo[];
      server_id: string;
      type: "reorder_channels";
    }
  | {
      custom_status?: string | null;
      status: string;
      status_emoji?: string | null;
      type: "set_presence";
    }
  | {
      server_id: string;
      type: "get_presences";
    }
  | {
      nickname?: string | null;
      server_id: string;
      type: "set_server_nickname";
    }
  | {
      channel?: string | null;
      continuation?: string | null;
      limit?: number | null;
      offset?: number | null;
      query: string;
      request_id?: string | null;
      server_id: string;
      type: "search_messages";
    }
  | {
      channel_id?: string | null;
      level: string;
      mute_until?: string | null;
      muted?: boolean | null;
      server_id: string;
      suppress_everyone?: boolean | null;
      suppress_roles?: boolean | null;
      type: "update_notification_settings";
    }
  | {
      server_id: string;
      type: "get_notification_settings";
    }
  | {
      type: "get_user_profile";
      user_id: string;
    }
  | {
      channel: string;
      message_id: string;
      server_id: string;
      type: "pin_message";
    }
  | {
      channel: string;
      message_id: string;
      server_id: string;
      type: "unpin_message";
    }
  | {
      channel: string;
      server_id: string;
      type: "get_pinned_messages";
    }
  | {
      is_private?: boolean;
      message_id: string;
      name: string;
      parent_channel: string;
      server_id: string;
      type: "create_thread";
    }
  | {
      server_id: string;
      thread_id: string;
      type: "archive_thread";
    }
  | {
      server_id: string;
      thread_id: string;
      type: "unarchive_thread";
    }
  | {
      channel: string;
      server_id: string;
      type: "list_threads";
    }
  | {
      channel: string;
      emoji?: string | null;
      moderated?: boolean;
      name: string;
      server_id: string;
      type: "create_forum_tag";
    }
  | {
      channel: string;
      emoji?: string | null;
      moderated: boolean;
      name: string;
      position: number;
      server_id: string;
      tag_id: string;
      type: "update_forum_tag";
    }
  | {
      channel: string;
      server_id: string;
      tag_id: string;
      type: "delete_forum_tag";
    }
  | {
      channel: string;
      server_id: string;
      type: "list_forum_tags";
    }
  | {
      server_id: string;
      tag_ids: string[];
      thread_id: string;
      type: "set_thread_tags";
    }
  | {
      server_id: string;
      thread_id: string;
      type: "get_thread_tags";
    }
  | {
      message_id: string;
      note?: string | null;
      type: "add_bookmark";
    }
  | {
      message_id: string;
      type: "remove_bookmark";
    }
  | {
      type: "list_bookmarks";
    }
  | {
      reason?: string | null;
      server_id: string;
      type: "kick_member";
      user_id: string;
    }
  | {
      delete_message_days?: number;
      reason?: string | null;
      server_id: string;
      type: "ban_member";
      user_id: string;
    }
  | {
      server_id: string;
      type: "unban_member";
      user_id: string;
    }
  | {
      server_id: string;
      type: "list_bans";
    }
  | {
      reason?: string | null;
      server_id: string;
      timeout_until?: string | null;
      type: "timeout_member";
      user_id: string;
    }
  | {
      channel: string;
      seconds: number;
      server_id: string;
      type: "set_slow_mode";
    }
  | {
      channel: string;
      is_nsfw: boolean;
      server_id: string;
      type: "set_nsfw";
    }
  | {
      channel: string;
      message_ids: string[];
      server_id: string;
      type: "bulk_delete_messages";
    }
  | {
      action_type?: string | null;
      before?: string | null;
      limit?: number | null;
      server_id: string;
      type: "get_audit_log";
    }
  | {
      action_type: string;
      config: string;
      name: string;
      rule_type: string;
      server_id: string;
      timeout_duration_seconds?: number | null;
      type: "create_automod_rule";
    }
  | {
      action_type: string;
      config: string;
      enabled: boolean;
      name: string;
      rule_id: string;
      server_id: string;
      timeout_duration_seconds?: number | null;
      type: "update_automod_rule";
    }
  | {
      rule_id: string;
      server_id: string;
      type: "delete_automod_rule";
    }
  | {
      server_id: string;
      type: "list_automod_rules";
    }
  | {
      channel_id?: string | null;
      expires_at?: string | null;
      max_uses?: number | null;
      server_id: string;
      type: "create_invite";
    }
  | {
      server_id: string;
      type: "list_invites";
    }
  | {
      invite_id: string;
      server_id: string;
      type: "delete_invite";
    }
  | {
      code: string;
      type: "use_invite";
    }
  | {
      channel_id?: string | null;
      description?: string | null;
      end_time?: string | null;
      image_url?: string | null;
      name: string;
      server_id: string;
      start_time: string;
      type: "create_event";
    }
  | {
      server_id: string;
      type: "list_events";
    }
  | {
      event_id: string;
      server_id: string;
      status: string;
      type: "update_event_status";
    }
  | {
      event_id: string;
      server_id: string;
      type: "delete_event";
    }
  | {
      event_id: string;
      server_id: string;
      status: string;
      type: "set_rsvp";
    }
  | {
      event_id: string;
      server_id: string;
      type: "remove_rsvp";
    }
  | {
      event_id: string;
      type: "list_rsvps";
    }
  | {
      category?: string | null;
      description?: string | null;
      is_discoverable: boolean;
      rules_text?: string | null;
      server_id: string;
      type: "update_community_settings";
      welcome_message?: string | null;
    }
  | {
      server_id: string;
      type: "get_community_settings";
    }
  | {
      category?: string | null;
      type: "discover_servers";
    }
  | {
      server_id: string;
      type: "accept_rules";
    }
  | {
      channel: string;
      is_announcement: boolean;
      server_id: string;
      type: "set_announcement_channel";
    }
  | {
      source_channel_id: string;
      target_channel_id: string;
      type: "follow_channel";
    }
  | {
      follow_id: string;
      type: "unfollow_channel";
    }
  | {
      channel_id: string;
      type: "list_channel_follows";
    }
  | {
      message_id: string;
      type: "publish_announcement";
    }
  | {
      description?: string | null;
      name: string;
      server_id: string;
      type: "create_template";
    }
  | {
      server_id: string;
      type: "list_templates";
    }
  | {
      server_id: string;
      template_id: string;
      type: "delete_template";
    }
  | {
      server_name: string;
      template_id: string;
      type: "instantiate_template";
    }
  | {
      channel_id: string;
      name: string;
      server_id: string;
      type: "create_webhook";
      url?: string | null;
      webhook_type: string;
    }
  | {
      server_id: string;
      type: "list_webhooks";
    }
  | {
      avatar_url?: string | null;
      channel_id: string;
      name: string;
      type: "update_webhook";
      webhook_id: string;
    }
  | {
      type: "delete_webhook";
      webhook_id: string;
    }
  | {
      avatar_url?: string | null;
      type: "create_bot";
      username: string;
    }
  | {
      type: "list_owned_bots";
    }
  | {
      bot_user_id: string;
      name: string;
      scopes?: string | null;
      type: "create_bot_token";
    }
  | {
      bot_user_id: string;
      type: "list_bot_tokens";
    }
  | {
      token_id: string;
      type: "delete_bot_token";
    }
  | {
      bot_user_id: string;
      server_id: string;
      type: "add_bot_to_server";
    }
  | {
      bot_user_id: string;
      server_id: string;
      type: "remove_bot_from_server";
    }
  | {
      description: string;
      name: string;
      options_json?: string | null;
      server_id: string;
      type: "register_slash_command";
    }
  | {
      server_id: string;
      type: "list_slash_commands";
    }
  | {
      command_id: string;
      type: "delete_slash_command";
    }
  | {
      args_json?: string | null;
      channel: string;
      command_name: string;
      request_id: string;
      server_id: string;
      type: "invoke_slash_command";
    }
  | {
      custom_id: string;
      message_id: string;
      request_id: string;
      type: "invoke_message_component";
      values?: string[];
    }
  | {
      components_json?: string | null;
      content?: string | null;
      embeds_json?: string | null;
      ephemeral?: boolean | null;
      interaction_id: string;
      type: "respond_to_interaction";
    }
  | {
      client_type?: string;
      description?: string | null;
      name: string;
      redirect_uris: string[];
      type: "create_o_auth2_app";
    }
  | {
      type: "list_o_auth2_apps";
    }
  | {
      app_id: string;
      type: "delete_o_auth2_app";
    }
  | {
      avatar_url?: string | null;
      server_id: string;
      type: "set_server_avatar";
    }
  | {
      server_id: string;
      type: "set_vanity_code";
      vanity_code?: string | null;
    }
  | {
      type: "get_server_limits";
    };
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "MentionKind".
 */
export type MentionKind = "user" | "role" | "everyone";
/**
 * Protocol-agnostic event that flows through the chat engine.
 * Both IRC and WebSocket adapters produce and consume these.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ChatEvent".
 */
export type ChatEvent =
  | {
      request_id: string;
      snapshot: SyncSnapshot;
      type: "sync_snapshot";
    }
  | {
      batch: ReplayBatch;
      request_id: string;
      type: "replay_batch";
    }
  | {
      event: DurableEventProjection;
      type: "durable_event";
    }
  | {
      reason: ResyncReason;
      request_id: string;
      type: "resync_required";
    }
  | {
      code: string;
      message: string;
      request_id: string;
      retryable: boolean;
      type: "command_error";
    }
  | {
      request_id: string;
      type: "lifecycle_command_succeeded";
    }
  | {
      receipt: CommandReceipt;
      type: "command_committed";
    }
  | {
      attachments?: AttachmentInfo[] | null;
      avatar_url?: string | null;
      content: string;
      /**
       * Canonical conversation identifier, present for direct messages.
       */
      conversation_id?: string | null;
      from: string;
      id: MessageId;
      reply_to?: ReplyInfo | null;
      server_id?: string | null;
      target: string;
      timestamp: string;
      type: "message";
    }
  | {
      channel: string;
      content: string;
      edited_at: string;
      id: MessageId;
      server_id: string;
      type: "message_edit";
    }
  | {
      channel: string;
      id: MessageId;
      server_id: string;
      type: "message_delete";
    }
  | {
      channel: string;
      client_message_id: string;
      conversation_id?: string | null;
      id: MessageId;
      nonce?: string | null;
      persisted_at: string;
      replayed: boolean;
      request_id: string;
      /**
       * Decimal string to preserve the full SQLite integer range in JavaScript.
       */
      sequence: string;
      server_id: string;
      type: "message_ack";
    }
  | {
      channel: string;
      emoji: string;
      message_id: MessageId;
      nickname: string;
      server_id: string;
      type: "reaction_add";
      user_id: string;
    }
  | {
      channel: string;
      emoji: string;
      message_id: MessageId;
      nickname: string;
      server_id: string;
      type: "reaction_remove";
      user_id: string;
    }
  | {
      channel: string;
      nickname: string;
      server_id: string;
      type: "typing_start";
    }
  | {
      avatar_url?: string | null;
      channel: string;
      nickname: string;
      role_ids?: string[];
      server_avatar_url?: string | null;
      server_id: string;
      type: "join";
      user_id?: string | null;
    }
  | {
      channel: string;
      nickname: string;
      reason?: string | null;
      server_id: string;
      type: "part";
    }
  | {
      nickname: string;
      reason?: string | null;
      type: "quit";
    }
  | {
      channel: string;
      server_id: string;
      set_by: string;
      topic: string;
      type: "topic_change";
    }
  | {
      new_nick: string;
      old_nick: string;
      type: "nick_change";
    }
  | {
      message: string;
      type: "server_notice";
    }
  | {
      channel: string;
      members: MemberInfo[];
      server_id: string;
      type: "names";
    }
  | {
      channel: string;
      server_id: string;
      topic: string;
      type: "topic";
    }
  | {
      channels: ChannelInfo[];
      server_id: string;
      type: "channel_list";
    }
  | {
      channel: string;
      has_more: boolean;
      messages: HistoryMessage[];
      server_id: string;
      type: "history";
    }
  | {
      servers: ServerInfo[];
      type: "server_list";
    }
  | {
      counts: UnreadCount[];
      server_id: string;
      type: "unread_counts";
    }
  | {
      channel: string;
      embeds: EmbedInfo[];
      message_id: MessageId;
      server_id: string;
      type: "message_embed";
    }
  | {
      /**
       * Present for an authoritative bootstrap; absent for a metadata-only mutation.
       */
      member_roles?: MemberRoleInfo[] | null;
      roles: RoleInfo[];
      server_id: string;
      type: "role_list";
      version: number;
    }
  | {
      role: RoleInfo;
      server_id: string;
      type: "role_update";
    }
  | {
      role_id: string;
      server_id: string;
      type: "role_delete";
    }
  | {
      role_ids: string[];
      server_id: string;
      type: "member_role_update";
      user_id: string;
      version: number;
    }
  | {
      channel_id: string;
      overrides: ChannelPermissionOverrideInfo[];
      server_id: string;
      type: "channel_permission_override_list";
    }
  | {
      categories: CategoryInfo[];
      server_id: string;
      type: "category_list";
    }
  | {
      category: CategoryInfo;
      server_id: string;
      type: "category_update";
    }
  | {
      category_id: string;
      server_id: string;
      type: "category_delete";
    }
  | {
      channels: ChannelPositionInfo[];
      server_id: string;
      type: "channel_reorder";
    }
  | {
      presence: PresenceInfo;
      server_id: string;
      type: "presence_update";
    }
  | {
      presences: PresenceInfo[];
      server_id: string;
      type: "presence_list";
    }
  | {
      custom_status?: string | null;
      effective_status: string;
      requested_status: string;
      status_emoji?: string | null;
      type: "own_presence";
    }
  | {
      profile: UserProfileInfo;
      type: "user_profile";
    }
  | {
      display_name: string;
      nickname?: string | null;
      server_avatar_url?: string | null;
      server_id: string;
      type: "server_nickname_update";
      user_id: string;
    }
  | {
      server_id: string;
      settings: NotificationSettingInfo[];
      type: "notification_settings";
    }
  | {
      next_continuation?: string | null;
      offset: number;
      query: string;
      request_id?: string | null;
      restarted?: boolean;
      results: SearchResultMessage[];
      server_id: string;
      total_count: number;
      type: "search_results";
    }
  | {
      channel: string;
      pin: PinnedMessageInfo;
      server_id: string;
      type: "message_pin";
    }
  | {
      channel: string;
      message_id: string;
      server_id: string;
      type: "message_unpin";
    }
  | {
      channel: string;
      pins: PinnedMessageInfo[];
      server_id: string;
      type: "pinned_messages";
    }
  | {
      parent_channel: string;
      server_id: string;
      thread: ThreadInfo;
      type: "thread_create";
    }
  | {
      server_id: string;
      thread: ThreadInfo;
      type: "thread_update";
    }
  | {
      channel: string;
      server_id: string;
      threads: ThreadInfo[];
      type: "thread_list";
    }
  | {
      channel: string;
      server_id: string;
      tags: ForumTagInfo[];
      type: "forum_tag_list";
    }
  | {
      channel: string;
      server_id: string;
      tag: ForumTagInfo;
      type: "forum_tag_update";
    }
  | {
      channel: string;
      server_id: string;
      tag_id: string;
      type: "forum_tag_delete";
    }
  | {
      server_id: string;
      tag_ids: string[];
      thread_id: string;
      type: "thread_tag_update";
      version: number;
    }
  | {
      bookmarks: BookmarkInfo[];
      type: "bookmark_list";
    }
  | {
      bookmark: BookmarkInfo;
      type: "bookmark_add";
    }
  | {
      message_id: string;
      type: "bookmark_remove";
    }
  | {
      conversations: DirectConversationInfo[];
      type: "direct_conversation_list";
    }
  | {
      kicked_by: string;
      reason?: string | null;
      server_id: string;
      type: "member_kick";
      user_id: string;
    }
  | {
      banned_by: string;
      reason?: string | null;
      server_id: string;
      type: "member_ban";
      user_id: string;
    }
  | {
      server_id: string;
      type: "member_unban";
      user_id: string;
    }
  | {
      server_id: string;
      timeout_until?: string | null;
      type: "member_timeout";
      user_id: string;
    }
  | {
      channel: string;
      seconds: number;
      server_id: string;
      type: "slow_mode_update";
    }
  | {
      channel: string;
      is_nsfw: boolean;
      server_id: string;
      type: "nsfw_update";
    }
  | {
      channel: string;
      message_ids: string[];
      server_id: string;
      type: "bulk_message_delete";
    }
  | {
      entries: AuditLogEntry[];
      server_id: string;
      type: "audit_log_entries";
    }
  | {
      bans: BanInfo[];
      server_id: string;
      type: "ban_list";
    }
  | {
      rules: AutomodRuleInfo[];
      server_id: string;
      type: "automod_rule_list";
    }
  | {
      rule: AutomodRuleInfo;
      server_id: string;
      type: "automod_rule_update";
    }
  | {
      rule_id: string;
      server_id: string;
      type: "automod_rule_delete";
    }
  | {
      invites: InviteInfo[];
      server_id: string;
      type: "invite_list";
    }
  | {
      invite: InviteInfo;
      server_id: string;
      type: "invite_create";
    }
  | {
      invite_id: string;
      server_id: string;
      type: "invite_delete";
    }
  | {
      events: EventInfo[];
      server_id: string;
      type: "event_list";
    }
  | {
      event: EventInfo;
      server_id: string;
      type: "event_update";
    }
  | {
      event_id: string;
      server_id: string;
      type: "event_delete";
    }
  | {
      event_id: string;
      rsvps: RsvpInfo[];
      type: "event_rsvp_list";
    }
  | {
      community: ServerCommunityInfo;
      type: "server_community";
    }
  | {
      servers: ServerCommunityInfo[];
      type: "discover_servers";
    }
  | {
      channel_id: string;
      follows: ChannelFollowInfo[];
      type: "channel_follow_list";
    }
  | {
      follow: ChannelFollowInfo;
      type: "channel_follow_create";
    }
  | {
      follow_id: string;
      type: "channel_follow_delete";
    }
  | {
      published_count: number;
      source_message_id: string;
      type: "announcement_published";
    }
  | {
      server_id: string;
      templates: TemplateInfo[];
      type: "template_list";
    }
  | {
      server_id: string;
      template: TemplateInfo;
      type: "template_update";
    }
  | {
      server_id: string;
      template_id: string;
      type: "template_delete";
    }
  | {
      server_id: string;
      template_id: string;
      type: "template_instantiated";
    }
  | {
      server_id: string;
      type: "webhook_list";
      webhooks: WebhookInfo[];
    }
  | {
      server_id: string;
      type: "webhook_update";
      webhook: WebhookInfo;
    }
  | {
      server_id: string;
      type: "webhook_delete";
      webhook_id: string;
    }
  | {
      commands: SlashCommandInfo[];
      server_id: string;
      type: "slash_command_list";
    }
  | {
      command: SlashCommandInfo;
      server_id: string;
      type: "slash_command_update";
    }
  | {
      command_id: string;
      server_id: string;
      type: "slash_command_delete";
    }
  | {
      interaction: InteractionInfo;
      type: "interaction_create";
    }
  | {
      channel: string;
      interaction_id: string;
      response: InteractionResponseData;
      server_id: string;
      type: "interaction_response";
    }
  | {
      request_id: string;
      type: "interaction_invoked";
    }
  | {
      bot_user_id: string;
      tokens: BotTokenInfo[];
      type: "bot_token_list";
    }
  | {
      bots: BotAccountInfo[];
      type: "bot_account_list";
    }
  | {
      bot_user_id: string;
      credential: BotTokenInfo;
      token: string;
      type: "bot_credential_created";
    }
  | {
      apps: OAuth2AppInfo[];
      type: "o_auth2_app_list";
    }
  | {
      app: OAuth2AppInfo;
      type: "o_auth2_app_update";
    }
  | {
      avatar_url?: string | null;
      banner_url?: string | null;
      bsky_handle: string;
      description?: string | null;
      display_name?: string | null;
      followers_count: number;
      follows_count: number;
      type: "bluesky_profile_sync";
      user_id: string;
    }
  | {
      error?: string | null;
      message_id: string;
      post_uri?: string | null;
      success: boolean;
      type: "bluesky_share_result";
    }
  | {
      avatar_url?: string | null;
      server_id: string;
      type: "server_avatar_update";
      user_id: string;
    }
  | {
      max_file_size_mb: number;
      max_message_length: number;
      type: "server_limits";
    }
  | {
      code: string;
      message: string;
      type: "error";
    };
/**
 * A message component (button, select menu, or action row).
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "MessageComponent".
 */
export type MessageComponent =
  | {
      components: MessageComponent[];
      type: "action_row";
    }
  | {
      custom_id: string;
      disabled?: boolean;
      emoji?: string | null;
      label: string;
      style?: string;
      type: "button";
    }
  | {
      custom_id: string;
      max_values?: number;
      min_values?: number;
      options: SelectOption[];
      placeholder?: string | null;
      type: "select_menu";
    };
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ConversationId".
 */
export type ConversationId = string;
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ResyncReason".
 */
export type ResyncReason =
  | "cursor_expired"
  | "database_restored"
  | "credential_changed"
  | "subscription_changed"
  | "access_revoked"
  | "protocol_changed"
  | "invalid_cursor";
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "MessageId".
 */
export type MessageId = string;
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ContentFormat".
 */
export type ContentFormat = "plain" | "markdown";

/**
 * The current bidirectional WebSocket payloads.
 *
 * This is a generation root rather than a second wire model: both fields point
 * directly at the types serialized and deserialized by the production socket.
 */
export interface ConcordWebSocketContract {
  client_message: ClientMessage;
  server_event: ChatEvent;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "MessageMention".
 */
export interface MessageMention {
  end_byte: number;
  kind: MentionKind;
  start_byte: number;
  target_id?: string | null;
}
/**
 * Minimal channel position info for reorder events.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ChannelPositionInfo".
 */
export interface ChannelPositionInfo {
  category_id?: string | null;
  id: string;
  position: number;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SyncSnapshot".
 */
export interface SyncSnapshot {
  cursor: string;
  history_before: {
    [k: string]: string;
  };
  messages: DurableMessageProjection[];
  operation_generation: string;
  protocol_version: number;
  reactions: SnapshotReactionGroup[];
  read_states: DurableReadProjection[];
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DurableMessageProjection".
 */
export interface DurableMessageProjection {
  attachments: DurableAttachmentProjection[];
  components?: MessageComponent[] | null;
  content?: string | null;
  content_format: string;
  conversation_id: ConversationId;
  created_at: string;
  deleted: boolean;
  edited_at?: string | null;
  entity_version: number;
  mentions: MessageMention[];
  message_id: string;
  reply_to?: DurableReplyProjection | null;
  reply_to_id?: string | null;
  rich_embeds?: RichEmbedInfo[] | null;
  sender_id: string;
  sender_nick: string;
  sequence: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DurableAttachmentProjection".
 */
export interface DurableAttachmentProjection {
  attachment_id: string;
  content_type: string;
  file_size: number;
  filename: string;
  state_version: number;
}
/**
 * An option in a select menu component.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SelectOption".
 */
export interface SelectOption {
  default?: boolean;
  description?: string | null;
  emoji?: string | null;
  label: string;
  value: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DurableReplyProjection".
 */
export interface DurableReplyProjection {
  content?: string | null;
  deleted: boolean;
  message_id: string;
  sender_id: string;
  sender_nick: string;
}
/**
 * Rich embed format for bot messages.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "RichEmbedInfo".
 */
export interface RichEmbedInfo {
  author?: EmbedAuthor | null;
  color?: string | null;
  description?: string | null;
  fields?: EmbedField[] | null;
  footer?: EmbedFooter | null;
  image_url?: string | null;
  thumbnail_url?: string | null;
  timestamp?: string | null;
  title?: string | null;
  url?: string | null;
}
/**
 * Author section for a rich embed.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "EmbedAuthor".
 */
export interface EmbedAuthor {
  icon_url?: string | null;
  name: string;
  url?: string | null;
}
/**
 * A field in a rich embed.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "EmbedField".
 */
export interface EmbedField {
  inline?: boolean;
  name: string;
  value: string;
}
/**
 * Footer for a rich embed.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "EmbedFooter".
 */
export interface EmbedFooter {
  icon_url?: string | null;
  text: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SnapshotReactionGroup".
 */
export interface SnapshotReactionGroup {
  count: number;
  emoji: string;
  message_id: string;
  reacted_by_me: boolean;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DurableReadProjection".
 */
export interface DurableReadProjection {
  conversation_id: ConversationId;
  entity_version: number;
  message_id: string;
  sequence: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ReplayBatch".
 */
export interface ReplayBatch {
  cursor: string;
  events: DurableEventProjection[];
  has_more: boolean;
  operation_generation: string;
  protocol_version: number;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DurableEventProjection".
 */
export interface DurableEventProjection {
  conversation_id: ConversationId;
  descriptor: unknown;
  entity_id: string;
  entity_type: string;
  entity_version: number;
  kind: string;
  message?: DurableMessageProjection | null;
  reaction?: DurableReactionProjection | null;
  read_state?: DurableReadProjection | null;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DurableReactionProjection".
 */
export interface DurableReactionProjection {
  emoji: string;
  entity_version: number;
  message_id: string;
  present: boolean;
  user_id: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "CommandReceipt".
 */
export interface CommandReceipt {
  client_message_id: string;
  entity_version: number;
  message_id: string;
  persisted_at: string;
  replayed?: boolean;
  request_id: string;
  /**
   * Decimal JSON string; JavaScript clients must not parse this as a number.
   */
  sequence: string;
}
/**
 * Metadata for a file attachment.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "AttachmentInfo".
 */
export interface AttachmentInfo {
  content_type: string;
  file_size: number;
  filename: string;
  id: string;
  url: string;
}
/**
 * Info about a replied-to message.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ReplyInfo".
 */
export interface ReplyInfo {
  content_preview: string;
  from: string;
  id: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "MemberInfo".
 */
export interface MemberInfo {
  avatar_url?: string | null;
  custom_status?: string | null;
  nickname: string;
  role_ids?: string[];
  server_avatar_url?: string | null;
  status?: string | null;
  status_emoji?: string | null;
  user_id?: string | null;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ChannelInfo".
 */
export interface ChannelInfo {
  archived: boolean;
  category_id?: string | null;
  channel_type: string;
  conversation_id: string;
  id: string;
  is_nsfw: boolean;
  is_private: boolean;
  member_count: number;
  name: string;
  position: number;
  server_id: string;
  slowmode_seconds: number;
  thread_parent_message_id?: string | null;
  topic: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "HistoryMessage".
 */
export interface HistoryMessage {
  attachments?: AttachmentInfo[] | null;
  components?: MessageComponent[] | null;
  content: string;
  edited_at?: string | null;
  embeds?: EmbedInfo[] | null;
  from: string;
  id: MessageId;
  reactions?: ReactionGroup[] | null;
  reply_to?: ReplyInfo | null;
  rich_embeds?: RichEmbedInfo[] | null;
  timestamp: string;
}
/**
 * Open Graph link embed preview metadata.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "EmbedInfo".
 */
export interface EmbedInfo {
  description?: string | null;
  image_url?: string | null;
  site_name?: string | null;
  title?: string | null;
  url: string;
}
/**
 * Grouped reactions for a message in history.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ReactionGroup".
 */
export interface ReactionGroup {
  count: number;
  emoji: string;
  user_ids: string[];
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ServerInfo".
 */
export interface ServerInfo {
  icon_url?: string | null;
  id: string;
  member_count: number;
  /**
   * Effective permission bitfield for the requesting user in this server.
   */
  my_permissions?: number;
  name: string;
  role?: string | null;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "UnreadCount".
 */
export interface UnreadCount {
  channel_name: string;
  count: number;
}
/**
 * One member's role assignments in an authoritative role bootstrap.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "MemberRoleInfo".
 */
export interface MemberRoleInfo {
  role_ids: string[];
  user_id: string;
}
/**
 * Role metadata sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "RoleInfo".
 */
export interface RoleInfo {
  color?: string | null;
  icon_url?: string | null;
  id: string;
  is_default: boolean;
  name: string;
  permissions: number;
  position: number;
  server_id: string;
}
/**
 * A role- or member-specific permission rule for one channel.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ChannelPermissionOverrideInfo".
 */
export interface ChannelPermissionOverrideInfo {
  allow_bits: number;
  channel_id: string;
  deny_bits: number;
  id: string;
  target_id: string;
  target_type: string;
}
/**
 * Channel category metadata sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "CategoryInfo".
 */
export interface CategoryInfo {
  id: string;
  name: string;
  position: number;
  server_id: string;
}
/**
 * User presence info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "PresenceInfo".
 */
export interface PresenceInfo {
  avatar_url?: string | null;
  custom_status?: string | null;
  nickname: string;
  status: string;
  status_emoji?: string | null;
  user_id: string;
}
/**
 * Full user profile info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "UserProfileInfo".
 */
export interface UserProfileInfo {
  avatar_url?: string | null;
  banner_url?: string | null;
  bio?: string | null;
  created_at: string;
  pronouns?: string | null;
  user_id: string;
  username: string;
}
/**
 * Notification setting info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "NotificationSettingInfo".
 */
export interface NotificationSettingInfo {
  channel_id?: string | null;
  id: string;
  level: string;
  mute_until?: string | null;
  muted: boolean;
  server_id?: string | null;
  suppress_everyone: boolean;
  suppress_roles: boolean;
}
/**
 * A search result message (same as HistoryMessage but with channel info).
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SearchResultMessage".
 */
export interface SearchResultMessage {
  channel_id: string;
  channel_name: string;
  content: string;
  edited_at?: string | null;
  from: string;
  id: string;
  timestamp: string;
}
/**
 * Info about a pinned message.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "PinnedMessageInfo".
 */
export interface PinnedMessageInfo {
  channel_id: string;
  content: string;
  from: string;
  id: string;
  message_id: string;
  pinned_at: string;
  pinned_by: string;
  timestamp: string;
}
/**
 * Info about a thread.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ThreadInfo".
 */
export interface ThreadInfo {
  archived: boolean;
  auto_archive_minutes: number;
  channel_type: string;
  created_at: string;
  creator_user_id?: string | null;
  id: string;
  message_count: number;
  name: string;
  parent_message_id?: string | null;
  state_version?: number;
  tag_ids?: string[];
  tags_version?: number;
}
/**
 * Forum tag info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ForumTagInfo".
 */
export interface ForumTagInfo {
  emoji?: string | null;
  id: string;
  moderated: boolean;
  name: string;
  position: number;
}
/**
 * Bookmark info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "BookmarkInfo".
 */
export interface BookmarkInfo {
  channel_id: string;
  content: string;
  created_at: string;
  from: string;
  id: string;
  message_id: string;
  note?: string | null;
  timestamp: string;
}
/**
 * Direct conversation navigation entry sent only to a participant.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "DirectConversationInfo".
 */
export interface DirectConversationInfo {
  id: string;
  last_message_at?: string | null;
  peer_avatar_url?: string | null;
  peer_id: string;
  peer_username: string;
  unread_count: number;
}
/**
 * Audit log entry sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "AuditLogEntry".
 */
export interface AuditLogEntry {
  action_type: string;
  actor_avatar_snapshot?: string | null;
  actor_id: string;
  actor_username_snapshot: string;
  changes?: string | null;
  created_at: string;
  id: string;
  reason?: string | null;
  target_id?: string | null;
  target_type?: string | null;
}
/**
 * Ban info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "BanInfo".
 */
export interface BanInfo {
  banned_by: string;
  created_at: string;
  id: string;
  reason?: string | null;
  user_id: string;
}
/**
 * AutoMod rule info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "AutomodRuleInfo".
 */
export interface AutomodRuleInfo {
  action_type: string;
  config: string;
  enabled: boolean;
  id: string;
  name: string;
  rule_type: string;
  timeout_duration_seconds?: number | null;
}
/**
 * Server invite info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "InviteInfo".
 */
export interface InviteInfo {
  channel_id?: string | null;
  code: string;
  created_at: string;
  created_by: string;
  expires_at?: string | null;
  id: string;
  max_uses?: number | null;
  server_id: string;
  use_count: number;
}
/**
 * Scheduled event info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "EventInfo".
 */
export interface EventInfo {
  channel_id?: string | null;
  created_at: string;
  created_by: string;
  description?: string | null;
  end_time?: string | null;
  id: string;
  image_url?: string | null;
  interested_count: number;
  name: string;
  server_id: string;
  start_time: string;
  status: string;
}
/**
 * RSVP info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "RsvpInfo".
 */
export interface RsvpInfo {
  status: string;
  user_id: string;
}
/**
 * Server community/discovery info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ServerCommunityInfo".
 */
export interface ServerCommunityInfo {
  category?: string | null;
  description?: string | null;
  is_discoverable: boolean;
  /**
   * Whether the requesting member accepted the server's current rules version.
   * Omitted from public discovery results.
   */
  rules_accepted?: boolean | null;
  rules_text?: string | null;
  server_id: string;
  welcome_message?: string | null;
}
/**
 * Channel follow info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "ChannelFollowInfo".
 */
export interface ChannelFollowInfo {
  created_by: string;
  id: string;
  source_channel_id: string;
  target_channel_id: string;
}
/**
 * Server template info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "TemplateInfo".
 */
export interface TemplateInfo {
  created_at: string;
  created_by: string;
  description?: string | null;
  id: string;
  name: string;
  server_id: string;
  use_count: number;
}
/**
 * Webhook info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "WebhookInfo".
 */
export interface WebhookInfo {
  avatar_url?: string | null;
  channel_id: string;
  created_at: string;
  created_by: string;
  id: string;
  name: string;
  server_id: string;
  token: string;
  url?: string | null;
  webhook_type: string;
}
/**
 * Slash command info sent to clients.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SlashCommandInfo".
 */
export interface SlashCommandInfo {
  bot_user_id: string;
  description: string;
  id: string;
  name: string;
  options: SlashCommandOption[];
}
/**
 * A single option/parameter for a slash command.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SlashCommandOption".
 */
export interface SlashCommandOption {
  choices?: SlashCommandChoice[] | null;
  description: string;
  name: string;
  option_type: string;
  required?: boolean;
}
/**
 * A pre-defined choice for a slash command option.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "SlashCommandChoice".
 */
export interface SlashCommandChoice {
  name: string;
  value: string;
}
/**
 * Interaction info sent to bots.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "InteractionInfo".
 */
export interface InteractionInfo {
  channel_id: string;
  command_name?: string | null;
  data: unknown;
  id: string;
  interaction_type: string;
  server_id: string;
  user_id: string;
}
/**
 * Bot's response to an interaction.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "InteractionResponseData".
 */
export interface InteractionResponseData {
  components?: MessageComponent[] | null;
  content?: string | null;
  embeds?: RichEmbedInfo[] | null;
  ephemeral?: boolean;
}
/**
 * Bot token info (without the actual hash).
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "BotTokenInfo".
 */
export interface BotTokenInfo {
  created_at: string;
  id: string;
  last_used?: string | null;
  name: string;
  scopes: string;
}
/**
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "BotAccountInfo".
 */
export interface BotAccountInfo {
  avatar_url?: string | null;
  id: string;
  installed_server_ids: string[];
  username: string;
}
/**
 * OAuth2 application info.
 *
 * This interface was referenced by `ConcordWebSocketContract`'s JSON-Schema
 * via the `definition` "OAuth2AppInfo".
 */
export interface OAuth2AppInfo {
  created_at: string;
  description: string;
  icon_url?: string | null;
  id: string;
  is_public: boolean;
  name: string;
  owner_id: string;
  redirect_uris: string[];
  scopes: string;
}
