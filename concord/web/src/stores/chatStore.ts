import { create } from 'zustand';
import type { ChannelInfo, HistoryMessage, MemberInfo, ServerEvent, ServerInfo } from '../api/types';
import { channelKey } from '../api/types';
import { WebSocketManager } from '../api/websocket';

// Stable empty references to prevent zustand selector re-render loops.
// Inline [] / {} in selectors create new references on every evaluation,
// failing Object.is comparison and causing infinite re-renders with React 19.
const EMPTY_SERVERS: ServerInfo[] = [];
const EMPTY_CHANNELS_MAP: Record<string, ChannelInfo[]> = {};
const EMPTY_MESSAGES_MAP: Record<string, HistoryMessage[]> = {};
const EMPTY_MEMBERS_MAP: Record<string, MemberInfo[]> = {};
const EMPTY_HAS_MORE: Record<string, boolean> = {};
const EMPTY_AVATARS: Record<string, string> = {};

interface ChatState {
  connected: boolean;
  nickname: string | null;
  servers: ServerInfo[];
  channels: Record<string, ChannelInfo[]>;   // server_id -> channels
  messages: Record<string, HistoryMessage[]>; // channelKey -> messages
  members: Record<string, MemberInfo[]>;      // channelKey -> members
  hasMore: Record<string, boolean>;           // channelKey -> has_more
  /** nickname -> avatar_url cache (populated from Names/Join/Message events) */
  avatars: Record<string, string>;
  ws: WebSocketManager | null;

  connect: (nickname: string) => void;
  disconnect: () => void;
  handleEvent: (event: ServerEvent) => void;
  sendMessage: (serverId: string, channel: string, content: string) => void;
  joinChannel: (serverId: string, channel: string) => void;
  partChannel: (serverId: string, channel: string) => void;
  setTopic: (serverId: string, channel: string, topic: string) => void;
  fetchHistory: (serverId: string, channel: string, before?: string) => void;
  listChannels: (serverId: string) => void;
  getMembers: (serverId: string, channel: string) => void;
  listServers: () => void;
  createServer: (name: string, iconUrl?: string) => void;
  joinServer: (serverId: string) => void;
  leaveServer: (serverId: string) => void;
  createChannel: (serverId: string, name: string) => void;
  deleteChannel: (serverId: string, channel: string) => void;
  deleteServer: (serverId: string) => void;
}

/** Cache an avatar_url for a nickname if present. */
function cacheAvatar(avatars: Record<string, string>, nickname: string, avatar_url?: string | null): Record<string, string> {
  if (avatar_url && avatars[nickname] !== avatar_url) {
    return { ...avatars, [nickname]: avatar_url };
  }
  return avatars;
}

export const useChatStore = create<ChatState>((set, get) => ({
  connected: false,
  nickname: null,
  servers: EMPTY_SERVERS,
  channels: EMPTY_CHANNELS_MAP,
  messages: EMPTY_MESSAGES_MAP,
  members: EMPTY_MEMBERS_MAP,
  hasMore: EMPTY_HAS_MORE,
  avatars: EMPTY_AVATARS,
  ws: null,

  connect: (nickname: string) => {
    if (get().ws) {
      return;
    }

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${protocol}//${window.location.host}/ws?nickname=${encodeURIComponent(nickname)}`;

    const ws = new WebSocketManager(
      url,
      (event) => {
        get().handleEvent(event);
      },
      (connected) => {
        set({ connected });
        if (connected) {
          ws.send({ type: 'list_servers' });
        }
      },
    );

    set({ ws, nickname });
    ws.connect();
  },

  disconnect: () => {
    get().ws?.disconnect();
    set({
      ws: null,
      connected: false,
      servers: EMPTY_SERVERS,
      channels: EMPTY_CHANNELS_MAP,
      messages: EMPTY_MESSAGES_MAP,
      members: EMPTY_MEMBERS_MAP,
      hasMore: EMPTY_HAS_MORE,
      avatars: EMPTY_AVATARS,
    });
  },

  handleEvent: (event: ServerEvent) => {
    switch (event.type) {
      case 'message': {
        const sid = event.server_id || 'default';
        const key = channelKey(sid, event.target);
        const msg: HistoryMessage = {
          id: event.id,
          from: event.from,
          content: event.content,
          timestamp: event.timestamp,
        };
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: [...(s.messages[key] || []), msg],
          },
          avatars: cacheAvatar(s.avatars, event.from, event.avatar_url),
        }));
        break;
      }

      case 'join': {
        const key = channelKey(event.server_id, event.channel);
        const memberInfo: MemberInfo = { nickname: event.nickname, avatar_url: event.avatar_url };
        set((s) => {
          const current = s.members[key] || [];
          if (current.some((m) => m.nickname === event.nickname)) return s;
          return {
            members: {
              ...s.members,
              [key]: [...current, memberInfo],
            },
            avatars: cacheAvatar(s.avatars, event.nickname, event.avatar_url),
          };
        });
        break;
      }

      case 'part': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          members: {
            ...s.members,
            [key]: (s.members[key] || []).filter(
              (m) => m.nickname !== event.nickname,
            ),
          },
        }));
        break;
      }

      case 'quit': {
        set((s) => {
          const newMembers = { ...s.members };
          for (const ch in newMembers) {
            newMembers[ch] = newMembers[ch].filter((m) => m.nickname !== event.nickname);
          }
          return { members: newMembers };
        });
        break;
      }

      case 'names': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => {
          let newAvatars = { ...s.avatars };
          for (const m of event.members) {
            if (m.avatar_url) {
              newAvatars[m.nickname] = m.avatar_url;
            }
          }
          return {
            members: { ...s.members, [key]: event.members },
            avatars: newAvatars,
          };
        });
        break;
      }

      case 'topic_change': {
        set((s) => {
          const serverChannels = s.channels[event.server_id];
          if (!serverChannels) return s;
          return {
            channels: {
              ...s.channels,
              [event.server_id]: serverChannels.map((ch) =>
                ch.name === event.channel ? { ...ch, topic: event.topic } : ch,
              ),
            },
          };
        });
        break;
      }

      case 'channel_list': {
        set((s) => ({
          channels: { ...s.channels, [event.server_id]: event.channels },
        }));
        break;
      }

      case 'history': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: [
              ...event.messages.reverse(),
              ...(s.messages[key] || []),
            ],
          },
          hasMore: { ...s.hasMore, [key]: event.has_more },
        }));
        break;
      }

      case 'server_list': {
        set({ servers: event.servers });
        break;
      }

      case 'error': {
        console.error(`Server error [${event.code}]: ${event.message}`);
        break;
      }
    }
  },

  sendMessage: (serverId, channel, content) => {
    const { ws, nickname } = get();
    if (!ws || !nickname) return;

    const key = channelKey(serverId, channel);

    // Add message locally (server excludes sender from broadcast)
    const msg: HistoryMessage = {
      id: crypto.randomUUID(),
      from: nickname,
      content,
      timestamp: new Date().toISOString(),
    };
    set((s) => ({
      messages: {
        ...s.messages,
        [key]: [...(s.messages[key] || []), msg],
      },
    }));

    ws.send({ type: 'send_message', server_id: serverId, channel, content });
  },

  joinChannel: (serverId, channel) => {
    get().ws?.send({ type: 'join_channel', server_id: serverId, channel });
  },

  partChannel: (serverId, channel) => {
    get().ws?.send({ type: 'part_channel', server_id: serverId, channel });
  },

  setTopic: (serverId, channel, topic) => {
    get().ws?.send({ type: 'set_topic', server_id: serverId, channel, topic });
  },

  fetchHistory: (serverId, channel, before) => {
    get().ws?.send({ type: 'fetch_history', server_id: serverId, channel, before, limit: 50 });
  },

  listChannels: (serverId) => {
    get().ws?.send({ type: 'list_channels', server_id: serverId });
  },

  getMembers: (serverId, channel) => {
    get().ws?.send({ type: 'get_members', server_id: serverId, channel });
  },

  listServers: () => {
    get().ws?.send({ type: 'list_servers' });
  },

  createServer: (name, iconUrl) => {
    get().ws?.send({ type: 'create_server', name, icon_url: iconUrl });
  },

  joinServer: (serverId) => {
    get().ws?.send({ type: 'join_server', server_id: serverId });
  },

  leaveServer: (serverId) => {
    get().ws?.send({ type: 'leave_server', server_id: serverId });
  },

  createChannel: (serverId, name) => {
    get().ws?.send({ type: 'create_channel', server_id: serverId, name });
  },

  deleteChannel: (serverId, channel) => {
    get().ws?.send({ type: 'delete_channel', server_id: serverId, channel });
  },

  deleteServer: (serverId) => {
    get().ws?.send({ type: 'delete_server', server_id: serverId });
  },
}));
