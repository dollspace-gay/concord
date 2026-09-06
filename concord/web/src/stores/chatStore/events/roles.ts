import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { ChatStoreContext } from '../types';

export function handleRolesEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'role_list' | 'role_update' | 'role_delete' | 'member_role_update' | 'channel_permission_override_list' }>) {
  switch (event.type) {
    case 'role_list': {
      set((s) => {
        const currentVersion = s.roleProjectionVersions[event.server_id] ?? -1;
        if (event.version < currentVersion) return s;
        const validRoleIds = new Set(event.roles.map((role) => role.id));
        const currentAssignments = s.memberRoles[event.server_id] ?? {};
        const memberRoles = event.member_roles
          ? Object.fromEntries(event.member_roles.map((member) => [member.user_id, member.role_ids]))
          : Object.fromEntries(Object.entries(currentAssignments).map(([userId, roleIds]) => [
            userId,
            roleIds.filter((roleId) => validRoleIds.has(roleId)),
          ]));
        return {
          roles: { ...s.roles, [event.server_id]: event.roles },
          memberRoles: { ...s.memberRoles, [event.server_id]: memberRoles },
          roleProjectionVersions: { ...s.roleProjectionVersions, [event.server_id]: event.version },
        };
      });
      break;
    }
    case 'role_update': {
      set((s) => {
        const current = s.roles[event.server_id] || [];
        const idx = current.findIndex((r) => r.id === event.role.id);
        const updated = idx >= 0
          ? current.map((r) => (r.id === event.role.id ? event.role : r))
          : [...current, event.role];
        return { roles: { ...s.roles, [event.server_id]: updated } };
      });
      break;
    }
    case 'role_delete': {
      set((s) => ({
        roles: {
          ...s.roles,
          [event.server_id]: (s.roles[event.server_id] || []).filter((r) => r.id !== event.role_id),
        },
        memberRoles: {
          ...s.memberRoles,
          [event.server_id]: Object.fromEntries(Object.entries(s.memberRoles[event.server_id] ?? {}).map(([userId, roleIds]) => [userId, roleIds.filter((roleId) => roleId !== event.role_id)])),
        },
      }));
      break;
    }
    case 'member_role_update': {
      set((state) => {
        const currentVersion = state.roleProjectionVersions[event.server_id] ?? -1;
        if (event.version < currentVersion) return state;
        return {
          memberRoles: {
            ...state.memberRoles,
            [event.server_id]: {
              ...(state.memberRoles[event.server_id] ?? {}),
              [event.user_id]: event.role_ids,
            },
          },
          roleProjectionVersions: {
            ...state.roleProjectionVersions,
            [event.server_id]: event.version,
          },
        };
      });
      break;
    }
    case 'channel_permission_override_list': {
      set((state) => ({
        channelPermissionOverrides: {
          ...state.channelPermissionOverrides,
          [event.channel_id]: event.overrides,
        },
      }));
      break;
    }
  }
}
