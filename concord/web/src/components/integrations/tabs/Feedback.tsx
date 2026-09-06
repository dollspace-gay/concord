

export function Feedback({ error, success }: { error: string | null; success: string | null }) {
  return (
    <>
      {error && <p role="alert" className="text-xs text-red-400">{error}</p>}
      {success && <p role="status" className="text-xs text-green-400">{success}</p>}
    </>
  );
}
