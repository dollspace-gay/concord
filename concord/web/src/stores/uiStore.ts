import { create } from 'zustand';

interface UiState {
  activeServer: string | null;
  activeChannel: string | null;
  showMemberList: boolean;
  showSettings: boolean;

  setActiveServer: (serverId: string | null) => void;
  setActiveChannel: (channel: string | null) => void;
  toggleMemberList: () => void;
  setShowSettings: (show: boolean) => void;
}

export const useUiStore = create<UiState>((set) => ({
  activeServer: null,
  activeChannel: null,
  showMemberList: true,
  showSettings: false,

  setActiveServer: (serverId) => set({ activeServer: serverId, activeChannel: null }),
  setActiveChannel: (channel) => set({ activeChannel: channel }),
  toggleMemberList: () => set((s) => ({ showMemberList: !s.showMemberList })),
  setShowSettings: (show) => set({ showSettings: show }),
}));
