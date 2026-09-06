import { useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { CreateServerModal } from './CreateServerModal';
import { ExternalImage } from '../ExternalImage';

export function ServerList() {
  const servers = useChatStore((s) => s.servers);
  const activeServer = useUiStore((s) => s.activeServer);
  const setActiveServer = useUiStore((s) => s.setActiveServer);
  const listChannels = useChatStore((s) => s.listChannels);
  const activeDirectConversation = useUiStore((s) => s.activeDirectConversation);
  const setActiveDirectConversation = useUiStore((s) => s.setActiveDirectConversation);
  const folders = useUiStore((s) => s.serverFolders);
  const addFolder = useUiStore((s) => s.addServerFolder);
  const removeFolder = useUiStore((s) => s.removeServerFolder);
  const toggleFolder = useUiStore((s) => s.toggleServerFolder);
  const folderSyncStatus = useUiStore((s) => s.folderSyncStatus);
  const folderSyncError = useUiStore((s) => s.folderSyncError);
  const retryFolderSync = useUiStore((s) => s.retryServerFolderSync);
  const [showCreate, setShowCreate] = useState(false);
  const [showFolderForm, setShowFolderForm] = useState(false);
  const [folderName, setFolderName] = useState('');
  const [folderServers, setFolderServers] = useState<string[]>([]);

  const handleSelect = (serverId: string) => {
    setActiveServer(serverId);
    listChannels(serverId);
  };

  const groupedServerIds = new Set(folders.flatMap((folder) => folder.serverIds));
  const serverButton = (server: (typeof servers)[number]) => {
    const isActive = activeServer === server.id;
    const initial = server.name[0]?.toUpperCase() || '?';
    return (
      <div key={server.id} className="relative flex items-center justify-center">
        <div className={`absolute left-0 w-1 rounded-r-full bg-text-primary transition-all ${isActive ? 'h-10' : 'h-0 group-hover:h-5'}`} />
        <div
          role="button"
          tabIndex={0}
          onClick={() => handleSelect(server.id)}
          onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); handleSelect(server.id); } }}
          title={server.name}
          aria-label={server.name}
          className={`flex h-12 w-12 items-center justify-center transition-all ${isActive ? 'rounded-2xl bg-bg-accent text-white' : 'rounded-3xl bg-bg-tertiary text-text-muted hover:rounded-2xl hover:bg-bg-accent hover:text-white'}`}
        >
          {server.icon_url ? <ExternalImage src={server.icon_url} alt="" label={`${server.name} icon`} className="h-12 w-12 rounded-[inherit] object-cover" privacyScopeKey={`server:${server.id}:icon`} /> : <span className="text-sm font-semibold">{initial}</span>}
        </div>
      </div>
    );
  };

  return (
    <div className="flex h-full w-14 flex-col items-center gap-2 overflow-y-auto bg-bg-primary py-3 md:w-[72px]">
      <button
        onClick={() => setActiveDirectConversation(activeDirectConversation ?? '')}
        title="Direct messages"
        aria-label="Direct messages"
        className={`flex h-12 w-12 items-center justify-center rounded-3xl transition-all hover:rounded-2xl ${
          activeServer === null ? 'bg-bg-accent text-white' : 'bg-bg-tertiary text-text-muted hover:bg-bg-accent hover:text-white'
        }`}
      >
        <span className="text-xl">@</span>
      </button>

      <div className="mx-auto h-0.5 w-8 rounded bg-border-primary" />
      {folders.map((folder) => (
        <div key={folder.id} className="flex w-full flex-col items-center gap-1">
          <div className="flex items-center gap-1">
            <button
              onClick={() => toggleFolder(folder.id)}
              aria-expanded={!folder.collapsed}
              aria-label={`${folder.collapsed ? 'Expand' : 'Collapse'} ${folder.name} folder`}
              title={folder.name}
              className="h-6 max-w-12 truncate rounded px-1 text-xs text-text-muted hover:bg-bg-tertiary"
              style={folder.color ? { borderBottom: `2px solid ${folder.color}` } : undefined}
            >
              {folder.name}
            </button>
            <button onClick={() => removeFolder(folder.id)} aria-label={`Delete ${folder.name} folder`} className="text-xs text-text-muted hover:text-red-400">×</button>
          </div>
          {!folder.collapsed && folder.serverIds.map((id) => servers.find((server) => server.id === id)).filter((server) => server !== undefined).map(serverButton)}
        </div>
      ))}
      {servers.filter((server) => !groupedServerIds.has(server.id)).map(serverButton)}

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

      <button
        onClick={() => setShowFolderForm((shown) => !shown)}
        title="Create server folder"
        aria-label="Create server folder"
        className="flex h-9 w-9 items-center justify-center rounded-full bg-bg-tertiary text-text-muted hover:bg-bg-accent hover:text-white"
      >
        ▤
      </button>

      {showFolderForm && (
        <form
          aria-label="New server folder"
          className="fixed left-20 top-3 z-50 w-64 rounded-md border border-border-primary bg-bg-secondary p-3 shadow-xl"
          onSubmit={(event) => {
            event.preventDefault();
            if (!folderName.trim() || folderServers.length === 0) return;
            addFolder(folderName.trim(), folderServers);
            setFolderName('');
            setFolderServers([]);
            setShowFolderForm(false);
          }}
        >
          <label className="block text-xs text-text-secondary">Folder name<input autoFocus value={folderName} onChange={(event) => setFolderName(event.target.value)} maxLength={100} className="mt-1 w-full rounded bg-bg-input px-2 py-1 text-text-primary" /></label>
          <fieldset className="mt-2 space-y-1"><legend className="text-xs text-text-secondary">Servers</legend>{servers.map((server) => <label key={server.id} className="flex gap-2 text-sm text-text-primary"><input type="checkbox" checked={folderServers.includes(server.id)} onChange={(event) => setFolderServers((selected) => event.target.checked ? [...selected, server.id] : selected.filter((id) => id !== server.id))} />{server.name}</label>)}</fieldset>
          <div className="mt-3 flex justify-end gap-2"><button type="button" onClick={() => setShowFolderForm(false)} className="text-sm text-text-muted">Cancel</button><button type="submit" className="rounded bg-bg-accent px-2 py-1 text-sm text-white">Create folder</button></div>
        </form>
      )}

      {folderSyncStatus === 'saving' && <span className="sr-only" role="status">Saving server folders</span>}
      {folderSyncStatus === 'error' && (
        <div role="alert" className="fixed bottom-3 left-20 z-50 flex items-center gap-2 rounded bg-bg-secondary p-2 text-xs text-red-300 shadow-xl">
          <span>Folder changes could not be saved. {folderSyncError}</span>
          <button onClick={retryFolderSync} className="rounded bg-bg-accent px-2 py-1 text-white">Retry</button>
        </div>
      )}

      {showCreate && <CreateServerModal onClose={() => setShowCreate(false)} />}
    </div>
  );
}
