import type { ChatState, ChatStoreContext } from './types';

export function createTemplateActions({ get }: ChatStoreContext): Pick<ChatState, 'createTemplate' | 'listTemplates' | 'deleteTemplate' | 'instantiateTemplate'> {
  return {
    createTemplate: (serverId, name, description) => {
      return get().runLifecycleCommand(
        { type: 'create_template', server_id: serverId, name, description },
        `community:${serverId}:template:create`,
      );
    },

    listTemplates: (serverId) => {
      get().ws?.send({ type: 'list_templates', server_id: serverId });
    },

    deleteTemplate: (serverId, templateId) => {
      return get().runLifecycleCommand(
        { type: 'delete_template', server_id: serverId, template_id: templateId },
        `community:${serverId}:template:${templateId}`,
      );
    },

    instantiateTemplate: (templateId, serverName) => {
      return get().runLifecycleCommand(
        { type: 'instantiate_template', template_id: templateId, server_name: serverName },
        `community:template:${templateId}:instantiate`,
      );
    }
  };
}
