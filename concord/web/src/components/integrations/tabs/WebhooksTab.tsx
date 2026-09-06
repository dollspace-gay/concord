import { useCallback, useEffect, useState } from 'react';
import type { WebhookDeliveryStatus, WebhookInfo } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';
import { Feedback } from './Feedback';
import { useLifecycleFeedback } from './useLifecycleFeedback';

// ── Webhooks Tab ──

export function WebhooksTab({ webhooks, serverId, channels, onCreate, onDelete }: {
  webhooks: WebhookInfo[];
  serverId: string;
  channels: { id: string; name: string }[];
  onCreate: (channelId: string, name: string, webhookType: string, url?: string) => Promise<void>;
  onDelete: (webhookId: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [channelId, setChannelId] = useState('');
  const [webhookType, setWebhookType] = useState<'incoming' | 'outgoing'>('incoming');
  const [url, setUrl] = useState('');
  const feedback = useLifecycleFeedback(serverId);

  const handleCreate = async () => {
    if (!name.trim() || !channelId) return;
    await feedback.run(
      'create',
      () => onCreate(channelId, name.trim(), webhookType, webhookType === 'outgoing' ? url.trim() : undefined),
      'Webhook created.',
      () => {
        setName('');
        setChannelId('');
        setWebhookType('incoming');
        setUrl('');
        setShowForm(false);
      },
    );
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Webhooks</h3>
        <button
          disabled={feedback.pendingKey !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {showForm ? 'Cancel' : 'Create Webhook'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Webhook name"
            value={name}
            onChange={e => setName(e.target.value)}
          />
          <select
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none"
            value={channelId}
            onChange={e => setChannelId(e.target.value)}
          >
            <option value="">Select channel...</option>
            {channels.map(ch => (
              <option key={ch.id} value={ch.id}>{ch.name}</option>
            ))}
          </select>
          <select
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none"
            value={webhookType}
            onChange={e => setWebhookType(e.target.value as 'incoming' | 'outgoing')}
          >
            <option value="incoming">Incoming</option>
            <option value="outgoing">Outgoing</option>
          </select>
          {webhookType === 'outgoing' && (
            <input
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
              placeholder="Outgoing URL"
              value={url}
              onChange={e => setUrl(e.target.value)}
            />
          )}
          <button
            disabled={feedback.pendingKey !== null}
            onClick={() => void handleCreate()}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {feedback.pendingKey === 'create' ? 'Creating…' : 'Create'}
          </button>
        </div>
      )}

      <Feedback error={feedback.error} success={feedback.success} />

      {webhooks.length === 0 ? (
        <p className="text-text-muted text-sm">No webhooks configured.</p>
      ) : (
        <div className="space-y-2">
          {webhooks.map(wh => <WebhookCard key={wh.id} webhook={wh} onDelete={onDelete} />)}
        </div>
      )}
    </div>
  );
}

export function WebhookCard({ webhook, onDelete }: { webhook: WebhookInfo; onDelete: (id: string) => Promise<void> }) {
  const [deliveries, setDeliveries] = useState<WebhookDeliveryStatus[]>([]);
  const [loading, setLoading] = useState(webhook.webhook_type === 'outgoing');
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const outgoing = webhook.webhook_type === 'outgoing';

  const refresh = useCallback((signal?: AbortSignal) => {
    if (!outgoing) return Promise.resolve();
    const generation = useChatStore.getState().accountGeneration;
    return fetch(`/api/webhooks/${encodeURIComponent(webhook.id)}/deliveries?limit=10`, { signal })
      .then(async (response) => {
        if (!response.ok) throw new Error('Delivery status is unavailable');
        return await response.json() as { deliveries: WebhookDeliveryStatus[] };
      })
      .then((body) => {
        if (signal?.aborted || useChatStore.getState().accountGeneration !== generation) return;
        setDeliveries(body.deliveries);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (signal?.aborted || useChatStore.getState().accountGeneration !== generation) return;
        setError(reason instanceof Error ? reason.message : 'Delivery status is unavailable');
      })
      .finally(() => {
        if (!signal?.aborted && useChatStore.getState().accountGeneration === generation) setLoading(false);
      });
  }, [outgoing, webhook.id]);

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh]);

  const test = async () => {
    const generation = useChatStore.getState().accountGeneration;
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`/api/webhooks/${encodeURIComponent(webhook.id)}/test`, { method: 'POST' });
      if (!response.ok) throw new Error('Test delivery could not be queued.');
      await refresh();
    } catch (reason) {
      if (useChatStore.getState().accountGeneration !== generation) return;
      setError(reason instanceof Error && reason.message === 'Test delivery could not be queued.'
        ? reason.message
        : 'The test delivery result is unknown; refresh status before retrying.');
    } finally {
      if (useChatStore.getState().accountGeneration === generation) setLoading(false);
    }
  };

  const retry = async (deliveryId: string) => {
    const generation = useChatStore.getState().accountGeneration;
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`/api/webhook-deliveries/${encodeURIComponent(deliveryId)}/retry`, { method: 'POST' });
      if (!response.ok) throw new Error('Delivery could not be retried.');
      await refresh();
    } catch (reason) {
      if (useChatStore.getState().accountGeneration !== generation) return;
      setError(reason instanceof Error && reason.message === 'Delivery could not be retried.'
        ? reason.message
        : 'The retry result is unknown; refresh status before retrying again.');
    } finally {
      if (useChatStore.getState().accountGeneration === generation) setLoading(false);
    }
  };

  const remove = async () => {
    setDeleting(true);
    setError(null);
    try {
      await onDelete(webhook.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Webhook deletion failed.');
    } finally {
      setDeleting(false);
    }
  };

  const copyIncomingUrl = async (incomingUrl: string) => {
    try {
      await navigator.clipboard.writeText(incomingUrl);
      setCopyStatus('Webhook URL copied.');
    } catch {
      setCopyStatus('Clipboard access failed. Select and copy the URL manually.');
    }
  };

  const incomingUrl = webhook.token
    ? `${window.location.origin}/api/webhooks/${webhook.id}/${webhook.token}`
    : null;
  return (
    <div className="rounded bg-bg-secondary p-3 space-y-2">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 min-w-0">
          <span className="font-medium text-text-primary text-sm truncate">{webhook.name}</span>
          <span className={`rounded px-1.5 py-0.5 text-xs ${outgoing ? 'bg-blue-900/30 text-blue-400' : 'bg-green-900/30 text-green-400'}`}>
            {webhook.webhook_type}
          </span>
        </div>
        <div className="flex gap-2">
          {outgoing && <button disabled={loading} onClick={() => void test()} className="text-xs text-blue-400 disabled:opacity-50">Test</button>}
          <button disabled={deleting} onClick={() => void remove()} className="text-red-400 hover:text-red-300 text-xs disabled:opacity-50">
            {deleting ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
      {!outgoing && incomingUrl && (
        <div className="text-xs text-amber-300">
          Credential shown once. <button className="underline" onClick={() => void copyIncomingUrl(incomingUrl)}>Copy webhook URL</button>
          {copyStatus && <div role={copyStatus.startsWith('Clipboard') ? 'alert' : 'status'}>{copyStatus}</div>}
        </div>
      )}
      {!outgoing && !incomingUrl && <div className="text-xs text-text-muted">Credential hidden. Create a new webhook to rotate it.</div>}
      {outgoing && webhook.token && (
        <div className="text-xs text-amber-300">
          <div>Signing secret shown once. Save it before closing this dialog.</div>
          <code className="mt-1 block break-all rounded bg-bg-tertiary p-2 text-text-primary select-all">{webhook.token}</code>
        </div>
      )}
      {outgoing && !webhook.token && <div className="text-xs text-text-muted">Signing secret hidden. Create a new webhook to rotate it.</div>}
      {outgoing && (
        <div className="space-y-1 text-xs">
          {error && <div className="text-red-400">{error}</div>}
          {!error && deliveries.length === 0 && <div className="text-text-muted">No deliveries yet.</div>}
          {deliveries.map(delivery => (
            <div key={delivery.delivery_id} className="flex justify-between gap-2 text-text-muted">
              <span>{delivery.event_type} · {delivery.state} · attempt {delivery.attempt_count}</span>
              {delivery.state === 'failed' && <button className="text-blue-400" disabled={loading} onClick={() => void retry(delivery.delivery_id)}>Retry</button>}
            </div>
          ))}
          <button className="text-blue-400 disabled:opacity-50" disabled={loading} onClick={() => { setLoading(true); void refresh(); }}>{loading ? 'Loading…' : 'Refresh status'}</button>
        </div>
      )}
    </div>
  );
}
