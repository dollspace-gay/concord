use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::authorization::AuthorizationStamp;
use super::events::{ChatEvent, ConnectionId};

/// Maximum queued outbound descriptors per connection.
pub const MAX_OUTBOUND_QUEUE: usize = 256;
/// Maximum serialized payload bytes queued per connection.
pub const MAX_OUTBOUND_BYTES: usize = 1024 * 1024;

/// Authorization evidence that must still be valid immediately before delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryGuard {
    ActorCurrent,
    Stamps(Vec<AuthorizationStamp>),
    Conversations(Vec<String>),
    Channels(Vec<String>),
    ChannelActions(Vec<(String, super::authorization::ChannelAction)>),
    ServerMembership(Vec<String>),
    ServerPermissions(Vec<(String, super::permissions::Permissions)>),
    BotInstallationScopes(Vec<(String, String)>),
}

impl DeliveryGuard {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ActorCurrent => "actor_current",
            Self::Stamps(_) => "authorization_stamps",
            Self::Conversations(_) => "conversations",
            Self::Channels(_) => "channels",
            Self::ChannelActions(_) => "channel_actions",
            Self::ServerMembership(_) => "server_membership",
            Self::ServerPermissions(_) => "server_permissions",
            Self::BotInstallationScopes(_) => "bot_installation_scopes",
        }
    }
}

#[derive(Debug)]
struct QueuedEnvelope {
    serialized_bytes: usize,
    guard: Option<DeliveryGuard>,
}

#[derive(Debug, Default)]
struct OutboundState {
    queued_bytes: usize,
    envelopes: VecDeque<QueuedEnvelope>,
}

/// Which protocol this session connected via.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Irc,
    WebSocket,
}

/// A connected user session. Protocol-agnostic — the engine doesn't care
/// whether this is an IRC client or a web browser.
#[derive(Debug)]
pub struct UserSession {
    pub id: ConnectionId,
    /// Database user ID (None for unauthenticated/guest sessions).
    pub user_id: Option<String>,
    pub nickname: String,
    pub protocol: Protocol,
    /// Send outbound events to this session's write loop (bounded to prevent memory exhaustion).
    outbound: mpsc::Sender<ChatEvent>,
    outbound_state: Mutex<OutboundState>,
    overflow: CancellationToken,
    pub connected_at: DateTime<Utc>,
    /// Avatar URL (from Bluesky profile or other source).
    pub avatar_url: Option<String>,
}

impl UserSession {
    pub fn new(
        id: ConnectionId,
        user_id: Option<String>,
        nickname: String,
        protocol: Protocol,
        outbound: mpsc::Sender<ChatEvent>,
        avatar_url: Option<String>,
    ) -> Self {
        Self {
            id,
            user_id,
            nickname,
            protocol,
            outbound,
            outbound_state: Mutex::new(OutboundState::default()),
            overflow: CancellationToken::new(),
            connected_at: Utc::now(),
            avatar_url,
        }
    }

    /// Queue an event. Exceeding either queue bound cancels the transport so the
    /// client reconnects and resynchronizes instead of silently losing events.
    pub fn send(&self, event: ChatEvent) -> bool {
        let Some(guard) = default_delivery_guard(&event) else {
            self.overflow.cancel();
            return false;
        };
        self.send_guarded(event, Some(guard))
    }

    pub fn send_guarded(&self, event: ChatEvent, guard: Option<DeliveryGuard>) -> bool {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::OutboundQueue);
        let mut ack_metric = matches!(
            &event,
            ChatEvent::MessageAck { .. }
                | ChatEvent::CommandCommitted { .. }
                | ChatEvent::LifecycleCommandSucceeded { .. }
        )
        .then(|| {
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::CommandAck)
        });
        let mut resync_metric = matches!(&event, ChatEvent::ResyncRequired { .. }).then(|| {
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Resync)
        });
        let guard = match guard.or_else(|| default_delivery_guard(&event)) {
            Some(guard) => guard,
            None => {
                self.overflow.cancel();
                return false;
            }
        };
        let serialized_bytes = match serde_json::to_vec(&event) {
            Ok(payload) => payload.len(),
            Err(_) => {
                self.overflow.cancel();
                return false;
            }
        };
        let mut state = self.outbound_state.lock().expect("outbound state poisoned");
        if state.envelopes.len() >= MAX_OUTBOUND_QUEUE
            || state.queued_bytes.saturating_add(serialized_bytes) > MAX_OUTBOUND_BYTES
        {
            drop(state);
            crate::runtime_metrics::record(
                crate::runtime_metrics::Operation::QueueOverflow,
                false,
                std::time::Duration::ZERO,
            );
            self.overflow.cancel();
            return false;
        }
        match self.outbound.try_send(event) {
            Ok(()) => {
                state.queued_bytes += serialized_bytes;
                state.envelopes.push_back(QueuedEnvelope {
                    serialized_bytes,
                    guard: Some(guard),
                });
                metric.succeed();
                if let Some(metric) = &mut ack_metric {
                    metric.succeed();
                }
                if let Some(metric) = &mut resync_metric {
                    metric.succeed();
                }
                true
            }
            Err(_) => {
                drop(state);
                crate::runtime_metrics::record(
                    crate::runtime_metrics::Operation::QueueOverflow,
                    false,
                    std::time::Duration::ZERO,
                );
                self.overflow.cancel();
                false
            }
        }
    }

    /// Consume metadata for the event just received from the outbound channel.
    pub fn take_delivery_guard(&self) -> Option<DeliveryGuard> {
        let mut state = self.outbound_state.lock().expect("outbound state poisoned");
        let envelope = state
            .envelopes
            .pop_front()
            .expect("outbound event missing envelope metadata");
        state.queued_bytes = state.queued_bytes.saturating_sub(envelope.serialized_bytes);
        envelope.guard
    }

    pub fn overflow_cancellation_token(&self) -> CancellationToken {
        self.overflow.clone()
    }
}

/// Exhaustive typed fallback policy for ordinary call sites. Events whose
/// authorization scope cannot be recovered from their typed payload return
/// `None` and must use `send_guarded` with authorization evidence.
fn default_delivery_guard(event: &ChatEvent) -> Option<DeliveryGuard> {
    use super::permissions::Permissions;
    use ChatEvent::*;

    let server_member =
        |server_id: &String| DeliveryGuard::ServerMembership(vec![server_id.clone()]);
    let server_permission = |server_id: &String, permission| {
        DeliveryGuard::ServerPermissions(vec![(server_id.clone(), permission)])
    };

    Some(match event {
        SyncSnapshot { .. }
        | ReplayBatch { .. }
        | DurableEvent { .. }
        | Message { .. }
        | MessageEdit { .. }
        | MessageDelete { .. }
        | ReactionAdd { .. }
        | ReactionRemove { .. }
        | TypingStart { .. }
        | Join { .. }
        | Part { .. }
        | TopicChange { .. }
        | Names { .. }
        | Topic { .. }
        | History { .. }
        | ChannelList { .. }
        | UnreadCounts { .. }
        | EventList { .. }
        | MessageEmbed { .. }
        | SearchResults { .. }
        | MessagePin { .. }
        | MessageUnpin { .. }
        | PinnedMessages { .. }
        | ThreadList { .. }
        | ForumTagList { .. }
        | ForumTagUpdate { .. }
        | ForumTagDelete { .. }
        | SlowModeUpdate { .. }
        | NsfwUpdate { .. }
        | BulkMessageDelete { .. }
        | BookmarkRemove { .. }
        | EventRsvpList { .. }
        | ChannelFollowDelete { .. }
        | InteractionResponse { .. }
        | Quit { .. }
        | NickChange { .. }
        | UserProfile { .. } => return None,
        MessageAck { .. }
        | InteractionInvoked { .. }
        | LifecycleCommandSucceeded { .. }
        | ResyncRequired { .. }
        | CommandError { .. }
        | CommandCommitted { .. }
        | ServerNotice { .. }
        | BotTokenList { .. }
        | BotAccountList { .. }
        | BotCredentialCreated { .. }
        | OAuth2AppList { .. }
        | OAuth2AppUpdate { .. }
        | BlueskyProfileSync { .. }
        | BlueskyShareResult { .. }
        | DirectConversationList { .. }
        | ServerLimits { .. }
        | Error { .. }
        | DiscoverServers { .. } => DeliveryGuard::ActorCurrent,
        OwnPresence { .. } => DeliveryGuard::ActorCurrent,
        AnnouncementPublished { .. } => DeliveryGuard::ActorCurrent,
        ThreadCreate { thread, .. } | ThreadUpdate { thread, .. } => {
            DeliveryGuard::Channels(vec![thread.id.clone()])
        }
        ThreadTagUpdate { thread_id, .. } => DeliveryGuard::Channels(vec![thread_id.clone()]),
        EventUpdate { server_id, event } if event.channel_id.is_none() => server_member(server_id),
        EventUpdate { event, .. } => DeliveryGuard::Channels(vec![
            event.channel_id.clone().expect("matched linked event"),
        ]),
        RoleList { server_id, .. }
        | RoleUpdate { server_id, .. }
        | RoleDelete { server_id, .. }
        | MemberRoleUpdate { server_id, .. }
        | ChannelPermissionOverrideList { server_id, .. }
        | CategoryList { server_id, .. }
        | CategoryUpdate { server_id, .. }
        | CategoryDelete { server_id, .. }
        | PresenceUpdate { server_id, .. }
        | PresenceList { server_id, .. }
        | ServerNicknameUpdate { server_id, .. }
        | NotificationSettings { server_id, .. }
        | MemberKick { server_id, .. }
        | MemberBan { server_id, .. }
        | MemberUnban { server_id, .. }
        | MemberTimeout { server_id, .. } => server_member(server_id),
        EventDelete { server_id, .. }
        | TemplateList { server_id, .. }
        | TemplateUpdate { server_id, .. }
        | TemplateDelete { server_id, .. }
        | TemplateInstantiated { server_id, .. }
        | ServerAvatarUpdate { server_id, .. } => server_member(server_id),
        ChannelReorder { server_id, .. } => {
            server_permission(server_id, Permissions::MANAGE_CHANNELS)
        }
        AuditLogEntries { server_id, .. }
        | AutomodRuleList { server_id, .. }
        | AutomodRuleUpdate { server_id, .. }
        | AutomodRuleDelete { server_id, .. }
        | InviteList { server_id, .. }
        | WebhookList { server_id, .. }
        | WebhookUpdate { server_id, .. }
        | WebhookDelete { server_id, .. }
        | SlashCommandList { server_id, .. }
        | SlashCommandUpdate { server_id, .. }
        | SlashCommandDelete { server_id, .. } => {
            server_permission(server_id, Permissions::MANAGE_SERVER)
        }
        BanList { server_id, .. } => server_permission(server_id, Permissions::BAN_MEMBERS),
        InviteCreate { server_id, .. } | InviteDelete { server_id, .. } => {
            server_permission(server_id, Permissions::CREATE_INVITES)
        }
        ServerList { servers } => DeliveryGuard::ServerMembership(
            servers.iter().map(|server| server.id.clone()).collect(),
        ),
        BookmarkList { bookmarks } => DeliveryGuard::ChannelActions(
            bookmarks
                .iter()
                .map(|bookmark| {
                    (
                        bookmark.channel_id.clone(),
                        super::authorization::ChannelAction::ReadHistory,
                    )
                })
                .collect(),
        ),
        BookmarkAdd { bookmark } => DeliveryGuard::ChannelActions(vec![(
            bookmark.channel_id.clone(),
            super::authorization::ChannelAction::ReadHistory,
        )]),
        ServerCommunity { community } => server_member(&community.server_id),
        ChannelFollowList { channel_id, .. } => DeliveryGuard::Channels(vec![channel_id.clone()]),
        ChannelFollowCreate { follow } => DeliveryGuard::Channels(vec![
            follow.source_channel_id.clone(),
            follow.target_channel_id.clone(),
        ]),
        InteractionCreate { interaction } => {
            DeliveryGuard::Channels(vec![interaction.channel_id.clone()])
        }
    })
}

#[cfg(test)]
mod tests;
