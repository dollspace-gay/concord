import type { AuthStatus, ChannelInfo, CreateTokenResponse, HistoryResponse, IrcToken, PublicUserProfile, ServerInfo, UserProfile } from './types';

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

// Channels (legacy endpoints, require server_id query param on server)
export const getChannels = () => request<ChannelInfo[]>('/channels');
export const getChannelHistory = (name: string, before?: string, limit = 50) => {
  const ch = name.startsWith('#') ? name.slice(1) : name;
  const params = new URLSearchParams({ limit: String(limit) });
  if (before) params.set('before', before);
  return request<HistoryResponse>(`/channels/${encodeURIComponent(ch)}/messages?${params}`);
};

// Servers
export const listServers = () => request<ServerInfo[]>('/servers');
export const createServer = (name: string, icon_url?: string) =>
  request<ServerInfo>('/servers', {
    method: 'POST',
    body: JSON.stringify({ name, icon_url: icon_url || null }),
  });
export const getServer = (id: string) => request<ServerInfo>(`/servers/${encodeURIComponent(id)}`);
export const deleteServer = (id: string) =>
  request<void>(`/servers/${encodeURIComponent(id)}`, { method: 'DELETE' });
export const listServerChannels = (serverId: string) =>
  request<ChannelInfo[]>(`/servers/${encodeURIComponent(serverId)}/channels`);
export const getServerChannelHistory = (serverId: string, channelName: string, before?: string, limit = 50) => {
  const ch = channelName.startsWith('#') ? channelName.slice(1) : channelName;
  const params = new URLSearchParams({ limit: String(limit) });
  if (before) params.set('before', before);
  return request<HistoryResponse>(`/servers/${encodeURIComponent(serverId)}/channels/${encodeURIComponent(ch)}/messages?${params}`);
};
export const listServerMembers = (serverId: string) =>
  request<{ user_id: string; role: string; joined_at: string }[]>(`/servers/${encodeURIComponent(serverId)}/members`);

// User profiles
export const getUserProfile = (nickname: string) =>
  request<PublicUserProfile>(`/users/${encodeURIComponent(nickname)}`);

// IRC Tokens
export const getTokens = () => request<IrcToken[]>('/tokens');
export const createToken = (label?: string) =>
  request<CreateTokenResponse>('/tokens', {
    method: 'POST',
    body: JSON.stringify({ label: label || null }),
  });
export const deleteToken = (id: string) =>
  request<void>(`/tokens/${encodeURIComponent(id)}`, { method: 'DELETE' });

// Admin
export const adminListServers = () => request<ServerInfo[]>('/admin/servers');
export const adminDeleteServer = (id: string) =>
  request<void>(`/admin/servers/${encodeURIComponent(id)}`, { method: 'DELETE' });
export const adminSetAdmin = (userId: string, isAdmin: boolean) =>
  request<void>(`/admin/users/${encodeURIComponent(userId)}/admin`, {
    method: 'PUT',
    body: JSON.stringify({ is_admin: isAdmin }),
  });
