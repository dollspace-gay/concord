import { create } from 'zustand';
import type { AttachmentInfo, ReplyInfo } from '../api/types';

export interface FailedComposition {
  id: string;
  accountId: string;
  serverId: string | null;
  channel: string;
  conversationId?: string;
  recipient?: string;
  key: string;
  content: string;
  attachments: AttachmentInfo[];
  replyTo: ReplyInfo | null;
  error: string;
}

export interface PendingUploadState {
  uploading: boolean;
  progress: number;
  error: string | null;
  voiceRetryBlob: Blob | null;
  controllers: AbortController[];
}

interface ComposerState {
  drafts: Record<string, string>;
  compositionFiles: Record<string, File[]>;
  failedCompositions: FailedComposition[];
  replyingTo: ReplyInfo | null;
  replies: Record<string, ReplyInfo>;
  uploads: Record<string, PendingUploadState>;
  setDraft: (key: string, text: string) => void;
  setCompositionFiles: (key: string, files: File[]) => void;
  setReplyingTo: (reply: ReplyInfo | null) => void;
  setReplyFor: (key: string, reply: ReplyInfo | null) => void;
  setUploadState: (key: string, state: Partial<PendingUploadState>) => void;
  clearUploadState: (key: string) => void;
  clearReplies: () => void;
  setFailedCompositions: (failed: FailedComposition[]) => void;
  replaceState: (state: Pick<ComposerState, 'drafts' | 'compositionFiles' | 'failedCompositions' | 'replyingTo'>) => void;
  removeChannelKeys: (removed: Set<string>) => void;
}

export const useComposerStore = create<ComposerState>((set) => ({
  drafts: {},
  compositionFiles: {},
  failedCompositions: [],
  replyingTo: null,
  replies: {},
  uploads: {},
  setDraft: (key, text) => set((state) => ({
    drafts: text
      ? { ...state.drafts, [key]: text }
      : Object.fromEntries(Object.entries(state.drafts).filter(([draftKey]) => draftKey !== key)),
  })),
  setCompositionFiles: (key, files) => set((state) => ({
    compositionFiles: files.length
      ? { ...state.compositionFiles, [key]: files }
      : Object.fromEntries(Object.entries(state.compositionFiles).filter(([entry]) => entry !== key)),
  })),
  setReplyingTo: (replyingTo) => set({ replyingTo }),
  setReplyFor: (key, reply) => set((state) => ({
    replies: reply
      ? { ...state.replies, [key]: reply }
      : Object.fromEntries(Object.entries(state.replies).filter(([entry]) => entry !== key)),
  })),
  setUploadState: (key, update) => set((state) => ({
    uploads: {
      ...state.uploads,
      [key]: state.uploads[key]
        ? { ...state.uploads[key], ...update }
        : {
            uploading: update.uploading ?? false,
            progress: update.progress ?? 0,
            error: update.error ?? null,
            voiceRetryBlob: update.voiceRetryBlob ?? null,
            controllers: update.controllers ?? [],
          },
    },
  })),
  clearUploadState: (key) => set((state) => ({
    uploads: Object.fromEntries(Object.entries(state.uploads).filter(([entry]) => entry !== key)),
  })),
  clearReplies: () => set({ replies: {}, replyingTo: null }),
  setFailedCompositions: (failedCompositions) => set({ failedCompositions }),
  replaceState: ({ drafts, compositionFiles, failedCompositions, replyingTo }) => set((state) => {
    Object.values(state.uploads).flatMap((upload) => upload.controllers).forEach((controller) => controller.abort());
    return { drafts, compositionFiles, failedCompositions, replyingTo, replies: {}, uploads: {} };
  }),
  removeChannelKeys: (removed) => set((state) => ({
    drafts: Object.fromEntries(Object.entries(state.drafts).filter(([key]) =>
      !key.split(':').some((segment) => removed.has(segment)))),
    compositionFiles: Object.fromEntries(Object.entries(state.compositionFiles).filter(([key]) =>
      !key.split(':').some((segment) => removed.has(segment)))),
    failedCompositions: state.failedCompositions.filter((failed) =>
      !failed.key.split(':').some((segment) => removed.has(segment))),
    replies: Object.fromEntries(Object.entries(state.replies).filter(([key]) =>
      !key.split(':').some((segment) => removed.has(segment)))),
    uploads: Object.fromEntries(Object.entries(state.uploads).filter(([key]) =>
      !key.split(':').some((segment) => removed.has(segment)))),
  })),
}));
