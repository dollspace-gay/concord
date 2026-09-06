import { useState } from 'react';
import type { ServerCommunityInfo, TemplateInfo } from '../../../api/types';
import { ActionOutcome } from './ActionOutcome';
import { useActionStatus } from './useActionStatus';

// ── Settings Tab ────────────────────────────────────────

export function SettingsTab({ serverId, settings, templates, onUpdate, onCreateTemplate, onDeleteTemplate, onInstantiateTemplate }: {
  serverId: string;
  settings?: ServerCommunityInfo;
  templates: TemplateInfo[];
  onUpdate: (serverId: string, settings: { description?: string; isDiscoverable: boolean; welcomeMessage?: string; rulesText?: string; category?: string }) => Promise<void>;
  onCreateTemplate: (serverId: string, name: string, description?: string) => Promise<void>;
  onDeleteTemplate: (serverId: string, templateId: string) => Promise<void>;
  onInstantiateTemplate: (templateId: string, serverName: string) => Promise<void>;
}) {
  const [description, setDescription] = useState(settings?.description ?? '');
  const [isDiscoverable, setIsDiscoverable] = useState(settings?.is_discoverable ?? false);
  const [welcomeMessage, setWelcomeMessage] = useState(settings?.welcome_message ?? '');
  const [rulesText, setRulesText] = useState(settings?.rules_text ?? '');
  const [category, setCategory] = useState(settings?.category ?? '');
  const [templateName, setTemplateName] = useState('');
  const [templateDesc, setTemplateDesc] = useState('');
  const [showTemplateForm, setShowTemplateForm] = useState(false);
  const [templateServerNames, setTemplateServerNames] = useState<Record<string, string>>({});
  const { pending, outcome, run } = useActionStatus();

  // Sync form when settings load (render-time adjustment per React docs)
  const [prevSettings, setPrevSettings] = useState(settings);
  if (settings && settings !== prevSettings) {
    setPrevSettings(settings);
    setDescription(settings.description ?? '');
    setIsDiscoverable(settings.is_discoverable);
    setWelcomeMessage(settings.welcome_message ?? '');
    setRulesText(settings.rules_text ?? '');
    setCategory(settings.category ?? '');
  }

  const handleSave = () => {
    void run('settings', () => onUpdate(serverId, {
      description: description || undefined,
      isDiscoverable,
      welcomeMessage: welcomeMessage || undefined,
      rulesText: rulesText || undefined,
      category: category || undefined,
    }), 'Community settings saved.');
  };

  const handleCreateTemplate = () => {
    if (!templateName.trim()) return;
    void run('create-template', () => onCreateTemplate(serverId, templateName.trim(), templateDesc.trim() || undefined), 'Template created.', () => {
      setTemplateName('');
      setTemplateDesc('');
      setShowTemplateForm(false);
    });
  };

  return (
    <div className="space-y-6">
      {/* Community Settings */}
      <div className="space-y-3">
        <h3 className="text-sm font-semibold text-text-secondary">Community Settings</h3>

        <div>
          <label htmlFor="community-description" className="block text-xs font-medium text-text-muted mb-1">Server Description</label>
          <textarea
            id="community-description"
            value={description}
            onChange={e => setDescription(e.target.value)}
            placeholder="Tell people about your server..."
            rows={2}
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
          />
        </div>

        <div className="flex items-center gap-3">
          <label className="flex items-center gap-2 text-sm text-text-secondary cursor-pointer">
            <input
              type="checkbox"
              checked={isDiscoverable}
              onChange={e => setIsDiscoverable(e.target.checked)}
              className="rounded"
            />
            Discoverable
          </label>
          <span className="text-xs text-text-muted">Allow this server to appear in Server Discovery</span>
        </div>

        <div>
          <label htmlFor="community-welcome" className="block text-xs font-medium text-text-muted mb-1">Welcome Message</label>
          <textarea
            id="community-welcome"
            value={welcomeMessage}
            onChange={e => setWelcomeMessage(e.target.value)}
            placeholder="Welcome new members with a message..."
            rows={2}
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
          />
        </div>

        <div>
          <label htmlFor="community-rules" className="block text-xs font-medium text-text-muted mb-1">Server Rules</label>
          <textarea
            id="community-rules"
            value={rulesText}
            onChange={e => setRulesText(e.target.value)}
            placeholder="Define rules that members must accept..."
            rows={3}
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
          />
        </div>

        <div>
          <label htmlFor="community-category" className="block text-xs font-medium text-text-muted mb-1">Category</label>
          <select
            id="community-category"
            value={category}
            onChange={e => setCategory(e.target.value)}
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
          >
            <option value="">None</option>
            <option value="gaming">Gaming</option>
            <option value="music">Music</option>
            <option value="education">Education</option>
            <option value="science">Science & Technology</option>
            <option value="entertainment">Entertainment</option>
            <option value="community">General Community</option>
          </select>
        </div>

        <button
          disabled={pending !== null}
          onClick={handleSave}
          className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          {pending === 'settings' ? 'Saving…' : 'Save Settings'}
        </button>
      </div>

      {/* Templates */}
      <div className="space-y-3 border-t border-border pt-4">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text-secondary">Server Templates</h3>
          <button
            disabled={pending !== null}
            onClick={() => setShowTemplateForm(!showTemplateForm)}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
          >
            {showTemplateForm ? 'Cancel' : 'Create Template'}
          </button>
        </div>

        {showTemplateForm && (
          <div className="rounded bg-bg-secondary p-3 space-y-3">
            <div>
              <label htmlFor="community-template-name" className="block text-xs font-medium text-text-muted mb-1">Template Name *</label>
              <input
                id="community-template-name"
                type="text"
                value={templateName}
                onChange={e => setTemplateName(e.target.value)}
                placeholder="My Server Template"
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
            <div>
              <label htmlFor="community-template-description" className="block text-xs font-medium text-text-muted mb-1">Description</label>
              <input
                id="community-template-description"
                type="text"
                value={templateDesc}
                onChange={e => setTemplateDesc(e.target.value)}
                placeholder="What's this template for?"
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
            <button
              onClick={handleCreateTemplate}
              disabled={!templateName.trim() || pending !== null}
              className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {pending === 'create-template' ? 'Creating…' : 'Create Template'}
            </button>
          </div>
        )}

        {templates.length === 0 ? (
          <p className="text-text-muted text-sm">No templates created.</p>
        ) : (
          <div className="space-y-2">
            {templates.map(tpl => (
              <div key={tpl.id} className="rounded bg-bg-secondary p-3 space-y-2">
                <div className="flex items-center justify-between">
                  <div>
                    <span className="text-sm font-medium text-text-primary">{tpl.name}</span>
                    {tpl.description && (
                      <p className="text-xs text-text-muted">{tpl.description}</p>
                    )}
                    <p className="text-xs text-text-muted">
                      Used {tpl.use_count} times | Created {new Date(tpl.created_at).toLocaleDateString()}
                    </p>
                  </div>
                  <button
                    disabled={pending !== null}
                    onClick={() => run(`delete-template:${tpl.id}`, () => onDeleteTemplate(serverId, tpl.id), 'Template deleted.')}
                    className="rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700"
                  >
                    {pending === `delete-template:${tpl.id}` ? 'Deleting…' : 'Delete'}
                  </button>
                </div>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={templateServerNames[tpl.id] ?? ''}
                    onChange={e => setTemplateServerNames(names => ({ ...names, [tpl.id]: e.target.value }))}
                    placeholder="New server name"
                    aria-label={`New server name for ${tpl.name}`}
                    className="min-w-0 flex-1 rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
                  />
                  <button
                    onClick={() => {
                      const name = (templateServerNames[tpl.id] ?? '').trim();
                      if (!name) return;
                      void run(`instantiate:${tpl.id}`, () => onInstantiateTemplate(tpl.id, name), 'Server created from template.', () => setTemplateServerNames(names => ({ ...names, [tpl.id]: '' })));
                    }}
                    disabled={!(templateServerNames[tpl.id] ?? '').trim() || pending !== null}
                    className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {pending === `instantiate:${tpl.id}` ? 'Creating…' : 'Create Server'}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
      <ActionOutcome outcome={outcome} />
    </div>
  );
}
