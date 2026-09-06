import type { ChatState, ChatStoreContext } from './types';

export function createSearchActions({ set, get }: ChatStoreContext): Pick<ChatState, 'searchMessages' | 'clearSearch'> {
  return {
    searchMessages: (serverId, query, channel, limit, offset = 0, continuation) => {
      const requestId = crypto.randomUUID();
      if (offset === 0 && !continuation) {
        set({ searchContinuationTokens: {}, searchNextContinuation: null, searchRestarted: false });
      }
      set({ activeSearchRequestId: requestId });
      get().ws?.send({
        type: 'search_messages', request_id: requestId, server_id: serverId, query, channel,
        limit, offset, continuation,
      });
    },

    clearSearch: () => {
      set({
        searchResults: null, searchQuery: '', searchTotalCount: 0, searchOffset: 0,
        searchNextContinuation: null, searchContinuationTokens: {}, activeSearchRequestId: null,
        searchRestarted: false,
      });
    }
  };
}
