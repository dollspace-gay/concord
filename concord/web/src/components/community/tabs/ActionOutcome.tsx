

export function ActionOutcome({ outcome }: { outcome: { kind: 'success' | 'error'; message: string } | null }) {
  if (!outcome) return null;
  return <p role={outcome.kind === 'error' ? 'alert' : 'status'} className={`text-sm ${outcome.kind === 'error' ? 'text-red-400' : 'text-green-400'}`}>{outcome.message}</p>;
}
