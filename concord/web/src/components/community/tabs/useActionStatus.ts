import { useState } from 'react';

export function useActionStatus() {
  const [pending, setPending] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ kind: 'success' | 'error'; message: string } | null>(null);
  const run = async (key: string, action: () => Promise<void>, success: string, accepted?: () => void) => {
    if (pending) return;
    setPending(key);
    setOutcome(null);
    try {
      await action();
      accepted?.();
      setOutcome({ kind: 'success', message: success });
    } catch (cause) {
      setOutcome({ kind: 'error', message: cause instanceof Error ? cause.message : 'The action was rejected.' });
    } finally {
      setPending(null);
    }
  };
  return { pending, outcome, run };
}
