import { create } from 'zustand';

export interface ServerFolder {
  id: string;
  name: string;
  serverIds: string[];
  collapsed: boolean;
}

interface UiState {
  activeServer: string | null;
  activeChannel: string | null;
  showMemberList: boolean;
  showSettings: boolean;
  showServerSettings: boolean;
  /** category_id -> collapsed */
  collapsedCategories: Record<string, boolean>;
  /** Client-side server folder groupings (persisted to localStorage) */
  serverFolders: ServerFolder[];
  showSearch: boolean;
  showUserProfile: string | null; // user_id to show, null = hidden
  showQuickSwitcher: boolean;
  showPinnedMessages: boolean;
  showThreadPanel: boolean;
  activeThreadId: string | null;
  showBookmarks: boolean;
  showModerationPanel: boolean;
  showCommunityPanel: boolean;
  showIntegrationsPanel: boolean;

  setActiveServer: (serverId: string | null) => void;
  setActiveChannel: (channel: string | null) => void;
  toggleMemberList: () => void;
  setShowSettings: (show: boolean) => void;
  setShowServerSettings: (show: boolean) => void;
  toggleCategory: (categoryId: string) => void;
  setServerFolders: (folders: ServerFolder[]) => void;
  addServerFolder: (name: string, serverIds: string[]) => void;
  removeServerFolder: (folderId: string) => void;
  toggleServerFolder: (folderId: string) => void;
  setShowSearch: (show: boolean) => void;
  setShowUserProfile: (userId: string | null) => void;
  setShowQuickSwitcher: (show: boolean) => void;
  setShowPinnedMessages: (show: boolean) => void;
  setShowThreadPanel: (show: boolean) => void;
  setActiveThreadId: (threadId: string | null) => void;
  setShowBookmarks: (show: boolean) => void;
  setShowModerationPanel: (show: boolean) => void;
  setShowCommunityPanel: (show: boolean) => void;
  setShowIntegrationsPanel: (show: boolean) => void;
}

function loadFolders(): ServerFolder[] {
  try {
    const raw = localStorage.getItem('concord:server-folders');
    if (raw) return JSON.parse(raw);
  } catch { /* ignore */ }
  return [];
}

function saveFolders(folders: ServerFolder[]) {
  localStorage.setItem('concord:server-folders', JSON.stringify(folders));
}

export const useUiStore = create<UiState>((set) => ({
  activeServer: null,
  activeChannel: null,
  showMemberList: true,
  showSettings: false,
  showServerSettings: false,
  collapsedCategories: {},
  serverFolders: loadFolders(),
  showSearch: false,
  showUserProfile: null,
  showQuickSwitcher: false,
  showPinnedMessages: false,
  showThreadPanel: false,
  activeThreadId: null,
  showBookmarks: false,
  showModerationPanel: false,
  showCommunityPanel: false,
  showIntegrationsPanel: false,

  setActiveServer: (serverId) => set({ activeServer: serverId, activeChannel: null }),
  setActiveChannel: (channel) => set({ activeChannel: channel }),
  toggleMemberList: () => set((s) => ({ showMemberList: !s.showMemberList })),
  setShowSettings: (show) => set({ showSettings: show }),
  setShowServerSettings: (show) => set({ showServerSettings: show }),

  toggleCategory: (categoryId) =>
    set((s) => ({
      collapsedCategories: {
        ...s.collapsedCategories,
        [categoryId]: !s.collapsedCategories[categoryId],
      },
    })),

  setServerFolders: (folders) => {
    saveFolders(folders);
    set({ serverFolders: folders });
  },

  addServerFolder: (name, serverIds) =>
    set((s) => {
      const folder: ServerFolder = {
        id: crypto.randomUUID(),
        name,
        serverIds,
        collapsed: false,
      };
      const updated = [...s.serverFolders, folder];
      saveFolders(updated);
      return { serverFolders: updated };
    }),

  removeServerFolder: (folderId) =>
    set((s) => {
      const updated = s.serverFolders.filter((f) => f.id !== folderId);
      saveFolders(updated);
      return { serverFolders: updated };
    }),

  toggleServerFolder: (folderId) =>
    set((s) => {
      const updated = s.serverFolders.map((f) =>
        f.id === folderId ? { ...f, collapsed: !f.collapsed } : f,
      );
      saveFolders(updated);
      return { serverFolders: updated };
    }),

  setShowSearch: (show) => set({ showSearch: show }),
  setShowUserProfile: (userId) => set({ showUserProfile: userId }),
  setShowQuickSwitcher: (show) => set({ showQuickSwitcher: show }),
  setShowPinnedMessages: (show) => set({ showPinnedMessages: show }),
  setShowThreadPanel: (show) => set({ showThreadPanel: show }),
  setActiveThreadId: (threadId) => set({ activeThreadId: threadId, showThreadPanel: threadId !== null }),
  setShowBookmarks: (show) => set({ showBookmarks: show }),
  setShowModerationPanel: (show) => set({ showModerationPanel: show }),
  setShowCommunityPanel: (show) => set({ showCommunityPanel: show }),
  setShowIntegrationsPanel: (show) => set({ showIntegrationsPanel: show }),
}));
