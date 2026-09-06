import { useState } from 'react';
import type { MessageComponent } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';

export function MessageComponents({ messageId, components }: { messageId: string; components: MessageComponent[] }) {
  return (
    <div className="mt-2 flex flex-col gap-2" aria-label="Message actions">
      {components.map((component, index) => (
        <MessageComponentControl key={`${messageId}-component-${index}`} messageId={messageId} component={component} />
      ))}
    </div>
  );
}

export function MessageComponentControl({ messageId, component }: { messageId: string; component: MessageComponent }) {
  const invoke = useChatStore((state) => state.invokeMessageComponent);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const invokeOnce = async (values: string[] = []) => {
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      await invoke(messageId, component.type === 'action_row' ? '' : component.custom_id, values);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Action was not accepted.');
    } finally {
      setPending(false);
    }
  };
  if (component.type === 'action_row') {
    return (
      <div className="flex flex-wrap items-center gap-2">
        {component.components.map((child, index) => (
          <MessageComponentControl key={`${messageId}-row-${index}`} messageId={messageId} component={child} />
        ))}
      </div>
    );
  }
  if (component.type === 'button') {
    const styles: Record<string, string> = {
      primary: 'bg-blue-600 text-white hover:bg-blue-500',
      secondary: 'bg-bg-secondary text-text-primary hover:bg-bg-hover',
      success: 'bg-green-700 text-white hover:bg-green-600',
      danger: 'bg-red-700 text-white hover:bg-red-600',
    };
    return (
      <div>
        <button
          type="button"
          disabled={component.disabled || pending}
          onClick={() => { void invokeOnce(); }}
          className={`rounded border border-border px-3 py-1.5 text-sm font-medium disabled:cursor-not-allowed disabled:opacity-50 ${styles[component.style || 'primary'] || styles.primary}`}
        >
          {component.emoji && <span aria-hidden="true" className="mr-1">{component.emoji}</span>}
          {component.label}
        </button>
        {error && <div role="alert" className="mt-1 text-xs text-red-400">{error}</div>}
      </div>
    );
  }
  return (
    <label className="min-w-52 max-w-sm text-xs text-text-muted">
      <span className="sr-only">{component.placeholder || 'Select an option'}</span>
      <select
        aria-label={component.placeholder || 'Select an option'}
        multiple={(component.max_values ?? 1) > 1}
        disabled={pending}
        defaultValue={(component.max_values ?? 1) > 1
          ? component.options.filter((option) => option.default).map((option) => option.value)
          : component.options.find((option) => option.default)?.value ?? ((component.min_values ?? 1) === 0 ? '' : undefined)}
        onChange={(event) => { void invokeOnce(Array.from(event.currentTarget.selectedOptions, (option) => option.value).filter(Boolean)); }}
        className="w-full rounded border border-border bg-bg-input px-2 py-1.5 text-sm text-text-primary"
      >
        {(component.min_values ?? 1) === 0 && <option value="">{component.placeholder || 'None'}</option>}
        {component.options.map((option) => (
          <option key={option.value} value={option.value} title={option.description || undefined}>
            {option.emoji ? `${option.emoji} ` : ''}{option.label}
          </option>
        ))}
      </select>
      {error && <span role="alert" className="mt-1 block text-xs text-red-400">{error}</span>}
    </label>
  );
}
