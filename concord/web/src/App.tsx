import { useEffect } from 'react';
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
  const servers = useChatStore((s) => s.servers);
  const listChannels = useChatStore((s) => s.listChannels);
  const activeServer = useUiStore((s) => s.activeServer);
  const setActiveServer = useUiStore((s) => s.setActiveServer);

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
