import { useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { CreateServerModal } from './CreateServerModal';

export function ServerList() {
  const servers = useChatStore((s) => s.servers);
  const activeServer = useUiStore((s) => s.activeServer);
  const setActiveServer = useUiStore((s) => s.setActiveServer);
  const listChannels = useChatStore((s) => s.listChannels);
  const [showCreate, setShowCreate] = useState(false);

  const handleSelect = (serverId: string) => {
    setActiveServer(serverId);
    listChannels(serverId);
  };

  return (
    <div className="flex h-full w-[72px] flex-col items-center gap-2 bg-bg-primary py-3 overflow-y-auto">
      {servers.map((server) => {
        const isActive = activeServer === server.id;
        const initial = server.name[0]?.toUpperCase() || '?';

        return (
          <div key={server.id} className="relative flex items-center justify-center">
            {/* Active indicator pill */}
            <div
              className={`absolute left-0 w-1 rounded-r-full bg-text-primary transition-all ${
                isActive ? 'h-10' : 'h-0 group-hover:h-5'
              }`}
            />

            <button
              onClick={() => handleSelect(server.id)}
              title={server.name}
              className={`flex h-12 w-12 items-center justify-center transition-all ${
                isActive
                  ? 'rounded-2xl bg-bg-accent text-white'
                  : 'rounded-3xl bg-bg-tertiary text-text-muted hover:rounded-2xl hover:bg-bg-accent hover:text-white'
              }`}
            >
              {server.icon_url ? (
                <img
                  src={server.icon_url}
                  alt={server.name}
                  className="h-12 w-12 rounded-[inherit] object-cover"
                />
              ) : (
                <span className="text-sm font-semibold">{initial}</span>
              )}
            </button>
          </div>
        );
      })}

      {/* Separator */}
      <div className="mx-auto h-0.5 w-8 rounded bg-border-primary" />

      {/* Add server button */}
      <button
        onClick={() => setShowCreate(true)}
        title="Create a server"
        className="flex h-12 w-12 items-center justify-center rounded-3xl bg-bg-tertiary text-green-500 transition-all hover:rounded-2xl hover:bg-green-500 hover:text-white"
      >
        <svg className="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
        </svg>
      </button>

      {showCreate && <CreateServerModal onClose={() => setShowCreate(false)} />}
    </div>
  );
}
