import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { ChatStoreContext } from '../types';

export function handleSearchEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'notification_settings' | 'search_results' }>) {
  switch (event.type) {
    case 'notification_settings': {
      set((state) => ({
        notificationSettings: { ...state.notificationSettings, [event.server_id]: event.settings },
      }));
      break;
    }
    case 'search_results': {
      const activeRequestId = get().activeSearchRequestId;
      if (activeRequestId !== null
        ? event.request_id !== activeRequestId
        : event.request_id !== undefined && event.request_id !== null) break;
      set((state) => ({
        searchResults: event.results.filter((result) => !state.deletedMessageIds[result.id]),
        searchQuery: event.query,
        searchTotalCount: event.total_count,
        searchOffset: event.offset,
        searchNextContinuation: event.next_continuation ?? null,
        searchContinuationTokens: event.restarted
          ? (event.next_continuation ? { [String(event.results.length)]: event.next_continuation } : {})
          : (event.next_continuation
            ? { ...state.searchContinuationTokens, [String(event.offset + event.results.length)]: event.next_continuation }
            : state.searchContinuationTokens),
        activeSearchRequestId: null,
        searchRestarted: event.restarted,
      }));
      break;
    }
  }
}
