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

let authGeneration = 0;

export const useAuthStore = create<AuthState>((set) => ({
  user: null,
  providers: [],
  loading: true,
  error: null,

  checkAuth: async () => {
    const generation = ++authGeneration;
    set({ loading: true, error: null });
    try {
      const status = await api.getAuthStatus();
      if (generation !== authGeneration) return;
      set({ providers: status.providers });

      try {
        const user = await api.getMe();
        if (generation !== authGeneration) return;
        set({ user, loading: false });
      } catch (e) {
        if (generation !== authGeneration) return;
        if (e instanceof api.HttpError && e.status === 401) {
          set({ user: null, loading: false });
        } else {
          set({ error: `Unable to verify sign-in: ${String(e)}`, loading: false });
        }
      }
    } catch (e) {
      if (generation !== authGeneration) return;
      set({ error: String(e), loading: false });
    }
  },

  logout: async () => {
    ++authGeneration;
    set({ user: null, loading: false, error: null });
    try {
      await api.logout();
    } catch (e) {
      set({ error: `Signed out locally, but the server session could not be revoked: ${String(e)}` });
    }
  },
}));
