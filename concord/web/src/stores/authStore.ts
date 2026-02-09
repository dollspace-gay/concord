import { create } from 'zustand';
import type { UserProfile } from '../api/types';
import * as api from '../api/client';

interface AuthState {
  user: UserProfile | null;
  providers: string[];
  loading: boolean;
  error: string | null;

  checkAuth: () => Promise<void>;
  logout: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set) => ({
  user: null,
  providers: [],
  loading: true,
  error: null,

  checkAuth: async () => {
    set({ loading: true, error: null });
    try {
      const status = await api.getAuthStatus();
      set({ providers: status.providers });

      try {
        const user = await api.getMe();
        set({ user, loading: false });
      } catch {
        // Not authenticated — that's fine
        set({ user: null, loading: false });
      }
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  logout: async () => {
    try {
      await api.logout();
    } catch {
      // ignore
    }
    set({ user: null });
  },
}));
