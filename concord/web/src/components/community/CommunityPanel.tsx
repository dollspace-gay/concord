import { useState, useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useAuthStore } from '../../stores/authStore';
import type { ChannelFollowInfo, ChannelInfo, InviteInfo, EventInfo, RsvpInfo, ServerCommunityInfo, TemplateInfo } from '../../api/types';
import { Dialog } from '../Dialog';

const EMPTY_INVITES: InviteInfo[] = [];
const EMPTY_EVENTS: EventInfo[] = [];
const EMPTY_TEMPLATES: TemplateInfo[] = [];

function useActionStatus() {
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

function ActionOutcome({ outcome }: { outcome: { kind: 'success' | 'error'; message: string } | null }) {
  if (!outcome) return null;
  return <p role={outcome.kind === 'error' ? 'alert' : 'status'} className={`text-sm ${outcome.kind === 'error' ? 'text-red-400' : 'text-green-400'}`}>{outcome.message}</p>;
}

type Tab = 'invites' | 'events' | 'announcements' | 'settings' | 'discovery';

interface Props {
  serverId: string;
  onClose: () => void;
}

export function CommunityPanel({ serverId, onClose }: Props) {
  const [activeTab, setActiveTab] = useState<Tab>('invites');

  const invites = useChatStore(s => s.invites[serverId] ?? EMPTY_INVITES);
  const serverEvents = useChatStore(s => s.serverEvents[serverId] ?? EMPTY_EVENTS);
  const eventRsvps = useChatStore(s => s.eventRsvps);
  const communitySettings = useChatStore(s => s.communitySettings[serverId]);
  const discoverableServers = useChatStore(s => s.discoverableServers);
  const templates = useChatStore(s => s.templates[serverId] ?? EMPTY_TEMPLATES);
  const channelsByServer = useChatStore(s => s.channels);
  const channelFollows = useChatStore(s => s.channelFollows);
  const userId = useAuthStore(s => s.user?.id);
  const serverChannels = channelsByServer[serverId] ?? [];
  const allChannels = Object.values(channelsByServer).flat();

  const listInvites = useChatStore(s => s.listInvites);
  const createInvite = useChatStore(s => s.createInvite);
  const deleteInvite = useChatStore(s => s.deleteInvite);
  const listEvents = useChatStore(s => s.listEvents);
  const createEvent = useChatStore(s => s.createEvent);
  const deleteEvent = useChatStore(s => s.deleteEvent);
  const setRsvp = useChatStore(s => s.setRsvp);
  const removeRsvp = useChatStore(s => s.removeRsvp);
  const listRsvps = useChatStore(s => s.listRsvps);
  const updateCommunitySettings = useChatStore(s => s.updateCommunitySettings);
  const getCommunitySettings = useChatStore(s => s.getCommunitySettings);
  const discoverServers = useChatStore(s => s.discoverServers);
  const useInvite = useChatStore(s => s.useInvite);
  const listTemplates = useChatStore(s => s.listTemplates);
  const createTemplate = useChatStore(s => s.createTemplate);
  const deleteTemplate = useChatStore(s => s.deleteTemplate);
  const instantiateTemplate = useChatStore(s => s.instantiateTemplate);
  const setAnnouncementChannel = useChatStore(s => s.setAnnouncementChannel);
  const followChannel = useChatStore(s => s.followChannel);
  const unfollowChannel = useChatStore(s => s.unfollowChannel);
  const listChannelFollows = useChatStore(s => s.listChannelFollows);
  const updateEventStatus = useChatStore(s => s.updateEventStatus);

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

  const tabLabels: Record<Tab, string> = {
    invites: 'Invites',
    events: 'Events',
    announcements: 'Announcements',
    settings: 'Settings',
    discovery: 'Discovery',
  };

  return (
    <Dialog label="Community" onClose={onClose} panelClassName="w-full max-w-3xl max-h-[85vh] flex flex-col rounded-lg bg-bg-primary shadow-xl">
      <div className="flex min-h-0 flex-1 flex-col">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border p-4">
          <h2 className="text-lg font-bold text-text-primary">Community</h2>
          <button onClick={onClose} aria-label="Close community" className="text-text-muted hover:text-text-primary text-xl leading-none">&times;</button>
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
              channels={serverChannels}
              rsvps={eventRsvps}
              userId={userId}
              serverId={serverId}
              onCreate={createEvent}
              onDelete={deleteEvent}
              onRsvp={setRsvp}
              onRemoveRsvp={removeRsvp}
              onListRsvps={listRsvps}
              onStatus={updateEventStatus}
            />
          )}
          {activeTab === 'announcements' && (
            <AnnouncementsTab
              serverId={serverId}
              serverChannels={serverChannels}
              allChannels={allChannels}
              follows={channelFollows}
              onSetAnnouncement={setAnnouncementChannel}
              onFollow={followChannel}
              onUnfollow={unfollowChannel}
              onList={listChannelFollows}
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
              onInstantiateTemplate={instantiateTemplate}
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
    </Dialog>
  );
}

// ── Invites Tab ──────────────────────────────────────────

function InvitesTab({ invites, serverId, onCreate, onDelete }: {
  invites: InviteInfo[];
  serverId: string;
  onCreate: (serverId: string, maxUses?: number, expiresAt?: string, channelId?: string) => Promise<void>;
  onDelete: (serverId: string, inviteId: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [maxUses, setMaxUses] = useState('');
  const [expiresIn, setExpiresIn] = useState('24'); // hours
  const [copied, setCopied] = useState<string | null>(null);
  const { pending, outcome, run } = useActionStatus();

  const handleCreate = () => {
    const mu = maxUses ? parseInt(maxUses, 10) : undefined;
    const hours = parseInt(expiresIn, 10);
    const ea = hours > 0 ? new Date(Date.now() + hours * 3600000).toISOString() : undefined;
    void run('create', () => onCreate(serverId, mu, ea), 'Invite created.', () => {
      setShowForm(false);
      setMaxUses('');
      setExpiresIn('24');
    });
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
          disabled={pending !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          {showForm ? 'Cancel' : 'Create Invite'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <div>
            <label htmlFor="invite-max-uses" className="block text-xs font-medium text-text-muted mb-1">Max Uses (0 = unlimited)</label>
            <input
              id="invite-max-uses"
              type="number"
              value={maxUses}
              onChange={e => setMaxUses(e.target.value)}
              placeholder="0"
              min="0"
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            />
          </div>
          <div>
            <label htmlFor="invite-expiry" className="block text-xs font-medium text-text-muted mb-1">Expires In (hours, 0 = never)</label>
            <select
              id="invite-expiry"
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
            disabled={pending !== null}
            onClick={handleCreate}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
          >
            {pending === 'create' ? 'Generating…' : 'Generate Invite'}
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
                disabled={pending !== null}
                onClick={() => run(`delete:${invite.id}`, () => onDelete(serverId, invite.id), 'Invite deleted.')}
                className="ml-2 rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700"
              >
                {pending === `delete:${invite.id}` ? 'Deleting…' : 'Delete'}
              </button>
            </div>
          ))}
        </div>
      )}
      <ActionOutcome outcome={outcome} />
    </div>
  );
}

// ── Events Tab ──────────────────────────────────────────

function EventsTab({ events, channels, rsvps, userId, serverId, onCreate, onDelete, onRsvp, onRemoveRsvp, onListRsvps, onStatus }: {
  events: EventInfo[];
  channels: ChannelInfo[];
  rsvps: Record<string, RsvpInfo[]>;
  userId?: string;
  serverId: string;
  onCreate: (serverId: string, name: string, startTime: string, options?: { description?: string; channelId?: string; endTime?: string; imageUrl?: string }) => Promise<void>;
  onDelete: (serverId: string, eventId: string) => Promise<void>;
  onRsvp: (serverId: string, eventId: string, status: string) => Promise<void>;
  onRemoveRsvp: (serverId: string, eventId: string) => Promise<void>;
  onListRsvps: (eventId: string) => void;
  onStatus: (serverId: string, eventId: string, status: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [startTime, setStartTime] = useState('');
  const [endTime, setEndTime] = useState('');
  const [channelId, setChannelId] = useState('');
  const { pending, outcome, run } = useActionStatus();

  useEffect(() => {
    for (const event of events) onListRsvps(event.id);
  }, [events, onListRsvps]);

  const handleCreate = () => {
    if (!name.trim() || !startTime) return;
    void run('create', () => onCreate(serverId, name.trim(), new Date(startTime).toISOString(), {
      description: description.trim() || undefined,
      channelId: channelId || undefined,
      endTime: endTime ? new Date(endTime).toISOString() : undefined,
    }), 'Event created.', () => {
      setShowForm(false);
      setName('');
      setDescription('');
      setStartTime('');
      setEndTime('');
      setChannelId('');
    });
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
          disabled={pending !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          {showForm ? 'Cancel' : 'Create Event'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <div>
            <label htmlFor="community-event-name" className="block text-xs font-medium text-text-muted mb-1">Event Name *</label>
            <input
              id="community-event-name"
              type="text"
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder="Community Game Night"
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            />
          </div>
          <div>
            <label htmlFor="community-event-description" className="block text-xs font-medium text-text-muted mb-1">Description</label>
            <textarea
              id="community-event-description"
              value={description}
              onChange={e => setDescription(e.target.value)}
              placeholder="What's this event about?"
              rows={2}
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent resize-none"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label htmlFor="community-event-start" className="block text-xs font-medium text-text-muted mb-1">Start Time *</label>
              <input
                id="community-event-start"
                type="datetime-local"
                value={startTime}
                onChange={e => setStartTime(e.target.value)}
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
            <div>
              <label htmlFor="community-event-end" className="block text-xs font-medium text-text-muted mb-1">End Time</label>
              <input
                id="community-event-end"
                type="datetime-local"
                value={endTime}
                onChange={e => setEndTime(e.target.value)}
                className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
              />
            </div>
          </div>
          <div>
            <label htmlFor="community-event-channel" className="block text-xs font-medium text-text-muted mb-1">Linked Channel</label>
            <select
              id="community-event-channel"
              value={channelId}
              onChange={event => setChannelId(event.target.value)}
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            >
              <option value="">No linked channel</option>
              {channels.map(channel => <option key={channel.id} value={channel.id}>#{channel.name}</option>)}
            </select>
          </div>
          <button
            onClick={handleCreate}
            disabled={!name.trim() || !startTime || pending !== null}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {pending === 'create' ? 'Creating…' : 'Create Event'}
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
                  {evt.channel_id && (
                    <div className="mt-1 text-xs text-text-muted">
                      Linked channel: #{channels.find(channel => channel.id === evt.channel_id)?.name ?? evt.channel_id}
                    </div>
                  )}
                  <div className="mt-1 text-xs text-text-muted">
                    {(rsvps[evt.id] ?? []).filter(rsvp => rsvp.status === 'going').length} going,{' '}
                    {(rsvps[evt.id] ?? []).filter(rsvp => rsvp.status === 'interested').length} interested
                    <span className="ml-2">by {evt.created_by}</span>
                  </div>
                </div>
                <div className="flex items-center gap-1 ml-2">
                  {evt.status === 'scheduled' && (
                    <>
                      <button
                        disabled={pending !== null}
                        onClick={() => run(`rsvp:${evt.id}`, () => onRsvp(serverId, evt.id, 'interested'), 'RSVP updated.')}
                        className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-muted hover:text-text-primary"
                        title="Mark as interested"
                      >
                        Interested
                      </button>
                      <button
                        disabled={pending !== null}
                        onClick={() => run(`rsvp:${evt.id}`, () => onRsvp(serverId, evt.id, 'going'), 'RSVP updated.')}
                        className="rounded bg-bg-accent/20 px-2 py-1 text-xs text-bg-accent hover:bg-bg-accent/30"
                        title="Mark as going"
                      >
                        Going
                      </button>
                      {(rsvps[evt.id] ?? []).some(rsvp => rsvp.user_id === userId) && (
                        <button
                          disabled={pending !== null}
                          onClick={() => run(`rsvp:${evt.id}`, () => onRemoveRsvp(serverId, evt.id), 'RSVP cleared.')}
                          className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-muted hover:text-text-primary"
                        >
                          Clear RSVP
                        </button>
                      )}
                      <button
                        disabled={pending !== null}
                        onClick={() => run(`status:${evt.id}`, () => onStatus(serverId, evt.id, 'active'), 'Event started.')}
                        className="rounded bg-green-600/20 px-2 py-1 text-xs text-green-400"
                      >
                        Start
                      </button>
                    </>
                  )}
                  {evt.status === 'active' && (
                    <button
                      disabled={pending !== null}
                      onClick={() => run(`status:${evt.id}`, () => onStatus(serverId, evt.id, 'completed'), 'Event completed.')}
                      className="rounded bg-bg-accent/20 px-2 py-1 text-xs text-bg-accent"
                    >
                      Complete
                    </button>
                  )}
                  {evt.status !== 'completed' && evt.status !== 'cancelled' && (
                    <button
                      disabled={pending !== null}
                      onClick={() => run(`status:${evt.id}`, () => onStatus(serverId, evt.id, 'cancelled'), 'Event cancelled.')}
                      className="rounded bg-red-600/20 px-2 py-1 text-xs text-red-400"
                    >
                      Cancel
                    </button>
                  )}
                  <button
                    disabled={pending !== null}
                    onClick={() => run(`delete:${evt.id}`, () => onDelete(serverId, evt.id), 'Event deleted.')}
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
      <ActionOutcome outcome={outcome} />
    </div>
  );
}

function AnnouncementsTab({
  serverId,
  serverChannels,
  allChannels,
  follows,
  onSetAnnouncement,
  onFollow,
  onUnfollow,
  onList,
}: {
  serverId: string;
  serverChannels: ChannelInfo[];
  allChannels: ChannelInfo[];
  follows: Record<string, ChannelFollowInfo[]>;
  onSetAnnouncement: (serverId: string, channel: string, isAnnouncement: boolean) => Promise<void>;
  onFollow: (sourceChannelId: string, targetChannelId: string) => Promise<void>;
  onUnfollow: (followId: string) => Promise<void>;
  onList: (channelId: string) => void;
}) {
  const [sourceId, setSourceId] = useState('');
  const [targetId, setTargetId] = useState('');
  const { pending, outcome, run } = useActionStatus();
  const source = serverChannels.find(channel => channel.id === sourceId);
  const sourceFollows = sourceId ? follows[sourceId] ?? [] : [];

  useEffect(() => {
    if (sourceId) onList(sourceId);
  }, [sourceId, onList]);

  const channelLabel = (channelId: string) => {
    const channel = allChannels.find(candidate => candidate.id === channelId);
    return channel ? `${channel.server_id} / #${channel.name}` : channelId;
  };

  return (
    <div className="space-y-4">
      <div>
        <label htmlFor="announcement-source" className="mb-1 block text-sm font-medium text-text-secondary">Announcement source</label>
        <select
          id="announcement-source"
          value={sourceId}
          onChange={event => setSourceId(event.target.value)}
          className="w-full rounded bg-bg-tertiary px-3 py-2 text-sm text-text-primary"
        >
          <option value="">Select a channel</option>
          {serverChannels.map(channel => <option key={channel.id} value={channel.id}>#{channel.name}</option>)}
        </select>
      </div>

      {source && (
        <div className="flex flex-wrap gap-2">
          <button
            disabled={pending !== null}
            onClick={() => run('announcement', () => onSetAnnouncement(serverId, source.name, true), 'Announcement channel enabled.')}
            className="rounded bg-bg-accent px-3 py-2 text-sm text-white"
          >
            Enable announcements
          </button>
          <button
            disabled={pending !== null}
            onClick={() => run('announcement', () => onSetAnnouncement(serverId, source.name, false), 'Announcement channel disabled.')}
            className="rounded bg-bg-tertiary px-3 py-2 text-sm text-text-secondary"
          >
            Disable announcements
          </button>
        </div>
      )}

      <div>
        <label htmlFor="announcement-target" className="mb-1 block text-sm font-medium text-text-secondary">Cross-post destination</label>
        <div className="flex gap-2">
          <select
            id="announcement-target"
            value={targetId}
            onChange={event => setTargetId(event.target.value)}
            className="min-w-0 flex-1 rounded bg-bg-tertiary px-3 py-2 text-sm text-text-primary"
          >
            <option value="">Select a destination</option>
            {allChannels.filter(channel => channel.id !== sourceId).map(channel => (
              <option key={channel.id} value={channel.id}>{channel.server_id} / #{channel.name}</option>
            ))}
          </select>
          <button
            disabled={!sourceId || !targetId || pending !== null}
            onClick={() => run('follow', () => onFollow(sourceId, targetId), 'Channel followed.', () => setTargetId(''))}
            className="rounded bg-bg-accent px-3 py-2 text-sm text-white disabled:opacity-50"
          >
            {pending === 'follow' ? 'Following…' : 'Follow'}
          </button>
        </div>
      </div>

      {sourceId && sourceFollows.length === 0 && (
        <p className="text-sm text-text-muted">This channel has no outgoing follows.</p>
      )}
      {sourceFollows.map(follow => (
        <div key={follow.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
          <span className="text-sm text-text-secondary">Cross-posts to {channelLabel(follow.target_channel_id)}</span>
          <button
            disabled={pending !== null}
            onClick={() => run(`unfollow:${follow.id}`, () => onUnfollow(follow.id), 'Channel unfollowed.')}
            className="rounded bg-red-600 px-3 py-1 text-xs font-medium text-white"
          >
            {pending === `unfollow:${follow.id}` ? 'Unfollowing…' : 'Unfollow'}
          </button>
        </div>
      ))}
      <ActionOutcome outcome={outcome} />
    </div>
  );
}

// ── Settings Tab ────────────────────────────────────────

function SettingsTab({ serverId, settings, templates, onUpdate, onCreateTemplate, onDeleteTemplate, onInstantiateTemplate }: {
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

// ── Discovery Tab ───────────────────────────────────────

function DiscoveryTab({ servers, onJoin, onRefresh }: {
  servers: ServerCommunityInfo[];
  onJoin: (code: string) => Promise<void>;
  onRefresh: (category?: string) => void;
}) {
  const [filterCategory, setFilterCategory] = useState('');
  const [joinCode, setJoinCode] = useState('');
  const { pending, outcome, run } = useActionStatus();

  const handleJoinByCode = () => {
    if (!joinCode.trim()) return;
    const code = joinCode.trim();
    void run('join', () => onJoin(code), 'Invite accepted.', () => setJoinCode(''));
  };

  return (
    <div className="space-y-4">
      {/* Join by invite code */}
      <div className="rounded bg-bg-secondary p-3 space-y-2">
        <h3 className="text-sm font-semibold text-text-secondary">Join by Invite Code</h3>
        <div className="flex gap-2">
          <input
            aria-label="Invite code"
            type="text"
            value={joinCode}
            onChange={e => setJoinCode(e.target.value)}
            placeholder="Enter invite code..."
            className="flex-1 rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            onKeyDown={e => { if (e.key === 'Enter') handleJoinByCode(); }}
          />
          <button
            onClick={handleJoinByCode}
            disabled={!joinCode.trim() || pending !== null}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {pending === 'join' ? 'Joining…' : 'Join'}
          </button>
        </div>
      </div>
      <ActionOutcome outcome={outcome} />

      {/* Browse servers */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text-secondary">Discover Servers</h3>
          <div className="flex items-center gap-2">
            <select
              aria-label="Discovery category"
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
