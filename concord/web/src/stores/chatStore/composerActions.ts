import { useComposerStore } from '../composerStore';
import type { ChatState, ChatStoreContext } from './types';

export function createComposerActions({ get }: ChatStoreContext): Pick<ChatState, 'setDraft' | 'setCompositionFiles' | 'retryFailedComposition' | 'dismissFailedComposition'> {
  return {
    setDraft: (key, text) => useComposerStore.getState().setDraft(key, text),

    setCompositionFiles: (key, files) => useComposerStore.getState().setCompositionFiles(key, files),

    retryFailedComposition: (id) => {
      const failed = useComposerStore.getState().failedCompositions.find((entry) => entry.id === id);
      if (!failed || failed.accountId !== get().activeAccountId) return false;
      const previousReply = useComposerStore.getState().replies[failed.key] ?? null;
      useComposerStore.getState().setReplyFor(failed.key, failed.replyTo);
      const accepted = failed.conversationId && failed.recipient
        ? get().sendDirectMessage(failed.conversationId, failed.recipient, failed.content, failed.attachments)
        : failed.serverId !== null && get().sendMessage(
          failed.serverId, failed.channel, failed.content, failed.attachments,
        );
      useComposerStore.getState().setReplyFor(failed.key, previousReply);
      if (accepted) {
        useComposerStore.getState().setFailedCompositions(
          useComposerStore.getState().failedCompositions.filter((entry) => entry.id !== id),
        );
      }
      return accepted;
    },

    dismissFailedComposition: (id) => useComposerStore.getState().setFailedCompositions(
      useComposerStore.getState().failedCompositions.filter((entry) => entry.id !== id),
    )
  };
}
