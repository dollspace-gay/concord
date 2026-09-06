import { useCallback, useEffect, useRef, useState } from 'react';
import { useAuthStore } from '../../stores/authStore';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import * as api from '../../api/client';
import type { AtprotoPublicationStatus, IrcToken, UserProfileInfo } from '../../api/types';
import { Dialog } from '../Dialog';

export function SettingsPage() {
  const user = useAuthStore((s) => s.user);
  const logout = useAuthStore((s) => s.logout);
  const setShowSettings = useUiStore((s) => s.setShowSettings);
  const activeAccountId = useChatStore((s) => s.activeAccountId);
  const protectedGeneration = useChatStore((s) => s.protectedGeneration);
  const activeServer = useUiStore((s) => s.activeServer);
  const servers = useChatStore((s) => s.servers);
  const setPresence = useChatStore((s) => s.setPresence);
  const setServerNickname = useChatStore((s) => s.setServerNickname);

  const [tokens, setTokens] = useState<IrcToken[]>([]);
  const [newTokenLabel, setNewTokenLabel] = useState('');
  const [newToken, setNewToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [profile, setProfile] = useState<UserProfileInfo | null>(null);
  const [bio, setBio] = useState('');
  const [pronouns, setPronouns] = useState('');
  const [status, setStatus] = useState('online');
  const [customStatus, setCustomStatus] = useState('');
  const [statusEmoji, setStatusEmoji] = useState('');
  const [serverNickname, setServerNicknameValue] = useState('');
  const [profileSaving, setProfileSaving] = useState(false);
  const bioEdited = useRef(false);
  const pronounsEdited = useRef(false);

  const captureScope = useCallback(() => ({ userId: user?.id, activeAccountId, protectedGeneration }), [user?.id, activeAccountId, protectedGeneration]);
  const scopeIsCurrent = useCallback((scope: { userId?: string; activeAccountId: string | null; protectedGeneration: number }) => {
    const auth = useAuthStore.getState();
    const chat = useChatStore.getState();
    return auth.user?.id === scope.userId
      && chat.activeAccountId === scope.activeAccountId
      && chat.protectedGeneration === scope.protectedGeneration;
  }, []);

  useEffect(() => {
    if (!user) return;
    bioEdited.current = false;
    pronounsEdited.current = false;
    let current = true;
    const scope = captureScope();
    api.getFullUserProfile(user.id).then((next) => {
      if (!current || !scopeIsCurrent(scope)) return;
      setProfile(next);
      if (!bioEdited.current) setBio(next.bio ?? '');
      if (!pronounsEdited.current) setPronouns(next.pronouns ?? '');
    }).catch((cause) => {
      if (current && scopeIsCurrent(scope)) setError(String(cause));
    });
    return () => { current = false; };
  }, [user, captureScope, scopeIsCurrent]);

  const saveProfile = async () => {
    const scope = captureScope();
    setProfileSaving(true);
    setError(null);
    try {
      await api.updateProfile({ bio: bio.trim() || null, pronouns: pronouns.trim() || null });
      if (scopeIsCurrent(scope)) setProfile((value) => value ? { ...value, bio: bio.trim() || null, pronouns: pronouns.trim() || null } : value);
    } catch (cause) {
      if (scopeIsCurrent(scope)) setError(String(cause));
    } finally {
      if (scopeIsCurrent(scope)) setProfileSaving(false);
    }
  };

  const uploadProfileMedia = async (file: File, purpose: 'user_avatar' | 'user_banner') => {
    const scope = captureScope();
    setProfileSaving(true);
    setError(null);
    try {
      const uploaded = await api.uploadFile(file, { purpose });
      await api.updateProfile({ [purpose === 'user_avatar' ? 'avatar_url' : 'banner_url']: uploaded.url });
      if (!scopeIsCurrent(scope)) return;
      setProfile((value) => value ? { ...value, [purpose === 'user_avatar' ? 'avatar_url' : 'banner_url']: uploaded.url } : value);
    } catch (cause) {
      if (scopeIsCurrent(scope)) setError(String(cause));
    } finally {
      if (scopeIsCurrent(scope)) setProfileSaving(false);
    }
  };

  const uploadMemberAvatar = async (file: File) => {
    if (!activeServer) return;
    const scope = captureScope();
    setProfileSaving(true);
    setError(null);
    try {
      const uploaded = await api.uploadFile(file, { serverId: activeServer, purpose: 'server_member_avatar' });
      await api.updateServerMemberAvatar(activeServer, uploaded.url);
    } catch (cause) {
      if (scopeIsCurrent(scope)) setError(String(cause));
    } finally {
      if (scopeIsCurrent(scope)) setProfileSaving(false);
    }
  };

  useEffect(() => {
    let current = true;
    const scope = captureScope();
    api.getTokens().then((next) => {
      if (current && scopeIsCurrent(scope)) setTokens(next);
    }).catch((cause) => {
      if (current && scopeIsCurrent(scope)) setError(String(cause));
    });
    return () => { current = false; };
  }, [user?.id, activeAccountId, protectedGeneration, captureScope, scopeIsCurrent]);

  const handleCreateToken = async () => {
    const scope = captureScope();
    setLoading(true);
    setError(null);
    try {
      const result = await api.createToken(newTokenLabel || undefined);
      if (!scopeIsCurrent(scope)) return;
      setNewToken(result.token);
      setNewTokenLabel('');
      const updated = await api.getTokens();
      if (!scopeIsCurrent(scope)) return;
      setTokens(updated);
    } catch (e) {
      if (scopeIsCurrent(scope)) setError(String(e));
    }
    if (scopeIsCurrent(scope)) setLoading(false);
  };

  const handleDeleteToken = async (id: string) => {
    const scope = captureScope();
    setError(null);
    try {
      await api.deleteToken(id);
      if (!scopeIsCurrent(scope)) return;
      setTokens((prev) => prev.filter((t) => t.id !== id));
    } catch (e) {
      if (scopeIsCurrent(scope)) setError(String(e));
    }
  };

  const handleLogout = async () => {
    await logout();
    setShowSettings(false);
  };

  return (
    <Dialog label="Settings" onClose={() => setShowSettings(false)} panelClassName="max-h-[80vh] w-full max-w-lg overflow-y-auto rounded-lg bg-bg-secondary p-6">
      <div>
        {error && <p role="alert" className="mb-4 rounded bg-red-950/50 p-2 text-sm text-red-300">{error}</p>}
        <div className="mb-6 flex items-center justify-between">
          <h2 className="text-xl font-bold text-text-primary">Settings</h2>
          <button
            onClick={() => setShowSettings(false)}
            aria-label="Close settings"
            className="rounded p-1 text-text-muted transition-colors hover:text-text-primary"
          >
            <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Profile */}
        {user && (
          <section className="mb-6">
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">
              Profile
            </h3>
            <div className="flex items-center gap-4 rounded-md bg-bg-tertiary p-4">
              {profile?.avatar_url || user.avatar_url ? (
                <img src={profile?.avatar_url || user.avatar_url || ''} alt="" className="h-16 w-16 rounded-full" />
              ) : (
                <div className="flex h-16 w-16 items-center justify-center rounded-full bg-bg-accent text-xl font-bold text-white">
                  {user.username[0]?.toUpperCase()}
                </div>
              )}
              <div>
                <p className="text-lg font-semibold text-text-primary">{user.username}</p>
                {user.email && <p className="text-sm text-text-muted">{user.email}</p>}
              </div>
            </div>
            <div className="mt-3 space-y-3 rounded-md bg-bg-tertiary p-4">
              {profile?.banner_url && <img src={profile.banner_url} alt="Profile banner" className="h-24 w-full rounded object-cover" />}
              <label className="block text-xs text-text-muted">Pronouns
                <input aria-label="Pronouns" value={pronouns} maxLength={100} onChange={(event) => { pronounsEdited.current = true; setPronouns(event.target.value); }} className="mt-1 w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary" />
              </label>
              <label className="block text-xs text-text-muted">Bio
                <textarea aria-label="Bio" value={bio} maxLength={2000} onChange={(event) => { bioEdited.current = true; setBio(event.target.value); }} className="mt-1 h-20 w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary" />
              </label>
              <div className="flex flex-wrap gap-2">
                <button disabled={profileSaving} onClick={saveProfile} className="rounded bg-bg-accent px-3 py-2 text-sm text-white disabled:opacity-50">Save profile</button>
                <label className="cursor-pointer rounded bg-bg-input px-3 py-2 text-sm text-text-secondary">Upload avatar
                  <input aria-label="Upload profile avatar" type="file" accept="image/*" className="hidden" onChange={(event) => { const file = event.target.files?.[0]; if (file) void uploadProfileMedia(file, 'user_avatar'); }} />
                </label>
                <label className="cursor-pointer rounded bg-bg-input px-3 py-2 text-sm text-text-secondary">Upload banner
                  <input aria-label="Upload profile banner" type="file" accept="image/*" className="hidden" onChange={(event) => { const file = event.target.files?.[0]; if (file) void uploadProfileMedia(file, 'user_banner'); }} />
                </label>
              </div>
            </div>
          </section>
        )}

        <section className="mb-6">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">Presence</h3>
          <div className="space-y-2 rounded-md bg-bg-tertiary p-4">
            <select aria-label="Presence status" value={status} onChange={(event) => setStatus(event.target.value)} className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary">
              <option value="online">Online</option><option value="idle">Idle</option><option value="dnd">Do not disturb</option><option value="invisible">Invisible</option>
            </select>
            <input aria-label="Status emoji" value={statusEmoji} maxLength={64} onChange={(event) => setStatusEmoji(event.target.value)} placeholder="Status emoji" className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary" />
            <input aria-label="Custom status" value={customStatus} maxLength={128} onChange={(event) => setCustomStatus(event.target.value)} placeholder="Custom status" className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary" />
            <button onClick={() => setPresence(status, customStatus.trim() || undefined, statusEmoji.trim() || undefined)} className="rounded bg-bg-accent px-3 py-2 text-sm text-white">Update presence</button>
          </div>
        </section>

        {activeServer && <section className="mb-6">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">{servers.find((server) => server.id === activeServer)?.name ?? 'Server'} identity</h3>
          <div className="space-y-2 rounded-md bg-bg-tertiary p-4">
            <input aria-label="Server nickname" value={serverNickname} maxLength={256} onChange={(event) => setServerNicknameValue(event.target.value)} placeholder="Server nickname" className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary" />
            <button onClick={() => setServerNickname(activeServer, serverNickname.trim() || undefined)} className="rounded bg-bg-accent px-3 py-2 text-sm text-white">Save server nickname</button>
            <label className="inline-block cursor-pointer rounded bg-bg-input px-3 py-2 text-sm text-text-secondary">Upload server avatar
              <input aria-label="Upload server avatar" type="file" accept="image/*" className="hidden" onChange={(event) => { const file = event.target.files?.[0]; if (file) void uploadMemberAvatar(file); }} />
            </label>
          </div>
        </section>}

        {/* IRC Tokens */}
        <section className="mb-6">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">
            IRC Access Tokens
          </h3>
          <p className="mb-3 text-sm text-text-muted">
            Use these tokens to connect from IRC clients like HexChat, irssi, or WeeChat.
            Use the token as your password with <code className="text-text-secondary">PASS</code>.
          </p>

          {newToken && (
            <div className="mb-3 rounded-md border border-status-online/30 bg-status-online/10 p-3">
              <p className="mb-1 text-sm font-semibold text-status-online">
                Token created! Copy it now — it won't be shown again.
              </p>
              <code className="block break-all rounded bg-bg-primary p-2 text-sm text-text-primary">
                {newToken}
              </code>
              <button
                onClick={() => {
                  navigator.clipboard.writeText(newToken);
                }}
                className="mt-2 rounded bg-bg-accent px-3 py-1 text-sm text-white transition-colors hover:bg-bg-accent-hover"
              >
                Copy to clipboard
              </button>
            </div>
          )}

          <div className="mb-3 flex gap-2">
            <input
              type="text"
              value={newTokenLabel}
              onChange={(e) => setNewTokenLabel(e.target.value)}
              placeholder="Token label (optional)"
              className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            />
            <button
              onClick={handleCreateToken}
              disabled={loading}
              className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover disabled:opacity-50"
            >
              Generate
            </button>
          </div>

          {tokens.length > 0 && (
            <div className="space-y-2">
              {tokens.map((t) => (
                <div
                  key={t.id}
                  className="flex items-center justify-between rounded-md bg-bg-tertiary p-3"
                >
                  <div>
                    <p className="text-sm font-medium text-text-primary">
                      {t.label || 'Unnamed token'}
                    </p>
                    <p className="text-xs text-text-muted">
                      Created {new Date(t.created_at).toLocaleDateString()}
                      {t.last_used && ` · Last used ${new Date(t.last_used).toLocaleDateString()}`}
                    </p>
                  </div>
                  <button
                    onClick={() => handleDeleteToken(t.id)}
                    className="rounded px-3 py-1 text-sm text-bg-danger transition-colors hover:bg-bg-danger/10"
                  >
                    Revoke
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* AT Protocol Settings */}
        <AtprotoSyncSection />

        {/* Logout */}
        <section>
          <button
            onClick={handleLogout}
            className="w-full rounded bg-bg-danger px-4 py-2 font-medium text-white transition-colors hover:bg-bg-danger/80"
          >
            Log Out
          </button>
        </section>
      </div>
    </Dialog>
  );
}

function AtprotoSyncSection() {
  const user = useAuthStore((s) => s.user);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const atprotoSyncEnabled = useChatStore((s) => s.atprotoSyncEnabled);
  const fetchAtprotoSyncSetting = useChatStore((s) => s.fetchAtprotoSyncSetting);
  const setAtprotoSyncEnabled = useChatStore((s) => s.setAtprotoSyncEnabled);
  const [toggling, setToggling] = useState(false);
  const [publications, setPublications] = useState<AtprotoPublicationStatus[]>([]);
  const [publicationError, setPublicationError] = useState<string | null>(null);
  const [channelPolicy, setChannelPolicy] = useState<import('../../api/types').AtprotoChannelPublicationPolicy | null>(null);

  const refreshPublications = useCallback(async () => {
    const accountId = useAuthStore.getState().user?.id;
    try {
      const next = await api.listAtprotoPublications();
      if (useAuthStore.getState().user?.id !== accountId) return;
      setPublications(next);
      setPublicationError(null);
    } catch (error) {
      if (useAuthStore.getState().user?.id !== accountId) return;
      setPublicationError(`Unable to load publication status: ${String(error)}`);
    }
  }, []);

  useEffect(() => {
    fetchAtprotoSyncSetting();
    queueMicrotask(() => void refreshPublications());
  }, [fetchAtprotoSyncSetting, refreshPublications]);

  useEffect(() => {
    let current = true;
    if (!activeChannel) return () => { current = false; };
    void api.getAtprotoChannelPublicationPolicy(activeChannel).then((policy) => {
      if (current) setChannelPolicy(policy);
    }).catch(() => { if (current) setChannelPolicy(null); });
    return () => { current = false; };
  }, [activeChannel]);

  const retryPublication = async (id: string) => {
    try {
      await api.retryAtprotoPublication(id);
      await refreshPublications();
    } catch (error) {
      setPublicationError(`Unable to retry publication: ${String(error)}`);
    }
  };

  const toggleChannelGrant = async () => {
    if (!channelPolicy) return;
    try {
      await api.setAtprotoPublicationGrant(channelPolicy.channel_id, !channelPolicy.user_granted);
      setChannelPolicy(await api.getAtprotoChannelPublicationPolicy(channelPolicy.channel_id));
    } catch (error) {
      setPublicationError(`Unable to change publication permission: ${String(error)}`);
    }
  };

  const handleToggle = async () => {
    setToggling(true);
    try {
      await setAtprotoSyncEnabled(!atprotoSyncEnabled);
    } catch (e) {
      console.error('Failed to toggle AT Protocol sync:', e);
    }
    setToggling(false);
  };

  return (
    <section className="mb-6">
      <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">
        AT Protocol
      </h3>
      <div className="rounded-md bg-bg-tertiary p-4">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-text-primary">
              Legacy AT Protocol preference
            </p>
            <p className="mt-1 text-xs text-text-muted">
              This preference alone never publishes messages. Publication requires an enabled public channel, your per-channel permission, and an explicit Share to Bluesky action.
            </p>
          </div>
          <button
            onClick={handleToggle}
            disabled={toggling}
            className={`relative h-6 w-11 rounded-full transition-colors ${
              atprotoSyncEnabled ? 'bg-blue-500' : 'bg-bg-input'
            } ${toggling ? 'opacity-50' : ''}`}
          >
            <span
              className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
                atprotoSyncEnabled ? 'translate-x-5' : ''
              }`}
            />
          </button>
        </div>
      </div>
      <div className="mt-3 rounded-md bg-bg-tertiary p-4">
        <p className="text-sm font-medium text-text-primary">Current channel permission</p>
        {!activeChannel || !channelPolicy ? (
          <p className="mt-1 text-xs text-text-muted">Select an eligible public channel to manage your publication permission.</p>
        ) : !channelPolicy.eligible ? (
          <p className="mt-1 text-xs text-text-muted">This channel cannot be published.</p>
        ) : !channelPolicy.channel_enabled ? (
          <p className="mt-1 text-xs text-text-muted">A channel manager must enable AT Protocol publication first.</p>
        ) : (
          <div className="mt-2 flex items-center justify-between gap-3">
            <p className="text-xs text-text-muted">Allow your explicitly selected messages in this channel to be public on Bluesky.</p>
            <button type="button" onClick={() => void toggleChannelGrant()} className="shrink-0 rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white">
              {channelPolicy.user_granted ? 'Revoke permission' : 'Allow publication'}
            </button>
          </div>
        )}
      </div>
      <div className="mt-3 rounded-md bg-bg-tertiary p-4">
        <div className="mb-2 flex items-center justify-between">
          <p className="text-sm font-medium text-text-primary">Publication history</p>
          <button type="button" onClick={() => void refreshPublications()} className="text-xs font-medium text-accent">Refresh</button>
        </div>
        {publicationError && <p role="alert" className="mb-2 text-xs text-red-300">{publicationError}</p>}
        {publications.length === 0 ? (
          <p className="text-xs text-text-muted">No messages have been submitted for publication.</p>
        ) : (
          <ul className="max-h-52 space-y-2 overflow-y-auto">
            {publications.map((publication) => (
              <li key={publication.id} className="rounded bg-bg-secondary p-2 text-xs text-text-secondary">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{publication.status.replaceAll('_', ' ')}</span>
                  <span className="text-text-muted">message {publication.source_message_id.slice(0, 8)}</span>
                </div>
                {publication.safe_error_code && <p className="mt-1 text-text-muted">{publication.safe_error_code}</p>}
                <div className="mt-1 flex gap-3">
                  {publication.remote_uri && <a href={publication.remote_uri} target="_blank" rel="noopener noreferrer" className="text-accent">View record</a>}
                  {publication.retryable && !publication.reauthentication_required && (
                    <button type="button" onClick={() => void retryPublication(publication.id)} className="text-accent">Retry safely</button>
                  )}
                  {publication.reauthentication_required && user && (
                    <a href={`/api/auth/atproto/login?handle=${encodeURIComponent(user.username)}`} className="text-accent">Reconnect AT Protocol</a>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
