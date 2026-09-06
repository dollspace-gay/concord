use super::Instant;

pub(super) fn fixed_window_admit(count: &mut u32, window_start: &mut Instant, limit: u32) -> bool {
    if window_start.elapsed() >= std::time::Duration::from_secs(1) {
        *count = 0;
        *window_start = Instant::now();
    }
    *count = count.saturating_add(1);
    let admitted = *count <= limit;
    crate::runtime_metrics::record(
        crate::runtime_metrics::Operation::CommandAdmission,
        admitted,
        std::time::Duration::ZERO,
    );
    admitted
}

pub(super) fn websocket_command_correlation(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("request_id")
                .or_else(|| value.get("nonce"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 128)
                .map(str::to_owned)
        })
}

/// Read-only bootstrap and query commands have an independent admission budget
/// so reconnect hydration cannot consume the budget needed for user mutations.
pub(super) fn websocket_command_is_read(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    matches!(
        value.get("type").and_then(serde_json::Value::as_str),
        Some(
            "sync"
                | "fetch_history"
                | "list_channels"
                | "get_members"
                | "list_servers"
                | "list_direct_conversations"
                | "get_unread_counts"
                | "list_roles"
                | "list_channel_permission_overrides"
                | "list_categories"
                | "get_presences"
                | "search_messages"
                | "get_notification_settings"
                | "get_user_profile"
                | "get_pinned_messages"
                | "list_threads"
                | "list_forum_tags"
                | "get_thread_tags"
                | "list_bookmarks"
                | "list_bans"
                | "get_audit_log"
                | "list_automod_rules"
                | "list_invites"
                | "list_events"
                | "list_rsvps"
                | "get_community_settings"
                | "discover_servers"
                | "list_channel_follows"
                | "list_templates"
                | "list_webhooks"
                | "list_owned_bots"
                | "list_bot_tokens"
                | "list_slash_commands"
                | "list_o_auth2_apps"
                | "get_server_limits"
                | "get_bluesky_identity"
                | "get_atproto_sync_setting"
        )
    )
}
