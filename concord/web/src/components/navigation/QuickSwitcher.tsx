import React, { useState, useEffect, useMemo, useRef } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';

export function QuickSwitcher() {
  const show = useUiStore((s) => s.showQuickSwitcher);
  const setShow = useUiStore((s) => s.setShowQuickSwitcher);
  const servers = useChatStore((s) => s.servers);
  const channels = useChatStore((s) => s.channels);
  const setActiveServer = useUiStore((s) => s.setActiveServer);
  const setActiveChannel = useUiStore((s) => s.setActiveChannel);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const inputRef = useRef<HTMLInputElement>(null);

  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);

  // Global Ctrl+K handler
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault();
        setShow(!show);
        setQuery('');
        setSelectedIndex(0);
      }
      if (e.key === 'Escape' && show) {
        setShow(false);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [show, setShow]);

  useEffect(() => {
    if (show) inputRef.current?.focus();
  }, [show]);

  const results = useMemo(() => {
    if (!query.trim()) return [];
    const q = query.toLowerCase();
    const items: { type: string; serverId: string; serverName: string; channelName?: string }[] = [];

    for (const server of servers) {
      if (server.name.toLowerCase().includes(q)) {
        items.push({ type: 'server', serverId: server.id, serverName: server.name });
      }
      for (const ch of channels[server.id] || []) {
        if (ch.name.toLowerCase().includes(q)) {
          items.push({ type: 'channel', serverId: server.id, serverName: server.name, channelName: ch.name });
        }
      }
    }
    return items.slice(0, 10);
  }, [query, servers, channels]);

  // Reset selection when query changes (render-time adjustment per React docs)
  const [prevQuery, setPrevQuery] = useState(query);
  if (query !== prevQuery) {
    setPrevQuery(query);
    setSelectedIndex(0);
  }

  if (!show) return null;

  const handleSelect = (item: (typeof results)[0]) => {
    setActiveServer(item.serverId);
    if (item.channelName) {
      setActiveChannel(item.channelName);
      joinChannel(item.serverId, item.channelName);
    }
    setShow(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, results.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter' && results[selectedIndex]) {
      handleSelect(results[selectedIndex]);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] bg-black/60" onClick={() => setShow(false)}>
      <div className="w-[500px] overflow-hidden rounded-lg bg-bg-primary shadow-xl" onClick={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Where would you like to go?"
          className="w-full border-b border-border bg-bg-primary px-4 py-3 text-lg text-text-primary outline-none placeholder:text-text-muted"
        />
        {results.length > 0 && (
          <div className="max-h-[300px] overflow-y-auto py-1">
            {results.map((item, i) => (
              <button
                key={`${item.serverId}-${item.channelName || 'srv'}`}
                onClick={() => handleSelect(item)}
                className={`flex w-full items-center gap-2 px-4 py-2 text-left text-sm transition-colors ${
                  i === selectedIndex ? 'bg-bg-hover text-text-primary' : 'text-text-secondary hover:bg-bg-hover'
                }`}
              >
                {item.type === 'channel' ? (
                  <>
                    <span className="text-text-muted">#</span>
                    <span>{item.channelName}</span>
                    <span className="ml-auto text-xs text-text-muted">{item.serverName}</span>
                  </>
                ) : (
                  <>
                    <span className="text-text-muted">●</span>
                    <span>{item.serverName}</span>
                  </>
                )}
              </button>
            ))}
          </div>
        )}
        <div className="border-t border-border px-4 py-2 text-xs text-text-muted">
          Tip: Start typing to search servers and channels
        </div>
      </div>
    </div>
  );
}
