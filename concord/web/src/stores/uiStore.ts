import { create } from 'zustand';

interface UiState {
  activeChannel: string | null;
  showMemberList: boolean;
  showSettings: boolean;

  setActiveChannel: (channel: string | null) => void;
  toggleMemberList: () => void;
  setShowSettings: (show: boolean) => void;
}

export const useUiStore = create<UiState>((set) => ({
  activeChannel: null,
  showMemberList: true,
  showSettings: false,

  setActiveChannel: (channel) => set({ activeChannel: channel }),
  toggleMemberList: () => set((s) => ({ showMemberList: !s.showMemberList })),
  setShowSettings: (show) => set({ showSettings: show }),
}));
