import { useEffect, useState } from 'react';
import { useAuthStore } from '../../stores/authStore';
import { useChatStore } from '../../stores/chatStore';
import { Dialog } from '../Dialog';
import { AnnouncementsTab } from './tabs/AnnouncementsTab';
import { DiscoveryTab } from './tabs/DiscoveryTab';
import { EventsTab } from './tabs/EventsTab';
import { InvitesTab } from './tabs/InvitesTab';
import { SettingsTab } from './tabs/SettingsTab';
import { EMPTY_EVENTS, EMPTY_INVITES, EMPTY_TEMPLATES } from './tabs/defaults';

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
              className={`px-4 py-2 text-sm font-medium ${activeTab === t ? 'border-b-2 border-bg-accent text-text-primary' : 'text-text-muted hover:text-text-secondary'
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
