

// ── Server types ────────────────────────────────────────

export interface UserProfile {
  id: string;
  username: string;
  email: string | null;
  avatar_url: string | null;
  is_system_admin?: boolean;
}

export interface AuthStatus {
  authenticated: boolean;
  providers: string[];
}
