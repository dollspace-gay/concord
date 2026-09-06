import type { AttachmentInfo, AtprotoChannelPublicationPolicy, AtprotoPublicationStatus, AuthStatus, BlueskyIdentityInfo, BlueskyShareResult, ChannelInfo, CreateTokenResponse, HistoryResponse, IrcToken, PublicUserProfile, ServerInfo, UserProfile, UserProfileInfo } from './types';

const BASE = '/api';

export class HttpError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'HttpError';
    this.status = status;
  }
}

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
    throw new HttpError(res.status, text || `HTTP ${res.status}`);
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

export interface ServerFolderData {
  id: string;
  name: string;
  color?: string | null;
  server_ids: string[];
  collapsed?: boolean;
}

export const listServerFolders = () => request<ServerFolderData[]>('/server-folders');
export const replaceServerFolders = (folders: ServerFolderData[]) =>
  request<void>('/server-folders', { method: 'PUT', body: JSON.stringify(folders) });

// User profiles
export const getUserProfile = (nickname: string) =>
  request<PublicUserProfile>(`/users/${encodeURIComponent(nickname)}`);
export const getFullUserProfile = (userId: string) =>
  request<UserProfileInfo>(`/users/${encodeURIComponent(userId)}/profile`);
export const updateProfile = (profile: Partial<Pick<UserProfileInfo, 'bio' | 'pronouns' | 'avatar_url' | 'banner_url'>>) =>
  request<void>('/profile', { method: 'PATCH', body: JSON.stringify(profile) });
export const updateServerMemberAvatar = (serverId: string, iconUrl: string) =>
  request<void>(`/servers/${encodeURIComponent(serverId)}/member-media`, {
    method: 'PATCH', body: JSON.stringify({ icon_url: iconUrl }),
  });

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

// Custom emoji
export interface CustomEmoji {
  id: string;
  server_id: string;
  name: string;
  image_url: string;
}

export const listServerEmoji = (serverId: string) =>
  request<CustomEmoji[]>(`/servers/${encodeURIComponent(serverId)}/emoji`);
export const createServerEmoji = (serverId: string, name: string, imageUrl: string) =>
  request<CustomEmoji>(`/servers/${encodeURIComponent(serverId)}/emoji`, {
    method: 'POST',
    body: JSON.stringify({ name, image_url: imageUrl }),
  });
export const deleteServerEmoji = (serverId: string, emojiId: string) =>
  request<void>(`/servers/${encodeURIComponent(serverId)}/emoji/${encodeURIComponent(emojiId)}`, {
    method: 'DELETE',
  });

// Bluesky / AT Protocol
export const syncBlueskyProfile = () =>
  request<void>('/bluesky/sync-profile', { method: 'POST' });
export const getBlueskyIdentity = (userId: string) =>
  request<BlueskyIdentityInfo>(`/users/${encodeURIComponent(userId)}/bluesky`);
export const shareToBluesky = (messageId: string) =>
  request<BlueskyShareResult>(`/messages/${encodeURIComponent(messageId)}/share-bluesky`, { method: 'POST' });
export const listAtprotoPublications = () =>
  request<AtprotoPublicationStatus[]>('/atproto/publications');
export const retryAtprotoPublication = (publicationId: string) =>
  request<{ id: string; status: string; remote_uri: string | null; remote_cid: string | null }>(`/atproto/publications/${encodeURIComponent(publicationId)}/retry`, { method: 'POST' });
export const getAtprotoChannelPublicationPolicy = (channelId: string) =>
  request<AtprotoChannelPublicationPolicy>(`/channels/${encodeURIComponent(channelId)}/atproto-publication`);
export const setAtprotoChannelEnabled = (channelId: string, enabled: boolean) =>
  request<AtprotoChannelPublicationPolicy>(`/channels/${encodeURIComponent(channelId)}/atproto-publication`, {
    method: 'PATCH', body: JSON.stringify({ enabled }),
  });
export const setAtprotoPublicationGrant = (channelId: string, enabled: boolean) =>
  request<AtprotoChannelPublicationPolicy>('/settings/atproto-sync', {
    method: 'PATCH', body: JSON.stringify({ channel_id: channelId, enabled }),
  });
export const getAtprotoSyncSetting = () =>
  request<{ atproto_sync_enabled: boolean }>('/settings/atproto-sync');
export const updateAtprotoSyncSetting = (enabled: boolean) =>
  request<{ atproto_sync_enabled: boolean }>('/settings/atproto-sync', {
    method: 'PATCH',
    body: JSON.stringify({ enabled }),
  });

// Stickers
export interface StickerData {
  id: string;
  server_id: string;
  name: string;
  image_url: string;
  description?: string | null;
}

export const listServerStickers = (serverId: string) =>
  request<StickerData[]>(`/servers/${encodeURIComponent(serverId)}/stickers`);
export const createServerSticker = (serverId: string, name: string, imageUrl: string, description?: string) =>
  request<StickerData>(`/servers/${encodeURIComponent(serverId)}/stickers`, {
    method: 'POST',
    body: JSON.stringify({ name, image_url: imageUrl, description: description || null }),
  });
export const deleteServerSticker = (serverId: string, stickerId: string) =>
  request<void>(`/servers/${encodeURIComponent(serverId)}/stickers/${encodeURIComponent(stickerId)}`, {
    method: 'DELETE',
  });

// Cross-server emoji
export const listAllUserEmoji = (targetServerId: string) =>
  request<{ server_id: string; name: string; image_url: string }[]>(
    `/users/me/emoji?target_server_id=${encodeURIComponent(targetServerId)}`,
  );

// Emoji settings
export const updateServerEmojiSettings = (serverId: string, allowExternal: boolean, shareable: boolean) =>
  request<void>(`/servers/${encodeURIComponent(serverId)}/emoji-settings`, {
    method: 'PATCH',
    body: JSON.stringify({ allow_external_emoji: allowExternal, shareable_emoji: shareable }),
  });

// Server limits
export const getServerLimits = () =>
  request<{ max_message_length: number; max_file_size_mb: number }>('/config/limits');

// File uploads
export async function uploadFile(
  file: File,
  target?: { conversationId?: string; serverId?: string; channel?: string; purpose?: 'message' | 'emoji' | 'sticker' | 'server_avatar' | 'server_member_avatar' | 'user_avatar' | 'user_banner' },
  options?: { signal?: AbortSignal; onProgress?: (loaded: number, total: number) => void },
): Promise<AttachmentInfo> {
  const formData = new FormData();
  formData.append('file', file);

  const params = new URLSearchParams();
  if (target?.conversationId) params.set('conversation_id', target.conversationId);
  if (target?.serverId) params.set('server_id', target.serverId);
  if (target?.channel) params.set('channel', target.channel);
  if (target?.purpose) params.set('purpose', target.purpose);
  const query = params.size ? `?${params.toString()}` : '';
  return new Promise<AttachmentInfo>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const abort = () => xhr.abort();
    xhr.open('POST', `${BASE}/uploads${query}`);
    xhr.withCredentials = true;
    xhr.responseType = 'json';
    xhr.upload.onprogress = (event) => options?.onProgress?.(event.loaded, event.lengthComputable ? event.total : file.size);
    xhr.onload = () => {
      options?.signal?.removeEventListener('abort', abort);
      if (xhr.status >= 200 && xhr.status < 300) {
        resolve(xhr.response as AttachmentInfo);
      } else {
        const message = typeof xhr.response === 'string'
          ? xhr.response
          : (xhr.response as { error?: string } | null)?.error;
        reject(new HttpError(xhr.status, message || xhr.statusText || `Upload failed: HTTP ${xhr.status}`));
      }
    };
    xhr.onerror = () => {
      options?.signal?.removeEventListener('abort', abort);
      reject(new Error('Upload failed because the network connection was lost'));
    };
    xhr.onabort = () => {
      options?.signal?.removeEventListener('abort', abort);
      reject(new DOMException('Upload cancelled', 'AbortError'));
    };
    if (options?.signal?.aborted) return reject(new DOMException('Upload cancelled', 'AbortError'));
    options?.signal?.addEventListener('abort', abort, { once: true });
    xhr.send(formData);
  });
}
