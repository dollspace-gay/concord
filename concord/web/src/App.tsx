import { useEffect } from 'react';
import { useAuthStore } from './stores/authStore';
import { useChatStore } from './stores/chatStore';
import { LoginPage } from './components/auth/LoginPage';
import { AppLayout } from './components/layout/AppLayout';

let renderCount = 0;

function App() {
  renderCount++;
  const user = useAuthStore((s) => s.user);
  const loading = useAuthStore((s) => s.loading);
  const checkAuth = useAuthStore((s) => s.checkAuth);
  const connect = useChatStore((s) => s.connect);
  const disconnect = useChatStore((s) => s.disconnect);

  console.log(`[App] render #${renderCount} loading=${loading} user=${user?.username ?? 'null'}`);

  // Check auth on mount
  useEffect(() => {
    console.log('[App] checkAuth effect firing');
    checkAuth();
  }, [checkAuth]);

  // Connect WebSocket when authenticated
  useEffect(() => {
    if (user) {
      console.log('[App] connect effect firing for user:', user.username);
      connect(user.username);
      return () => {
        console.log('[App] disconnect cleanup');
        disconnect();
      };
    }
  }, [user, connect, disconnect]);

  if (loading) {
    console.log('[App] rendering: loading spinner');
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-text-muted">Loading...</div>
      </div>
    );
  }

  if (!user) {
    console.log('[App] rendering: LoginPage');
    return <LoginPage />;
  }

  console.log('[App] rendering: AppLayout');
  return <AppLayout />;
}

export default App;
