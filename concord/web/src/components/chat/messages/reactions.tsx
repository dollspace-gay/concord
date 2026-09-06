import { useChatStore } from '../../../stores/chatStore';

export function ReactionBadge({
  emoji,
  count,
  messageId,
  userIds,
}: {
  emoji: string;
  count: number;
  messageId: string;
  userIds: string[];
}) {
  const activeAccountId = useChatStore((s) => s.activeAccountId);
  const addReaction = useChatStore((s) => s.addReaction);
  const removeReaction = useChatStore((s) => s.removeReaction);

  const hasReacted = userIds.includes(activeAccountId || '') || userIds.includes('__self__');

  const handleClick = () => {
    if (hasReacted) {
      removeReaction(messageId, emoji);
    } else {
      addReaction(messageId, emoji);
    }
  };

  return (
    <button
      onClick={handleClick}
      aria-label={`${hasReacted ? 'Remove' : 'Add'} ${emoji} reaction, ${count}`}
      className={`flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs transition-colors ${hasReacted
          ? 'border-blue-500/50 bg-blue-500/10 text-text-primary'
          : 'border-border bg-bg-secondary text-text-muted hover:bg-bg-hover'
        }`}
    >
      <span>{emoji}</span>
      <span>{count}</span>
    </button>
  );
}
