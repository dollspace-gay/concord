import { useAuthStore } from '../../stores/authStore';

export function LoginPage() {
  const providers = useAuthStore((s) => s.providers);

  return (
    <div className="flex h-full items-center justify-center bg-bg-primary">
      <div className="w-full max-w-md rounded-lg bg-bg-secondary p-8">
        <div className="mb-8 text-center">
          <h1 className="mb-2 text-2xl font-bold text-text-primary">Welcome to Concord</h1>
          <p className="text-text-muted">Sign in to start chatting</p>
        </div>

        <div className="space-y-3">
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
