import type { AuthStatus, ChannelInfo, CreateTokenResponse, HistoryResponse, IrcToken, UserProfile } from './types';

const BASE = '/api';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    credentials: 'include',
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
  });

  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(text || `HTTP ${res.status}`);
  }

  if (res.status === 204) return undefined as T;
  return res.json();
}

// Auth
export const getAuthStatus = () => request<AuthStatus>('/auth/status');
export const getMe = () => request<UserProfile>('/me');
export const logout = () => request<void>('/auth/logout', { method: 'POST' });

// Channels
export const getChannels = () => request<ChannelInfo[]>('/channels');
export const getChannelHistory = (name: string, before?: string, limit = 50) => {
  const ch = name.startsWith('#') ? name.slice(1) : name;
  const params = new URLSearchParams({ limit: String(limit) });
  if (before) params.set('before', before);
  return request<HistoryResponse>(`/channels/${encodeURIComponent(ch)}/messages?${params}`);
};

// IRC Tokens
export const getTokens = () => request<IrcToken[]>('/tokens');
export const createToken = (label?: string) =>
  request<CreateTokenResponse>('/tokens', {
    method: 'POST',
    body: JSON.stringify({ label: label || null }),
  });
export const deleteToken = (id: string) =>
  request<void>(`/tokens/${encodeURIComponent(id)}`, { method: 'DELETE' });
