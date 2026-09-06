import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { ChatStoreContext } from '../types';

export function handleCommunityEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'invite_list' | 'invite_create' | 'invite_delete' | 'event_list' | 'event_update' | 'event_delete' | 'event_rsvp_list' | 'server_community' | 'discover_servers' | 'channel_follow_list' | 'channel_follow_create' | 'channel_follow_delete' | 'template_list' | 'template_update' | 'template_delete' | 'template_instantiated' }>) {
  switch (event.type) {
    // ── Phase 7: Community & Discovery events ──
    case 'invite_list':
      set({ invites: { ...get().invites, [event.server_id]: event.invites } });
      break;
    case 'invite_create':
      set({ invites: { ...get().invites, [event.server_id]: [...(get().invites[event.server_id] || []), event.invite] } });
      break;
    case 'invite_delete':
      set({ invites: { ...get().invites, [event.server_id]: (get().invites[event.server_id] || []).filter(i => i.id !== event.invite_id) } });
      break;
    case 'event_list':
      set({ serverEvents: { ...get().serverEvents, [event.server_id]: event.events } });
      break;
    case 'event_update': {
      const existing = get().serverEvents[event.server_id] || [];
      const idx = existing.findIndex(e => e.id === event.event.id);
      const updated = idx >= 0 ? [...existing.slice(0, idx), event.event, ...existing.slice(idx + 1)] : [...existing, event.event];
      set({ serverEvents: { ...get().serverEvents, [event.server_id]: updated } });
      break;
    }
    case 'event_delete':
      set({ serverEvents: { ...get().serverEvents, [event.server_id]: (get().serverEvents[event.server_id] || []).filter(e => e.id !== event.event_id) } });
      break;
    case 'event_rsvp_list':
      set({ eventRsvps: { ...get().eventRsvps, [event.event_id]: event.rsvps } });
      break;
    case 'server_community':
      set({ communitySettings: { ...get().communitySettings, [event.community.server_id]: event.community } });
      break;
    case 'discover_servers':
      set({ discoverableServers: event.servers });
      break;
    case 'channel_follow_list':
      set({ channelFollows: { ...get().channelFollows, [event.channel_id]: event.follows } });
      break;
    case 'channel_follow_create': {
      const existing = get().channelFollows[event.follow.source_channel_id] ?? [];
      set({
        channelFollows: {
          ...get().channelFollows,
          [event.follow.source_channel_id]: [
            ...existing.filter((follow) => follow.id !== event.follow.id),
            event.follow,
          ],
        },
      });
      break;
    }
    case 'channel_follow_delete':
      set({
        channelFollows: Object.fromEntries(
          Object.entries(get().channelFollows).map(([channelId, follows]) => [
            channelId,
            follows.filter((follow) => follow.id !== event.follow_id),
          ]),
        ),
      });
      break;
    case 'template_list':
      set({ templates: { ...get().templates, [event.server_id]: event.templates } });
      break;
    case 'template_update': {
      const existing = get().templates[event.server_id] || [];
      const idx = existing.findIndex(t => t.id === event.template.id);
      const updated = idx >= 0 ? [...existing.slice(0, idx), event.template, ...existing.slice(idx + 1)] : [...existing, event.template];
      set({ templates: { ...get().templates, [event.server_id]: updated } });
      break;
    }
    case 'template_delete':
      set({ templates: { ...get().templates, [event.server_id]: (get().templates[event.server_id] || []).filter(t => t.id !== event.template_id) } });
      break;
    case 'template_instantiated':
      get().ws?.send({ type: 'list_servers' });
      break;
  }
}
