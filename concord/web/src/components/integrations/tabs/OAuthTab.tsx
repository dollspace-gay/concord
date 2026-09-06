import { useState } from 'react';
import { Feedback } from './Feedback';
import { useLifecycleFeedback } from './useLifecycleFeedback';

// ── OAuth Apps Tab ──

export function OAuthTab({ apps, onCreate, onDelete }: {
  apps: { id: string; name: string; description: string; scopes: string; is_public: boolean; created_at: string }[];
  onCreate: (name: string, description: string, redirectUris: string, clientType: 'confidential' | 'public') => Promise<void>;
  onDelete: (appId: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [redirectUris, setRedirectUris] = useState('');
  const [clientType, setClientType] = useState<'confidential' | 'public'>('confidential');
  const feedback = useLifecycleFeedback('oauth-apps');

  const handleCreate = async () => {
    if (!name.trim() || !redirectUris.trim()) return;
    await feedback.run(
      'create',
      () => onCreate(name.trim(), description.trim(), redirectUris.trim(), clientType),
      'OAuth application created.',
      () => {
        setName('');
        setDescription('');
        setRedirectUris('');
        setShowForm(false);
      },
    );
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">OAuth2 Applications</h3>
        <button
          disabled={feedback.pendingKey !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {showForm ? 'Cancel' : 'Create App'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="App name"
            value={name}
            onChange={e => setName(e.target.value)}
          />
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Description"
            value={description}
            onChange={e => setDescription(e.target.value)}
          />
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Redirect URIs (comma separated)"
            value={redirectUris}
            onChange={e => setRedirectUris(e.target.value)}
          />
          <select
            aria-label="OAuth client type"
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none"
            value={clientType}
            onChange={e => setClientType(e.target.value as 'confidential' | 'public')}
          >
            <option value="confidential">Confidential client (server-side)</option>
            <option value="public">Public client (browser or native app)</option>
          </select>
          <p className="text-xs text-text-muted">Available scopes: identify, servers.read</p>
          <button
            disabled={feedback.pendingKey !== null}
            onClick={() => void handleCreate()}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {feedback.pendingKey === 'create' ? 'Creating…' : 'Create App'}
          </button>
        </div>
      )}

      <Feedback error={feedback.error} success={feedback.success} />

      {apps.length === 0 ? (
        <p className="text-text-muted text-sm">No OAuth2 applications.</p>
      ) : (
        <div className="space-y-2">
          {apps.map(app => (
            <div key={app.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-text-primary text-sm">{app.name}</span>
                  {app.is_public && <span className="rounded bg-green-900/30 px-1.5 py-0.5 text-xs text-green-400">Public</span>}
                </div>
                <p className="text-xs text-text-muted mt-0.5">{app.description || 'No description'}</p>
                <div className="text-xs text-text-muted mt-0.5">
                  Client ID: <code className="bg-bg-tertiary px-1 rounded">{app.id}</code>
                </div>
              </div>
              <button
                disabled={feedback.pendingKey !== null}
                onClick={() => void feedback.run(
                  `delete:${app.id}`,
                  () => onDelete(app.id),
                  'OAuth application deleted.',
                )}
                className="ml-2 text-red-400 hover:text-red-300 text-xs disabled:opacity-50"
              >
                {feedback.pendingKey === `delete:${app.id}` ? 'Deleting…' : 'Delete'}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
