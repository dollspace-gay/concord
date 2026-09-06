import { create } from 'zustand';
import * as api from '../api/client';

export interface ServerFolder {
  id: string;
  name: string;
  color?: string;
  serverIds: string[];
  collapsed: boolean;
}

interface UiState {
  activeServer: string | null;
  activeChannel: string | null;
  activeDirectConversation: string | null;
  showMemberList: boolean;
  showSettings: boolean;
  showServerSettings: boolean;
  /** category_id -> collapsed */
  collapsedCategories: Record<string, boolean>;
  /** Client-side server folder groupings (persisted to localStorage) */
  serverFolders: ServerFolder[];
  folderRevision: number;
  folderSyncStatus: 'idle' | 'saving' | 'error';
  folderSyncError: string | null;
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
  jumpToMessageId: string | null;

  setActiveServer: (serverId: string | null) => void;
  setActiveChannel: (channel: string | null) => void;
  setActiveDirectConversation: (conversationId: string | null) => void;
  toggleMemberList: () => void;
  setShowSettings: (show: boolean) => void;
  setShowServerSettings: (show: boolean) => void;
  toggleCategory: (categoryId: string) => void;
  setServerFolders: (folders: ServerFolder[]) => void;
  hydrateServerFolders: (accountId: string | null, folders?: ServerFolder[]) => void;
  addServerFolder: (name: string, serverIds: string[]) => void;
  removeServerFolder: (folderId: string) => void;
  toggleServerFolder: (folderId: string) => void;
  retryServerFolderSync: () => void;
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
  setJumpToMessageId: (messageId: string | null) => void;
}

let folderAccountId: string | null = null;
let folderGeneration = 0;
let pendingFolders: ServerFolder[] | null = null;
let folderSaveRunning = false;

function loadCollapsed(accountId: string): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(`concord:server-folder-state:${accountId}`);
    if (raw) return JSON.parse(raw);
  } catch { /* ignore */ }
  return {};
}

function saveCollapsed(folders: ServerFolder[]) {
  if (!folderAccountId) return;
  localStorage.setItem(
    `concord:server-folder-state:${folderAccountId}`,
    JSON.stringify(Object.fromEntries(folders.map((folder) => [folder.id, folder.collapsed]))),
  );
}

function folderPayload(folders: ServerFolder[]): api.ServerFolderData[] {
  return folders.map((folder) => ({
    id: folder.id,
    name: folder.name,
    color: folder.color ?? null,
    server_ids: folder.serverIds,
    collapsed: folder.collapsed,
  }));
}

export const useUiStore = create<UiState>((set, get) => {
  const persistFolders = (folders: ServerFolder[]) => {
    if (!folderAccountId) return;
    pendingFolders = folders.map((folder) => ({ ...folder, serverIds: [...folder.serverIds] }));
    if (folderSaveRunning) return;
    folderSaveRunning = true;
    const generation = folderGeneration;
    const accountId = folderAccountId;
    void (async () => {
      while (pendingFolders && generation === folderGeneration && accountId === folderAccountId) {
        const saving = pendingFolders;
        pendingFolders = null;
        set({ folderSyncStatus: 'saving', folderSyncError: null });
        try {
          await api.replaceServerFolders(folderPayload(saving));
        } catch (error) {
          if (generation === folderGeneration && accountId === folderAccountId) {
            pendingFolders ??= saving;
            set({ folderSyncStatus: 'error', folderSyncError: String(error) });
          }
          break;
        }
      }
      folderSaveRunning = false;
      if (generation === folderGeneration && accountId === folderAccountId && !pendingFolders) {
        set({ folderSyncStatus: 'idle', folderSyncError: null });
      }
    })();
  };

  const setStructuralFolders = (folders: ServerFolder[]) => {
    saveCollapsed(folders);
    set((state) => ({ serverFolders: folders, folderRevision: state.folderRevision + 1 }));
    persistFolders(folders);
  };

  return ({
  activeServer: null,
  activeChannel: null,
  activeDirectConversation: null,
  showMemberList: true,
  showSettings: false,
  showServerSettings: false,
  collapsedCategories: {},
  serverFolders: [],
  folderRevision: 0,
  folderSyncStatus: 'idle',
  folderSyncError: null,
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
  jumpToMessageId: null,

  setActiveServer: (serverId) => set({ activeServer: serverId, activeChannel: null, activeDirectConversation: null }),
  setActiveChannel: (channel) => set({ activeChannel: channel, activeDirectConversation: null }),
  setActiveDirectConversation: (conversationId) => set({
    activeDirectConversation: conversationId,
    activeServer: null,
    activeChannel: null,
  }),
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
    setStructuralFolders(folders);
  },

  hydrateServerFolders: (accountId, folders = []) => {
    folderAccountId = accountId;
    folderGeneration += 1;
    pendingFolders = null;
    if (!accountId) {
      set({ serverFolders: [], folderRevision: 0, folderSyncStatus: 'idle', folderSyncError: null });
      return;
    }
    const collapsed = loadCollapsed(accountId);
    set({
      serverFolders: folders.map((folder) => ({
        ...folder,
        collapsed: collapsed[folder.id] ?? folder.collapsed,
      })),
      folderRevision: 0,
      folderSyncStatus: 'idle',
      folderSyncError: null,
    });
  },

  addServerFolder: (name, serverIds) => {
    const s = get();
      const folder: ServerFolder = {
        id: crypto.randomUUID(),
        name,
        serverIds,
        collapsed: false,
      };
      const updated = [...s.serverFolders, folder];
      setStructuralFolders(updated);
  },

  removeServerFolder: (folderId) => {
    setStructuralFolders(get().serverFolders.filter((folder) => folder.id !== folderId));
  },

  toggleServerFolder: (folderId) =>
    set((s) => {
      const updated = s.serverFolders.map((f) =>
        f.id === folderId ? { ...f, collapsed: !f.collapsed } : f,
      );
      saveCollapsed(updated);
      return { serverFolders: updated };
    }),

  retryServerFolderSync: () => persistFolders(get().serverFolders),

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
  setJumpToMessageId: (messageId) => set({ jumpToMessageId: messageId }),
  });
});
