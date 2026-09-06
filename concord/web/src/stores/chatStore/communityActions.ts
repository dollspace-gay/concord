import type { ChatState, ChatStoreContext } from './types';

export function createCommunityActions({ set, get }: ChatStoreContext): Pick<ChatState, 'createInvite' | 'listInvites' | 'deleteInvite' | 'useInvite' | 'createEvent' | 'listEvents' | 'updateEventStatus' | 'deleteEvent' | 'setRsvp' | 'removeRsvp' | 'listRsvps' | 'updateCommunitySettings' | 'getCommunitySettings' | 'discoverServers' | 'acceptRules'> {
  return {
    // ── Phase 7: Community & Discovery ──
    createInvite: (serverId, maxUses, expiresAt, channelId) => {
      return get().runLifecycleCommand(
        { type: 'create_invite', server_id: serverId, max_uses: maxUses, expires_at: expiresAt, channel_id: channelId },
        `community:${serverId}:invite:create`,
      );
    },

    listInvites: (serverId) => {
      get().ws?.send({ type: 'list_invites', server_id: serverId });
    },

    deleteInvite: (serverId, inviteId) => {
      return get().runLifecycleCommand(
        { type: 'delete_invite', server_id: serverId, invite_id: inviteId },
        `community:${serverId}:invite:${inviteId}`,
      );
    },

    useInvite: (code) => {
      return get().runLifecycleCommand(
        { type: 'use_invite', code },
        `community:invite:redeem:${code}`,
      );
    },

    createEvent: (serverId, name, startTime, options) => {
      return get().runLifecycleCommand(
        { type: 'create_event', server_id: serverId, name, start_time: startTime, description: options?.description, channel_id: options?.channelId, end_time: options?.endTime, image_url: options?.imageUrl },
        `community:${serverId}:event:create`,
      );
    },

    listEvents: (serverId) => {
      get().ws?.send({ type: 'list_events', server_id: serverId });
    },

    updateEventStatus: (serverId, eventId, status) => {
      return get().runLifecycleCommand(
        { type: 'update_event_status', server_id: serverId, event_id: eventId, status },
        `community:${serverId}:event:${eventId}`,
      );
    },

    deleteEvent: (serverId, eventId) => {
      return get().runLifecycleCommand(
        { type: 'delete_event', server_id: serverId, event_id: eventId },
        `community:${serverId}:event:${eventId}`,
      );
    },

    setRsvp: (serverId, eventId, status) => {
      return get().runLifecycleCommand(
        { type: 'set_rsvp', server_id: serverId, event_id: eventId, status },
        `community:${serverId}:event:${eventId}:rsvp`,
      );
    },

    removeRsvp: (serverId, eventId) => {
      return get().runLifecycleCommand(
        { type: 'remove_rsvp', server_id: serverId, event_id: eventId },
        `community:${serverId}:event:${eventId}:rsvp`,
      );
    },

    listRsvps: (eventId) => {
      get().ws?.send({ type: 'list_rsvps', event_id: eventId });
    },

    updateCommunitySettings: (serverId, settings) => {
      return get().runLifecycleCommand(
        { type: 'update_community_settings', server_id: serverId, description: settings.description, is_discoverable: settings.isDiscoverable, welcome_message: settings.welcomeMessage, rules_text: settings.rulesText, category: settings.category },
        `community:${serverId}:settings`,
      );
    },

    getCommunitySettings: (serverId) => {
      get().ws?.send({ type: 'get_community_settings', server_id: serverId });
    },

    discoverServers: (category) => {
      get().ws?.send({ type: 'discover_servers', category });
    },

    acceptRules: (serverId) => {
      return get().runLifecycleCommand(
        { type: 'accept_rules', server_id: serverId },
        `community:${serverId}:rules:accept`,
      ).then(() => set((state) => {
        const community = state.communitySettings[serverId];
        return community ? { communitySettings: { ...state.communitySettings, [serverId]: { ...community, rules_accepted: true } } } : {};
      }));
    }
  };
}
