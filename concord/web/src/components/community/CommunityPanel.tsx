import { useState, useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import type { InviteInfo, EventInfo, ServerCommunityInfo, TemplateInfo } from '../../api/types';

type Tab = 'invites' | 'events' | 'settings' | 'discovery';

interface Props {
  serverId: string;
  onClose: () => void;
}

export function CommunityPanel({ serverId, onClose }: Props) {
  const [activeTab, setActiveTab] = useState<Tab>('invites');

  const invites = useChatStore(s => s.invites[serverId] ?? []);
  const serverEvents = useChatStore(s => s.serverEvents[serverId] ?? []);
  const communitySettings = useChatStore(s => s.communitySettings[serverId]);
  const discoverableServers = useChatStore(s => s.discoverableServers);
  const templates = useChatStore(s => s.templates[serverId] ?? []);

  const listInvites = useChatStore(s => s.listInvites);
  const createInvite = useChatStore(s => s.createInvite);
  const deleteInvite = useChatStore(s => s.deleteInvite);
  const listEvents = useChatStore(s => s.listEvents);
  const createEvent = useChatStore(s => s.createEvent);
  const deleteEvent = useChatStore(s => s.deleteEvent);
  const setRsvp = useChatStore(s => s.setRsvp);
  const updateCommunitySettings = useChatStore(s => s.updateCommunitySettings);
  const getCommunitySettings = useChatStore(s => s.getCommunitySettings);
  const discoverServers = useChatStore(s => s.discoverServers);
  const useInvite = useChatStore(s => s.useInvite);
  const listTemplates = useChatStore(s => s.listTemplates);
  const createTemplate = useChatStore(s => s.createTemplate);
  const deleteTemplate = useChatStore(s => s.deleteTemplate);

  // Fetch data on mount / tab change
  useEffect(() => {
    if (activeTab === 'invites') listInvites(serverId);
    if (activeTab === 'events') listEvents(serverId);
    if (activeTab === 'settings') {
      getCommunitySettings(serverId);
      listTemplates(serverId);
    }
    if (activeTab === 'discovery') discoverServers();
  }, [serverId, activeTab, listInvites, listEvents, getCommunitySettings, discoverServers, listTemplates]);

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose]);

  const tabLabels: Record<Tab, string> = {
    invites: 'Invites',
    events: 'Events',
    settings: 'Settings',
    discovery: 'Discovery',
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-full max-w-3xl max-h-[85vh] flex flex-col rounded-lg bg-bg-primary shadow-xl" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border p-4">
          <h2 className="text-lg font-bold text-text-primary">Community</h2>
          <button onClick={onClose} className="text-text-muted hover:text-text-primary text-xl leading-none">&times;</button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-border">
          {(Object.keys(tabLabels) as Tab[]).map(t => (
            <button
              key={t}
              onClick={() => setActiveTab(t)}
              className={`px-4 py-2 text-sm font-medium ${
                activeTab === t ? 'border-b-2 border-bg-accent text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
            >
              {tabLabels[t]}
            </button>
          ))}
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-4">
          {activeTab === 'invites' && (
            <InvitesTab
              invites={invites}
              serverId={serverId}
              onCreate={createInvite}
              onDelete={deleteInvite}
            />
          )}
          {activeTab === 'events' && (
            <EventsTab
              events={serverEvents}
              serverId={serverId}
              onCreate={createEvent}
              onDelete={deleteEvent}
              onRsvp={setRsvp}
            />
          )}
          {activeTab === 'settings' && (
            <SettingsTab
              serverId={serverId}
              settings={communitySettings}
              templates={templates}
              onUpdate={updateCommunitySettings}
              onCreateTemplate={createTemplate}
              onDeleteTemplate={deleteTemplate}
            />
          )}
          {activeTab === 'discovery' && (
            <DiscoveryTab
              servers={discoverableServers}
              onJoin={useInvite}
              onRefresh={discoverServers}
            />
          )}
        </div>
      </div>
    </div>
  );
}

// ── Invites Tab ──────────────────────────────────────────

function InvitesTab({ invites, serverId, onCreate, onDelete }: {
  invites: InviteInfo[];
  serverId: string;
  onCreate: (serverId: string, maxUses?: number, expiresAt?: string, channelId?: string) => void;
  onDelete: (serverId: string, inviteId: string) => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [maxUses, setMaxUses] = useState('');
  const [expiresIn, setExpiresIn] = useState('24'); // hours
  const [copied, setCopied] = useState<string | null>(null);

  const handleCreate = () => {
    const mu = maxUses ? parseInt(maxUses, 10) : undefined;
    const hours = parseInt(expiresIn, 10);
    const ea = hours > 0 ? new Date(Date.now() + hours * 3600000).toISOString() : undefined;
    onCreate(serverId, mu, ea);
    setShowForm(false);
    setMaxUses('');
    setExpiresIn('24');
  };

  const copyCode = (code: string) => {
    navigator.clipboard.writeText(code);
    setCopied(code);
    setTimeout(() => setCopied(null), 2000);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Server Invites</h3>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          {showForm ? 'Cancel' : 'Create Invite'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <div>
            <label className="block text-xs font-medium text-text-muted mb-1">Max Uses (0 = unlimited)</label>
            <input
              type="number"
              value={maxUses}
              onChange={e => setMaxUses(e.target.value)}
              placeholder="0"
              min="0"
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-text-muted mb-1">Expires In (hours, 0 = never)</label>
            <select
              value={expiresIn}
              onChange={e => setExpiresIn(e.target.value)}
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            >
              <option value="1">1 hour</option>
              <option value="6">6 hours</option>
              <option value="12">12 hours</option>
              <option value="24">24 hours</option>
              <option value="168">7 days</option>
              <option value="720">30 days</option>
              <option value="0">Never</option>
            </select>
          </div>
          <button
            onClick={handleCreate}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
          >
            Generate Invite
          </button>
        </div>
      )}

      {invites.length === 0 ? (
        <p className="text-text-muted text-sm">No active invites.</p>
      ) : (
        <div className="space-y-2">
          {invites.map(invite => (
            <div key={invite.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <code className="text-sm font-mono text-text-primary">{invite.code}</code>
                  <button
                    onClick={() => copyCode(invite.code)}
                    className="rounded bg-bg-tertiary px-2 py-0.5 text-xs text-text-muted hover:text-text-primary"
                  >
                    {copied === invite.code ? 'Copied!' : 'Copy'}
                  </button>
                </div>
                <div className="mt-1 text-xs text-text-muted">
                  Uses: {invite.use_count}{invite.max_uses ? ` / ${invite.max_uses}` : ' (unlimited)'}
                  {invite.expires_at && (
                    <span className="ml-2">
                      Expires: {new Date(invite.expires_at).toLocaleDateString()}
                    </span>
                  )}
                  <span className="ml-2">Created by: {invite.created_by}</span>
                </div>
              </div>
              <button
                onClick={() => onDelete(serverId, invite.id)}
                className="ml-2 rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700"
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Events Tab ──────────────────────────────────────────

function EventsTab({ events, serverId, onCreate, onDelete, onRsvp }: {
  events: EventInfo[];
  serverId: string;
  onCreate: (serverId: string, name: string, startTime: string, options?: { description?: string; channelId?: string; endTime?: string; imageUrl?: string }) => void;
  onDelete: (serverId: string, eventId: string) => void;
  onRsvp: (serverId: string, eventId: string, status: string) => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [startTime, setStartTime] = useState('');
  const [endTime, setEndTime] = useState('');

  const handleCreate = () => {
    if (!name.trim() || !startTime) return;
    onCreate(serverId, name.trim(), new Date(startTime).toISOString(), {
      description: description.trim() || undefined,
      endTime: endTime ? new Date(endTime).toISOString() : undefined,
    });
    setShowForm(false);
    setName('');
    setDescription('');
    setStartTime('');
    setEndTime('');
  };

  const statusColors: Record<string, string> = {
    scheduled: 'bg-blue-600/20 text-blue-400',
    active: 'bg-green-600/20 text-green-400',
    completed: 'bg-gray-600/20 text-gray-400',
    cancelled: 'bg-red-600/20 text-red-400',
  };

  const formatDate = (iso: string) => {
    const d = new Date(iso);
    return d.toLocaleString(undefined, {
      month: 'short', day: 'numeric', year: 'numeric',
      hour: '2-digit', minute: '2-digit',
    });
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Scheduled Events</h3>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          {showForm ? 'Cancel' : 'Create Event'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <div>
            <label className="block text-xs font-medium text-text-muted mb-1">Event Name *</label>
            <input
              type="text"
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder="Community Game Night"
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-text-muted mb-1">Description</label>
            <textarea
              value={description}
              onChange={e => setDescription(e.target.value)}
              placeholder="What's this event about?"
              rows={2}
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium text-text-muted mb-1">Start Time *</label>
              <input
                type="datetime-local"
                value={startTime}
                onChange={e => setStartTime(e.target.value)}
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-text-muted mb-1">End Time</label>
              <input
                type="datetime-local"
                value={endTime}
                onChange={e => setEndTime(e.target.value)}
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
          </div>
          <button
            onClick={handleCreate}
            disabled={!name.trim() || !startTime}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Create Event
          </button>
        </div>
      )}

      {events.length === 0 ? (
        <p className="text-text-muted text-sm">No scheduled events.</p>
      ) : (
        <div className="space-y-2">
          {events.map(evt => (
            <div key={evt.id} className="rounded bg-bg-secondary p-3">
              <div className="flex items-start justify-between">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-text-primary">{evt.name}</span>
                    <span className={`rounded px-1.5 py-0.5 text-xs ${statusColors[evt.status] ?? 'bg-gray-600/20 text-gray-400'}`}>
                      {evt.status}
                    </span>
                  </div>
                  {evt.description && (
                    <p className="mt-1 text-xs text-text-secondary">{evt.description}</p>
                  )}
                  <div className="mt-1 text-xs text-text-muted">
                    {formatDate(evt.start_time)}
                    {evt.end_time && ` - ${formatDate(evt.end_time)}`}
                  </div>
                  <div className="mt-1 text-xs text-text-muted">
                    {evt.interested_count} interested
                    <span className="ml-2">by {evt.created_by}</span>
                  </div>
                </div>
                <div className="flex items-center gap-1 ml-2">
                  {evt.status === 'scheduled' && (
                    <>
                      <button
                        onClick={() => onRsvp(serverId, evt.id, 'interested')}
                        className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-muted hover:text-text-primary"
                        title="Mark as interested"
                      >
                        Interested
                      </button>
                      <button
                        onClick={() => onRsvp(serverId, evt.id, 'going')}
                        className="rounded bg-bg-accent/20 px-2 py-1 text-xs text-bg-accent hover:bg-bg-accent/30"
                        title="Mark as going"
                      >
                        Going
                      </button>
                    </>
                  )}
                  <button
                    onClick={() => onDelete(serverId, evt.id)}
                    className="rounded bg-red-600 px-2 py-1 text-xs font-medium text-white hover:bg-red-700"
                  >
                    Delete
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Settings Tab ────────────────────────────────────────

function SettingsTab({ serverId, settings, templates, onUpdate, onCreateTemplate, onDeleteTemplate }: {
  serverId: string;
  settings?: ServerCommunityInfo;
  templates: TemplateInfo[];
  onUpdate: (serverId: string, settings: { description?: string; isDiscoverable: boolean; welcomeMessage?: string; rulesText?: string; category?: string }) => void;
  onCreateTemplate: (serverId: string, name: string, description?: string) => void;
  onDeleteTemplate: (serverId: string, templateId: string) => void;
}) {
  const [description, setDescription] = useState(settings?.description ?? '');
  const [isDiscoverable, setIsDiscoverable] = useState(settings?.is_discoverable ?? false);
  const [welcomeMessage, setWelcomeMessage] = useState(settings?.welcome_message ?? '');
  const [rulesText, setRulesText] = useState(settings?.rules_text ?? '');
  const [category, setCategory] = useState(settings?.category ?? '');
  const [templateName, setTemplateName] = useState('');
  const [templateDesc, setTemplateDesc] = useState('');
  const [showTemplateForm, setShowTemplateForm] = useState(false);

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
    onUpdate(serverId, {
      description: description || undefined,
      isDiscoverable,
      welcomeMessage: welcomeMessage || undefined,
      rulesText: rulesText || undefined,
      category: category || undefined,
    });
  };

  const handleCreateTemplate = () => {
    if (!templateName.trim()) return;
    onCreateTemplate(serverId, templateName.trim(), templateDesc.trim() || undefined);
    setTemplateName('');
    setTemplateDesc('');
    setShowTemplateForm(false);
  };

  return (
    <div className="space-y-6">
      {/* Community Settings */}
      <div className="space-y-3">
        <h3 className="text-sm font-semibold text-text-secondary">Community Settings</h3>

        <div>
          <label className="block text-xs font-medium text-text-muted mb-1">Server Description</label>
          <textarea
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
          <label className="block text-xs font-medium text-text-muted mb-1">Welcome Message</label>
          <textarea
            value={welcomeMessage}
            onChange={e => setWelcomeMessage(e.target.value)}
            placeholder="Welcome new members with a message..."
            rows={2}
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
          />
        </div>

        <div>
          <label className="block text-xs font-medium text-text-muted mb-1">Server Rules</label>
          <textarea
            value={rulesText}
            onChange={e => setRulesText(e.target.value)}
            placeholder="Define rules that members must accept..."
            rows={3}
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
          />
        </div>

        <div>
          <label className="block text-xs font-medium text-text-muted mb-1">Category</label>
          <select
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
          onClick={handleSave}
          className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          Save Settings
        </button>
      </div>

      {/* Templates */}
      <div className="space-y-3 border-t border-border pt-4">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text-secondary">Server Templates</h3>
          <button
            onClick={() => setShowTemplateForm(!showTemplateForm)}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
          >
            {showTemplateForm ? 'Cancel' : 'Create Template'}
          </button>
        </div>

        {showTemplateForm && (
          <div className="rounded bg-bg-secondary p-3 space-y-3">
            <div>
              <label className="block text-xs font-medium text-text-muted mb-1">Template Name *</label>
              <input
                type="text"
                value={templateName}
                onChange={e => setTemplateName(e.target.value)}
                placeholder="My Server Template"
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-text-muted mb-1">Description</label>
              <input
                type="text"
                value={templateDesc}
                onChange={e => setTemplateDesc(e.target.value)}
                placeholder="What's this template for?"
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
            <button
              onClick={handleCreateTemplate}
              disabled={!templateName.trim()}
              className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              Create Template
            </button>
          </div>
        )}

        {templates.length === 0 ? (
          <p className="text-text-muted text-sm">No templates created.</p>
        ) : (
          <div className="space-y-2">
            {templates.map(tpl => (
              <div key={tpl.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
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
                  onClick={() => onDeleteTemplate(serverId, tpl.id)}
                  className="rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700"
                >
                  Delete
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ── Discovery Tab ───────────────────────────────────────

function DiscoveryTab({ servers, onJoin, onRefresh }: {
  servers: ServerCommunityInfo[];
  onJoin: (code: string) => void;
  onRefresh: (category?: string) => void;
}) {
  const [filterCategory, setFilterCategory] = useState('');
  const [joinCode, setJoinCode] = useState('');

  const handleJoinByCode = () => {
    if (!joinCode.trim()) return;
    onJoin(joinCode.trim());
    setJoinCode('');
  };

  return (
    <div className="space-y-4">
      {/* Join by invite code */}
      <div className="rounded bg-bg-secondary p-3 space-y-2">
        <h3 className="text-sm font-semibold text-text-secondary">Join by Invite Code</h3>
        <div className="flex gap-2">
          <input
            type="text"
            value={joinCode}
            onChange={e => setJoinCode(e.target.value)}
            placeholder="Enter invite code..."
            className="flex-1 rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            onKeyDown={e => { if (e.key === 'Enter') handleJoinByCode(); }}
          />
          <button
            onClick={handleJoinByCode}
            disabled={!joinCode.trim()}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Join
          </button>
        </div>
      </div>

      {/* Browse servers */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text-secondary">Discover Servers</h3>
          <div className="flex items-center gap-2">
            <select
              value={filterCategory}
              onChange={e => {
                setFilterCategory(e.target.value);
                onRefresh(e.target.value || undefined);
              }}
              className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-primary outline-none"
            >
              <option value="">All Categories</option>
              <option value="gaming">Gaming</option>
              <option value="music">Music</option>
              <option value="education">Education</option>
              <option value="science">Science & Technology</option>
              <option value="entertainment">Entertainment</option>
              <option value="community">General Community</option>
            </select>
            <button
              onClick={() => onRefresh(filterCategory || undefined)}
              className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-muted hover:text-text-primary"
            >
              Refresh
            </button>
          </div>
        </div>

        {servers.length === 0 ? (
          <p className="text-text-muted text-sm">No discoverable servers found.</p>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {servers.map(server => (
              <div key={server.server_id} className="rounded bg-bg-secondary p-4 flex flex-col justify-between">
                <div>
                  <div className="flex items-center gap-2">
                    <div className="h-10 w-10 rounded-full bg-bg-accent/30 flex items-center justify-center text-text-primary text-sm font-bold">
                      {(server.description ?? server.server_id).charAt(0).toUpperCase()}
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-medium text-text-primary truncate">
                        {server.server_id}
                      </p>
                      {server.category && (
                        <span className="rounded bg-bg-accent/20 px-1.5 py-0.5 text-xs text-bg-accent">
                          {server.category}
                        </span>
                      )}
                    </div>
                  </div>
                  {server.description && (
                    <p className="mt-2 text-xs text-text-secondary line-clamp-2">{server.description}</p>
                  )}
                </div>
                <button
                  onClick={() => {
                    // For discovery, use server_id to join
                    const store = useChatStore.getState();
                    store.joinServer(server.server_id);
                  }}
                  className="mt-3 w-full rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
                >
                  Join Server
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
