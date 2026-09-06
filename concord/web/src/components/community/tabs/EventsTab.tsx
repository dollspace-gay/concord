import { useEffect, useState } from 'react';
import type { ChannelInfo, EventInfo, RsvpInfo } from '../../../api/types';
import { ActionOutcome } from './ActionOutcome';
import { useActionStatus } from './useActionStatus';

// ── Events Tab ──────────────────────────────────────────

export function EventsTab({ events, channels, rsvps, userId, serverId, onCreate, onDelete, onRsvp, onRemoveRsvp, onListRsvps, onStatus }: {
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
