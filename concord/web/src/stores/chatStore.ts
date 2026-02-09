import { create } from 'zustand';
import type { ChannelInfo, HistoryMessage, ServerEvent } from '../api/types';
import { WebSocketManager } from '../api/websocket';

interface ChatState {
  connected: boolean;
  channels: ChannelInfo[];
  messages: Record<string, HistoryMessage[]>;
  members: Record<string, string[]>;
  hasMore: Record<string, boolean>;
  ws: WebSocketManager | null;

  connect: (nickname: string) => void;
  disconnect: () => void;
  handleEvent: (event: ServerEvent) => void;
  sendMessage: (channel: string, content: string) => void;
  joinChannel: (channel: string) => void;
  partChannel: (channel: string) => void;
  setTopic: (channel: string, topic: string) => void;
  fetchHistory: (channel: string, before?: string) => void;
  listChannels: () => void;
  getMembers: (channel: string) => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  connected: false,
  channels: [],
  messages: {},
  members: {},
  hasMore: {},
  ws: null,

  connect: (nickname: string) => {
    const existing = get().ws;
    if (existing) existing.disconnect();

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${protocol}//${window.location.host}/ws?nickname=${encodeURIComponent(nickname)}`;

    const ws = new WebSocketManager(
      url,
      (event) => get().handleEvent(event),
      (connected) => {
        set({ connected });
        if (connected) {
          // Request channel list on connect
          ws.send({ type: 'list_channels' });
        }
      },
    );

    set({ ws });
    ws.connect();
  },

  disconnect: () => {
    get().ws?.disconnect();
    set({ ws: null, connected: false });
  },

  handleEvent: (event: ServerEvent) => {
    switch (event.type) {
      case 'message': {
        const msg: HistoryMessage = {
          id: event.id,
          from: event.from,
          content: event.content,
          timestamp: event.timestamp,
        };
        set((s) => ({
          messages: {
            ...s.messages,
            [event.target]: [...(s.messages[event.target] || []), msg],
          },
        }));
        break;
      }

      case 'join': {
        set((s) => {
          const current = s.members[event.channel] || [];
          if (current.includes(event.nickname)) return s;
          return {
            members: {
              ...s.members,
              [event.channel]: [...current, event.nickname],
            },
          };
        });
        break;
      }

      case 'part': {
        set((s) => ({
          members: {
            ...s.members,
            [event.channel]: (s.members[event.channel] || []).filter(
              (n) => n !== event.nickname,
            ),
          },
        }));
        break;
      }

      case 'quit': {
        set((s) => {
          const newMembers = { ...s.members };
          for (const ch in newMembers) {
            newMembers[ch] = newMembers[ch].filter((n) => n !== event.nickname);
          }
          return { members: newMembers };
        });
        break;
      }

      case 'names': {
        set((s) => ({
          members: { ...s.members, [event.channel]: event.members },
        }));
        break;
      }

      case 'topic_change': {
        set((s) => ({
          channels: s.channels.map((ch) =>
            ch.name === event.channel ? { ...ch, topic: event.topic } : ch,
          ),
        }));
        break;
      }

      case 'channel_list': {
        set({ channels: event.channels });
        break;
      }

      case 'history': {
        set((s) => ({
          messages: {
            ...s.messages,
            [event.channel]: [
              ...event.messages.reverse(),
              ...(s.messages[event.channel] || []),
            ],
          },
          hasMore: { ...s.hasMore, [event.channel]: event.has_more },
        }));
        break;
      }

      case 'error': {
        console.error(`Server error [${event.code}]: ${event.message}`);
        break;
      }
    }
  },

  sendMessage: (channel, content) => {
    get().ws?.send({ type: 'send_message', channel, content });
  },

  joinChannel: (channel) => {
    get().ws?.send({ type: 'join_channel', channel });
  },

  partChannel: (channel) => {
    get().ws?.send({ type: 'part_channel', channel });
  },

  setTopic: (channel, topic) => {
    get().ws?.send({ type: 'set_topic', channel, topic });
  },

  fetchHistory: (channel, before) => {
    get().ws?.send({ type: 'fetch_history', channel, before, limit: 50 });
  },

  listChannels: () => {
    get().ws?.send({ type: 'list_channels' });
  },

  getMembers: (channel) => {
    get().ws?.send({ type: 'get_members', channel });
  },
}));
