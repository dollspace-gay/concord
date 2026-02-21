import { useEffect, useRef } from 'react';
import { useAuthStore } from './stores/authStore';
import { useChatStore } from './stores/chatStore';
import { useUiStore } from './stores/uiStore';
import { LoginPage } from './components/auth/LoginPage';
import { AppLayout } from './components/layout/AppLayout';

function App() {
  const user = useAuthStore((s) => s.user);
  const loading = useAuthStore((s) => s.loading);
  const checkAuth = useAuthStore((s) => s.checkAuth);
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
  const setActiveServer = useUiStore((s) => s.setActiveServer);
  const hasConnectedOnce = useRef(false);

  // Check auth on mount
  useEffect(() => {
    checkAuth();
  }, [checkAuth]);

  // Connect WebSocket when authenticated
  useEffect(() => {
    if (user) {
      connect(user.username);
      return () => {
        disconnect();
      };
    }
  }, [user, connect, disconnect]);

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
    if (servers.length > 0 && !activeServer) {
      setActiveServer(servers[0].id);
      listChannels(servers[0].id);
    }
  }, [servers, activeServer, setActiveServer, listChannels]);

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

  return <AppLayout />;
}

export default App;
