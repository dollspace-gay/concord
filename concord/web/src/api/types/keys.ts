

// ── Helpers ─────────────────────────────────────────────

/** Composite key for channel-scoped data: "server_id:channel_name" */
export function channelKey(serverId: string, channel: string): string {
  return `${serverId}:${channel}`;
}
