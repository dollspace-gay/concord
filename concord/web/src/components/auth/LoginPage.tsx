import { useState } from 'react';
import { useAuthStore } from '../../stores/authStore';

export function LoginPage() {
  const providers = useAuthStore((s) => s.providers);
  const [handle, setHandle] = useState('');
  const [bskyLoading, setBskyLoading] = useState(false);

  const handleBskyLogin = () => {
    const trimmed = handle.trim();
    if (!trimmed) return;
    setBskyLoading(true);
    window.location.href = `/api/auth/atproto/login?handle=${encodeURIComponent(trimmed)}`;
  };

  return (
    <div className="flex h-full items-center justify-center bg-bg-primary">
      <div className="w-full max-w-md rounded-lg bg-bg-secondary p-8">
        <div className="mb-8 text-center">
          <h1 className="mb-2 text-2xl font-bold text-text-primary">Welcome to Concord</h1>
          <p className="text-text-muted">Sign in to start chatting</p>
        </div>

        <div className="space-y-3">
          {providers.includes('atproto') && (
            <div className="space-y-2">
              <div className="flex gap-2">
                <input
                  type="text"
                  value={handle}
                  onChange={(e) => setHandle(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && handleBskyLogin()}
                  placeholder="handle.bsky.social"
                  className="flex-1 rounded-md border border-border bg-bg-primary px-4 py-3 text-text-primary placeholder-text-muted focus:border-accent-primary focus:outline-none"
                  disabled={bskyLoading}
                />
                <button
                  onClick={handleBskyLogin}
                  disabled={!handle.trim() || bskyLoading}
                  className="flex items-center gap-2 rounded-md bg-[#0085ff] px-4 py-3 font-medium text-white transition-colors hover:bg-[#0070dd] disabled:opacity-50"
                >
                  <svg className="h-5 w-5" viewBox="0 0 568 501" fill="currentColor">
                    <path d="M123.121 33.664C188.241 82.553 258.281 181.68 284 234.873c25.719-53.192 95.759-152.32 160.879-201.21C491.866-1.611 568-28.906 568 57.947c0 17.346-9.945 145.713-15.778 166.555-20.275 72.453-94.155 90.933-159.875 79.748C507.222 323.8 536.444 388.56 502.222 434.602 430.398 531.552 366.444 440.09 316.889 370.177 306.293 354.622 296.889 339.2 284 324.264c-12.889 14.936-22.293 30.358-32.889 45.913C201.556 440.09 137.602 531.551 65.778 434.602 31.556 388.56 60.778 323.8 175.654 304.25 109.934 315.435 36.054 296.955 15.778 224.502 9.945 203.661 0 75.293 0 57.947 0-28.906 76.134-1.612 123.121 33.664z" />
                  </svg>
                  {bskyLoading ? 'Signing in...' : 'Bluesky'}
                </button>
              </div>
            </div>
          )}

          {providers.includes('github') && (
            <a
              href="/api/auth/github"
              className="flex w-full items-center justify-center gap-3 rounded-md bg-[#24292e] px-4 py-3 font-medium text-white transition-colors hover:bg-[#2f363d]"
            >
              <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z" />
              </svg>
              Continue with GitHub
            </a>
          )}

          {providers.includes('google') && (
            <a
              href="/api/auth/google"
              className="flex w-full items-center justify-center gap-3 rounded-md bg-white px-4 py-3 font-medium text-gray-800 transition-colors hover:bg-gray-100"
            >
              <svg className="h-5 w-5" viewBox="0 0 24 24">
                <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" />
                <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
                <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" />
                <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" />
              </svg>
              Continue with Google
            </a>
          )}

          {providers.length === 0 && (
            <p className="text-center text-text-muted">
              No OAuth providers configured. Set GITHUB_CLIENT_ID/SECRET or
              GOOGLE_CLIENT_ID/SECRET environment variables on the server.
            </p>
          )}
        </div>

        <div className="mt-8 text-center">
          <p className="text-xs text-text-muted">
            Concord is open source &middot; IRC compatible &middot; Self-hosted
          </p>
        </div>
      </div>
    </div>
  );
}
