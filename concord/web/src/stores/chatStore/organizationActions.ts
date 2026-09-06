import type { ChatState, ChatStoreContext } from './types';

export function createOrganizationActions({ get }: ChatStoreContext): Pick<ChatState, 'listRoles' | 'createRole' | 'updateRole' | 'deleteRole' | 'assignRole' | 'removeRole' | 'listChannelPermissionOverrides' | 'setChannelPermissionOverride' | 'deleteChannelPermissionOverride' | 'listCategories' | 'createCategory' | 'updateCategory' | 'deleteCategory' | 'reorderChannels'> {
  return {
    listRoles: (serverId) => {
      get().ws?.send({ type: 'list_roles', server_id: serverId });
    },

    createRole: (serverId, name, color, permissions) => {
      get().ws?.send({ type: 'create_role', server_id: serverId, name, color, permissions });
    },

    updateRole: (serverId, roleId, updates) => {
      const role = (get().roles[serverId] || []).find((candidate) => candidate.id === roleId);
      if (!role) return;
      get().ws?.send({
        type: 'update_role',
        server_id: serverId,
        role_id: roleId,
        name: updates.name ?? role.name,
        color: updates.color ?? role.color,
        permissions: updates.permissions ?? role.permissions,
      });
    },

    deleteRole: (serverId, roleId) => {
      get().ws?.send({ type: 'delete_role', server_id: serverId, role_id: roleId });
    },

    assignRole: (serverId, userId, roleId) => {
      get().ws?.send({ type: 'assign_role', server_id: serverId, user_id: userId, role_id: roleId });
    },

    removeRole: (serverId, userId, roleId) => {
      get().ws?.send({ type: 'remove_role', server_id: serverId, user_id: userId, role_id: roleId });
    },

    listChannelPermissionOverrides: (serverId, channelId) => {
      get().ws?.send({ type: 'list_channel_permission_overrides', server_id: serverId, channel_id: channelId });
    },

    setChannelPermissionOverride: (serverId, channelId, targetType, targetId, allowBits, denyBits) => {
      get().ws?.send({
        type: 'set_channel_permission_override',
        server_id: serverId,
        channel_id: channelId,
        target_type: targetType,
        target_id: targetId,
        allow_bits: allowBits,
        deny_bits: denyBits,
      });
    },

    deleteChannelPermissionOverride: (serverId, channelId, targetType, targetId) => {
      get().ws?.send({
        type: 'delete_channel_permission_override',
        server_id: serverId,
        channel_id: channelId,
        target_type: targetType,
        target_id: targetId,
      });
    },

    listCategories: (serverId) => {
      get().ws?.send({ type: 'list_categories', server_id: serverId });
    },

    createCategory: (serverId, name) => {
      get().ws?.send({ type: 'create_category', server_id: serverId, name });
    },

    updateCategory: (serverId, categoryId, updates) => {
      const category = (get().categories[serverId] || []).find((candidate) => candidate.id === categoryId);
      if (!category) return;
      get().ws?.send({ type: 'update_category', server_id: serverId, category_id: categoryId, name: updates.name ?? category.name });
    },

    deleteCategory: (serverId, categoryId) => {
      get().ws?.send({ type: 'delete_category', server_id: serverId, category_id: categoryId });
    },

    reorderChannels: (serverId, channels) => {
      get().ws?.send({ type: 'reorder_channels', server_id: serverId, channels });
    }
  };
}
