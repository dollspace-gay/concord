import { useCallback, useEffect, useRef, useState } from 'react';

export function useLifecycleFeedback(scope: string) {
  const [pendingKey, setPendingKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const generation = useRef(0);

  const [previousScope, setPreviousScope] = useState(scope);
  if (previousScope !== scope) {
    setPreviousScope(scope);
    setPendingKey(null);
    setError(null);
    setSuccess(null);
  }

  useEffect(() => {
    generation.current += 1;
    return () => { generation.current += 1; };
  }, [scope]);

  const run = useCallback(async (
    key: string,
    action: () => Promise<void>,
    successMessage: string,
    afterSuccess?: () => void,
  ) => {
    const started = generation.current;
    setPendingKey(key);
    setError(null);
    setSuccess(null);
    try {
      await action();
      if (generation.current !== started) return;
      afterSuccess?.();
      setSuccess(successMessage);
    } catch (reason) {
      if (generation.current !== started) return;
      setError(reason instanceof Error ? reason.message : 'The action could not be completed.');
    } finally {
      if (generation.current === started) setPendingKey(null);
    }
  }, []);

  return { pendingKey, error, success, run };
}
