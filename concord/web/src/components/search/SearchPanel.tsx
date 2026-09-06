import { useState, useCallback } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';

const PAGE_SIZE = 25;

function validateSearchQuery(query: string): string | null {
  if (query.length > 1024 || [...query].some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)) return 'Search query is too long or contains a control character.';
  let quoted = false;
  let escaped = false;
  for (const character of query) {
    if (escaped) escaped = false;
    else if (character === '\\' && quoted) escaped = true;
    else if (character === '"') quoted = !quoted;
  }
  if (quoted || escaped) return 'A quoted search phrase is not closed.';
  const seen = new Set<string>();
  const seenHas = new Set<string>();
  for (const match of query.matchAll(/(?:^|\s)(from|in|has|before|after):([^\s]*)/gi)) {
    const operator = match[1].toLowerCase();
    const value = match[2];
    if (!value || (operator !== 'has' && seen.has(operator)) || (operator === 'has' && (!/^(attachment|link)$/i.test(value) || seenHas.has(value.toLowerCase())))) return `Invalid ${operator}: filter.`;
    if ((operator === 'before' || operator === 'after') && !/^\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z)?$/.test(value)) return `Invalid ${operator}: date. Use YYYY-MM-DD or a UTC timestamp.`;
    if ((operator === 'before' || operator === 'after') && Number.isNaN(Date.parse(value))) return `Invalid ${operator}: date.`;
    if (operator === 'has') seenHas.add(value.toLowerCase());
    else seen.add(operator);
  }
  return null;
}

export function SearchPanel() {
  const show = useUiStore((s) => s.showSearch);
  const setShow = useUiStore((s) => s.setShowSearch);
  const activeServer = useUiStore((s) => s.activeServer);
  const setActiveChannel = useUiStore((s) => s.setActiveChannel);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const searchMessages = useChatStore((s) => s.searchMessages);
  const clearSearch = useChatStore((s) => s.clearSearch);
  const results = useChatStore((s) => s.searchResults);
  const totalCount = useChatStore((s) => s.searchTotalCount);
  const offset = useChatStore((s) => s.searchOffset);
  const nextContinuation = useChatStore((s) => s.searchNextContinuation);
  const continuationTokens = useChatStore((s) => s.searchContinuationTokens);
  const activeRequestId = useChatStore((s) => s.activeSearchRequestId);
  const searchRestarted = useChatStore((s) => s.searchRestarted);
  const setJumpToMessageId = useUiStore((s) => s.setJumpToMessageId);

  const [query, setQuery] = useState('');
  const [channelFilter, setChannelFilter] = useState('');
  const [validationError, setValidationError] = useState<string | null>(null);

  const handleSearch = useCallback((nextOffset = 0, continuation?: string) => {
    if (!query.trim() || !activeServer) return;
    const error = validateSearchQuery(query.trim());
    setValidationError(error);
    if (error) return;
    searchMessages(activeServer, query.trim(), channelFilter || undefined, PAGE_SIZE, nextOffset, continuation);
  }, [query, channelFilter, activeServer, searchMessages]);

  if (!show) return null;

  return (
    <div className="flex h-full w-80 flex-col border-l border-border bg-bg-secondary">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-semibold text-text-primary">Search</span>
        <button onClick={() => { setShow(false); clearSearch(); }} className="text-text-muted hover:text-text-primary">
          <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div className="space-y-2 p-3">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleSearch(0)}
          placeholder="Search messages..."
          className="w-full rounded border border-border bg-bg-primary px-2 py-1.5 text-sm text-text-primary outline-none placeholder:text-text-muted focus:border-accent"
          autoFocus
        />
        {validationError && <p role="alert" className="text-xs text-red-400">{validationError}</p>}
        <p className="text-xs leading-4 text-text-muted">
          Filters: <code>from:name</code>, <code>in:#channel</code>,{' '}
          <code>has:attachment</code>, <code>has:link</code>,{' '}
          <code>before:YYYY-MM-DD</code>, <code>after:YYYY-MM-DD</code> (dates excluded, UTC). Use quotes for phrases.
        </p>
        <input
          type="text"
          value={channelFilter}
          onChange={(e) => setChannelFilter(e.target.value)}
          placeholder="Filter by channel (optional)"
          className="w-full rounded border border-border bg-bg-primary px-2 py-1.5 text-sm text-text-primary outline-none placeholder:text-text-muted focus:border-accent"
        />
        <button
          onClick={() => handleSearch(0)}
          disabled={!query.trim()}
          className="w-full rounded bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
        >
          Search
        </button>
      </div>

      {results && (
        <div className="flex-1 overflow-y-auto">
          <div className="px-3 py-1 text-xs text-text-muted">
            {totalCount} result{totalCount !== 1 ? 's' : ''}
          </div>
          {searchRestarted && (
            <p role="status" className="px-3 py-1 text-xs text-amber-300">
              Results refreshed because your channel access changed.
            </p>
          )}
          {results.map((msg) => (
            <button
              key={msg.id}
              onClick={() => {
                if (activeServer && msg.channel_name) {
                  setJumpToMessageId(msg.id);
                  setActiveChannel(msg.channel_name);
                  joinChannel(activeServer, msg.channel_name);
                  setShow(false);
                }
              }}
              className="w-full border-b border-border px-3 py-2 text-left transition-colors hover:bg-bg-hover"
            >
              <div className="flex items-baseline gap-1">
                <span className="text-sm font-medium text-text-primary">{msg.from}</span>
                <span className="text-xs text-text-muted">in #{msg.channel_name}</span>
                <span className="ml-auto text-xs text-text-muted">
                  {new Date(msg.timestamp).toLocaleDateString()}
                </span>
              </div>
              <div className="mt-0.5 text-sm text-text-secondary line-clamp-2">{msg.content}</div>
            </button>
          ))}
          {totalCount > PAGE_SIZE && (
            <div className="flex items-center justify-between gap-2 p-3 text-xs text-text-secondary">
              <button
                disabled={offset === 0 || activeRequestId !== null}
                onClick={() => {
                  const previousOffset = Math.max(0, offset - PAGE_SIZE);
                  handleSearch(previousOffset, continuationTokens[String(previousOffset)]);
                }}
                className="rounded bg-bg-tertiary px-2 py-1 disabled:opacity-40"
              >Previous</button>
              <span>{offset + 1}–{Math.min(offset + PAGE_SIZE, totalCount)} of {totalCount}</span>
              <button
                disabled={!nextContinuation || activeRequestId !== null}
                onClick={() => handleSearch(offset + PAGE_SIZE, nextContinuation ?? undefined)}
                className="rounded bg-bg-tertiary px-2 py-1 disabled:opacity-40"
              >Next</button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
