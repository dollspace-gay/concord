import { useEffect, useState } from 'react';
import type { ChannelFollowInfo, ChannelInfo } from '../../../api/types';
import { ActionOutcome } from './ActionOutcome';
import { useActionStatus } from './useActionStatus';

export function AnnouncementsTab({
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
