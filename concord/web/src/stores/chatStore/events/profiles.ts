import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { PresenceInfo } from '../../../api/types';
import type { ChatStoreContext } from '../types';

export function handleProfilesEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'presence_update' | 'presence_list' | 'user_profile' | 'own_presence' | 'server_nickname_update' }>) {
  switch (event.type) {
    case 'presence_update': {
      const { server_id, presence } = event;
      set((s) => ({
        ...(presence.user_id === s.activeAccountId ? { ownPresenceStatus: presence.status } : {}),
        presences: {
          ...s.presences,
          [server_id]: {
            ...s.presences[server_id],
            [presence.user_id]: presence,
          },
        },
      }));
      break;
    }
    case 'presence_list': {
      const { server_id, presences: list } = event;
      const map: Record<string, PresenceInfo> = {};
      for (const p of list) {
        map[p.user_id] = p;
      }
      set((s) => ({
        ...(list.find((presence) => presence.user_id === s.activeAccountId)
          ? { ownPresenceStatus: list.find((presence) => presence.user_id === s.activeAccountId)!.status }
          : {}),
        presences: {
          ...s.presences,
          [server_id]: map,
        },
      }));
      break;
    }
    case 'user_profile': {
      set((s) => ({
        userProfiles: {
          ...s.userProfiles,
          [event.profile.user_id]: event.profile,
        },
      }));
      break;
    }
    case 'own_presence': {
      set({
        ownPresenceStatus: event.effective_status,
        ownRequestedStatus: event.requested_status,
        ownCustomStatus: event.custom_status ?? null,
        ownStatusEmoji: event.status_emoji ?? null,
      });
      break;
    }
    case 'server_nickname_update': {
      set((state) => ({
        members: Object.fromEntries(Object.entries(state.members).map(([key, members]) => [
          key,
          key.startsWith(`${event.server_id}:`)
            ? members.map((member) => member.user_id === event.user_id
              ? {
                ...member,
                nickname: event.display_name,
                server_avatar_url: event.server_avatar_url ?? undefined,
              }
              : member)
            : members,
        ])),
      }));
      break;
    }
  }
}
