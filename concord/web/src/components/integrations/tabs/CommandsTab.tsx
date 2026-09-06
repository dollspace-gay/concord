import { useState } from 'react';
import type { SlashCommandInfo } from '../../../api/types';

// ── Commands Tab ──

export function CommandsTab({ commands, onDelete }: {
  commands: SlashCommandInfo[];
  onDelete: (commandId: string) => Promise<void>;
}) {
  return (
    <div className="space-y-4">
      <h3 className="text-sm font-semibold text-text-secondary">Slash Commands</h3>
      <p className="text-xs text-text-muted">
        Commands are registered by bots. Use the Bot API to register slash commands.
      </p>

      {commands.length === 0 ? (
        <p className="text-text-muted text-sm">No slash commands registered.</p>
      ) : (
        <div className="space-y-2">
          {commands.map(cmd => <CommandCard key={cmd.id} command={cmd} onDelete={onDelete} />)}
        </div>
      )}
    </div>
  );
}

export function CommandCard({ command, onDelete }: {
  command: SlashCommandInfo;
  onDelete: (commandId: string) => Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remove = async () => {
    setPending(true);
    setError(null);
    try {
      await onDelete(command.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Command deletion failed.');
    } finally {
      setPending(false);
    }
  };
  return (
    <div className="rounded bg-bg-secondary p-3">
      <div className="flex items-center justify-between">
        <div className="min-w-0 flex-1">
          <span className="font-medium text-text-primary text-sm">/{command.name}</span>
          <p className="text-xs text-text-muted mt-0.5">{command.description || 'No description'}</p>
          {command.options.length > 0 && (
            <div className="mt-1 flex gap-1 flex-wrap">
              {command.options.map(opt => (
                <span key={opt.name} className="rounded bg-bg-tertiary px-1.5 py-0.5 text-xs text-text-muted">
                  {opt.name}{opt.required ? '*' : ''}
                </span>
              ))}
            </div>
          )}
        </div>
        <button disabled={pending} onClick={() => void remove()} className="ml-2 text-red-400 hover:text-red-300 text-xs disabled:opacity-50">
          {pending ? 'Deleting…' : 'Delete'}
        </button>
      </div>
      {error && <p role="alert" className="mt-2 text-xs text-red-400">{error}</p>}
    </div>
  );
}
