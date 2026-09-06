use super::{
    CategoryInfo, ChannelId, MemberRoleInfo, ModerationError, RoleInfo, ServerId, SqlitePool, Utc,
    WebhookInfo,
};

pub(super) fn referenced_server_id(value: &str) -> Result<ServerId, String> {
    ServerId::from_stored(value.to_owned())
        .map_err(|_| "INVALID_INPUT: invalid server id".to_owned())
}

pub(super) fn referenced_channel_id(value: &str) -> Result<ChannelId, String> {
    ChannelId::from_stored(value.to_owned())
        .map_err(|_| "INVALID_INPUT: invalid channel id".to_owned())
}

pub(super) fn moderation_wire(error: ModerationError) -> String {
    error.wire_message()
}

pub(super) fn moderation_dependency() -> String {
    moderation_wire(ModerationError::DependencyUnavailable)
}

pub(super) fn moderation_unavailable() -> String {
    moderation_wire(ModerationError::Unavailable)
}

pub(super) fn moderation_unauthenticated() -> String {
    moderation_wire(ModerationError::Unauthenticated)
}

pub(super) async fn server_member_display_identity(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Option<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT COALESCE(NULLIF(sm.nickname,''),u.username), \
                COALESCE(sm.avatar_url,u.avatar_url) \
         FROM server_members sm JOIN users u ON u.id=sm.user_id \
         WHERE sm.server_id=? AND sm.user_id=? AND NOT EXISTS( \
             SELECT 1 FROM bans b WHERE b.server_id=sm.server_id AND b.user_id=sm.user_id \
         )",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub(super) fn group_member_roles(
    assignments: Vec<(String, Option<String>)>,
) -> Vec<MemberRoleInfo> {
    let mut by_user = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (user_id, role_id) in assignments {
        let role_ids = by_user.entry(user_id).or_default();
        if let Some(role_id) = role_id {
            role_ids.push(role_id);
        }
    }
    by_user
        .into_iter()
        .map(|(user_id, role_ids)| MemberRoleInfo { user_id, role_ids })
        .collect()
}

/// Convert a RoleRow to a RoleInfo for client consumption.
pub(super) fn role_row_to_info(row: crate::db::models::RoleRow) -> RoleInfo {
    RoleInfo {
        id: row.id,
        server_id: row.server_id,
        name: row.name,
        color: row.color,
        icon_url: row.icon_url,
        position: row.position,
        permissions: row.permissions,
        is_default: row.is_default != 0,
    }
}

/// Convert a ChannelCategoryRow to a CategoryInfo for client consumption.
pub(super) fn category_row_to_info(row: crate::db::models::ChannelCategoryRow) -> CategoryInfo {
    CategoryInfo {
        id: row.id,
        server_id: row.server_id,
        name: row.name,
        position: row.position,
    }
}

/// Convert a WebhookRow to a WebhookInfo for client consumption.
pub(super) fn webhook_row_to_info(row: crate::db::models::WebhookRow) -> WebhookInfo {
    WebhookInfo {
        id: row.id,
        server_id: row.server_id,
        channel_id: row.channel_id,
        name: row.name,
        avatar_url: row.avatar_url,
        webhook_type: row.webhook_type,
        token: String::new(),
        url: row.url,
        created_by: row.created_by,
        created_at: row.created_at,
    }
}

/// Ensure channel names are lowercase and start with #.
pub(super) fn normalize_channel_name(name: &str) -> String {
    let name = name.to_lowercase();
    if name.starts_with('#') {
        name
    } else {
        format!("#{name}")
    }
}

pub(super) fn channel_conversation_id(channel_id: &str) -> String {
    let mut id = String::with_capacity(8 + channel_id.len() * 2);
    id.push_str("channel:");
    for byte in channel_id.as_bytes() {
        use std::fmt::Write as _;
        write!(&mut id, "{byte:02X}").expect("writing to String cannot fail");
    }
    id
}

pub(super) fn stable_irc_alias(name: &str, id: &str) -> String {
    let mut alias = String::new();
    let mut separator = false;
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            alias.push(character);
            separator = false;
        } else if !alias.is_empty() && !separator {
            alias.push('-');
            separator = true;
        }
    }
    while alias.ends_with('-') {
        alias.pop();
    }
    if alias.is_empty() {
        alias.push_str("server");
    }
    alias.truncate(20);
    let id_prefix: String = id.chars().take(8).collect();
    format!("{}-{id_prefix}", alias.trim_end_matches('-'))
}

pub(super) fn parse_persisted_timestamp(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .map(|timestamp| timestamp.and_utc())
                .ok()
        })
}
