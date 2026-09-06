import { useState } from 'react';
import { useAuthStore } from '../../stores/authStore';

export function LoginPage() {
  const authError = useAuthStore((state) => state.error);
  const checkAuth = useAuthStore((state) => state.checkAuth);
  const [handle, setHandle] = useState('');
  const [loading, setLoading] = useState(false);

  const handleLogin = () => {
    const trimmed = handle.trim();
    if (!trimmed) return;
    setLoading(true);
    window.location.href = `/api/auth/atproto/login?handle=${encodeURIComponent(trimmed)}`;
  };

  return (
    <div className="flex h-full items-center justify-center bg-bg-primary">
      <div className="w-full max-w-md rounded-lg bg-bg-secondary p-8">
        <div className="mb-8 text-center">
          <h1 className="mb-2 text-2xl font-bold text-text-primary">Welcome to Concord</h1>
          <p className="text-text-muted">Sign in with your Bluesky account</p>
        </div>

        <div className="space-y-3">
          {authError && (
            <div role="alert" className="flex items-center gap-3 rounded-md bg-red-950/50 px-3 py-2 text-sm text-red-300">
              <span className="min-w-0 flex-1">{authError}</span>
              <button type="button" onClick={() => void checkAuth()} className="shrink-0 font-medium text-accent">
                Retry sign-in check
              </button>
            </div>
          )}
          <div className="flex gap-2">
            <input
              type="text"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleLogin()}
              placeholder="handle.bsky.social"
              className="flex-1 rounded-md border border-border bg-bg-primary px-4 py-3 text-text-primary placeholder-text-muted focus:border-accent-primary focus:outline-none"
              disabled={loading}
            />
            <button
              onClick={handleLogin}
              disabled={!handle.trim() || loading}
              className="flex items-center gap-2 rounded-md bg-[#0085ff] px-4 py-3 font-medium text-white transition-colors hover:bg-[#0070dd] disabled:opacity-50"
            >
              <svg className="h-5 w-5" viewBox="0 0 568 501" fill="currentColor">
                <path d="M123.121 33.664C188.241 82.553 258.281 181.68 284 234.873c25.719-53.192 95.759-152.32 160.879-201.21C491.866-1.611 568-28.906 568 57.947c0 17.346-9.945 145.713-15.778 166.555-20.275 72.453-94.155 90.933-159.875 79.748C507.222 323.8 536.444 388.56 502.222 434.602 430.398 531.552 366.444 440.09 316.889 370.177 306.293 354.622 296.889 339.2 284 324.264c-12.889 14.936-22.293 30.358-32.889 45.913C201.556 440.09 137.602 531.551 65.778 434.602 31.556 388.56 60.778 323.8 175.654 304.25 109.934 315.435 36.054 296.955 15.778 224.502 9.945 203.661 0 75.293 0 57.947 0-28.906 76.134-1.612 123.121 33.664z" />
              </svg>
              {loading ? 'Signing in...' : 'Sign in'}
            </button>
          </div>
        </div>

        <div className="mt-8 text-center">
          <p className="text-xs text-text-muted">
            Concord is open source &middot; IRC compatible &middot; Self-hosted &middot; Powered by AT Protocol
          </p>
        </div>
      </div>
    </div>
  );
}
