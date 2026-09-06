import { create } from 'zustand';
import type { ChannelInfo, HistoryMessage, MemberInfo, ServerInfo } from '../api/types';
import type { DirectConversationInfo } from '../api/generated/contract';

interface EntityState {
  servers: ServerInfo[];
  channels: Record<string, ChannelInfo[]>;
  messages: Record<string, HistoryMessage[]>;
  members: Record<string, MemberInfo[]>;
  directConversations: DirectConversationInfo[];
  entityVersions: Record<string, number>;
  deletedMessageIds: Record<string, true>;
  replace: (state: Partial<Omit<EntityState, 'replace'>>) => void;
}

export const useEntityStore = create<EntityState>((set) => ({
  servers: [],
  channels: {},
  messages: {},
  members: {},
  directConversations: [],
  entityVersions: {},
  deletedMessageIds: {},
  replace: (state) => set(state),
}));
