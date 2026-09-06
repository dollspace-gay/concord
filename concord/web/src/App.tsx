import { useEffect, useRef } from 'react';
import { useAuthStore } from './stores/authStore';
import { useChatStore } from './stores/chatStore';
import { useUiStore } from './stores/uiStore';
import { LoginPage } from './components/auth/LoginPage';
import { AppLayout } from './components/layout/AppLayout';
import * as api from './api/client';

function App() {
  const user = useAuthStore((s) => s.user);
  const loading = useAuthStore((s) => s.loading);
  const checkAuth = useAuthStore((s) => s.checkAuth);
  const authError = useAuthStore((s) => s.error);
  const connect = useChatStore((s) => s.connect);
  const disconnect = useChatStore((s) => s.disconnect);
  const connected = useChatStore((s) => s.connected);
  const servers = useChatStore((s) => s.servers);
  const listChannels = useChatStore((s) => s.listChannels);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const getMembers = useChatStore((s) => s.getMembers);
  const fetchHistory = useChatStore((s) => s.fetchHistory);
  const getUnreadCounts = useChatStore((s) => s.getUnreadCounts);
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const activeDirectConversation = useUiStore((s) => s.activeDirectConversation);
  const setActiveServer = useUiStore((s) => s.setActiveServer);
  const hydrateServerFolders = useUiStore((s) => s.hydrateServerFolders);
  const hasConnectedOnce = useRef(false);

  // Check auth on mount
  useEffect(() => {
    checkAuth();
  }, [checkAuth]);

  // Connect WebSocket when authenticated
  useEffect(() => {
    if (user) {
      connect(user.username, user.id);
      return () => {
        disconnect();
      };
    }
  }, [user, connect, disconnect]);

  useEffect(() => {
    let current = true;
    hydrateServerFolders(user?.id ?? null);
    if (!user) return () => { current = false; };
    const revision = useUiStore.getState().folderRevision;
    void api.listServerFolders().then((folders) => {
      if (!current || useAuthStore.getState().user?.id !== user.id || useUiStore.getState().folderRevision !== revision) return;
      hydrateServerFolders(user.id, folders.map((folder) => ({
        id: folder.id,
        name: folder.name,
        color: folder.color ?? undefined,
        serverIds: folder.server_ids,
        collapsed: folder.collapsed ?? false,
      })));
    }).catch(() => {
      // The server list remains usable when folder preferences cannot be loaded.
    });
    return () => { current = false; };
  }, [user, hydrateServerFolders]);

  // Re-bootstrap active server/channel state on reconnect
  useEffect(() => {
    if (!connected) return;
    if (!hasConnectedOnce.current) {
      // First connection — let normal flow handle it
      hasConnectedOnce.current = true;
      return;
    }
    // This is a reconnect — re-fetch channels, unread counts, and rejoin active channel
    if (activeServer) {
      listChannels(activeServer);
      getUnreadCounts(activeServer);
      if (activeChannel) {
        joinChannel(activeServer, activeChannel);
        getMembers(activeServer, activeChannel);
        fetchHistory(activeServer, activeChannel);
      }
    }
  }, [connected, activeServer, activeChannel, listChannels, getUnreadCounts, joinChannel, getMembers, fetchHistory]);

  // Auto-select first server when server list arrives and no server is active
  useEffect(() => {
    if (servers.length > 0 && !activeServer && activeDirectConversation === null) {
      setActiveServer(servers[0].id);
      listChannels(servers[0].id);
    }
  }, [servers, activeServer, activeDirectConversation, setActiveServer, listChannels]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-text-muted">Loading...</div>
      </div>
    );
  }

  if (!user) {
    return <LoginPage />;
  }

  return (
    <div className="relative h-full">
      {authError && (
        <div role="alert" className="absolute inset-x-3 top-3 z-50 flex items-center gap-3 rounded border border-red-500/50 bg-bg-secondary px-3 py-2 text-sm text-text-secondary shadow-lg">
          <span className="min-w-0 flex-1">{authError}</span>
          <button onClick={() => void checkAuth()} className="shrink-0 font-medium text-accent">Retry sign-in check</button>
        </div>
      )}
      <AppLayout />
    </div>
  );
}

export default App;
