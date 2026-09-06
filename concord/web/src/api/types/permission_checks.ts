

// ── Permission bitfield constants ──────────────────────
export const Permissions = {
  VIEW_CHANNELS: 1 << 0,
  MANAGE_CHANNELS: 1 << 1,
  MANAGE_ROLES: 1 << 2,
  MANAGE_SERVER: 1 << 3,
  CREATE_INVITES: 1 << 4,
  KICK_MEMBERS: 1 << 5,
  BAN_MEMBERS: 1 << 6,
  ADMINISTRATOR: 1 << 7,
  SEND_MESSAGES: 1 << 10,
  EMBED_LINKS: 1 << 11,
  ATTACH_FILES: 1 << 12,
  ADD_REACTIONS: 1 << 13,
  MENTION_EVERYONE: 1 << 14,
  MANAGE_MESSAGES: 1 << 15,
  READ_MESSAGE_HISTORY: 1 << 16,
} as const;

export function hasPermission(perms: number, flag: number): boolean {
  if (perms & Permissions.ADMINISTRATOR) return true;
  return (perms & flag) === flag;
}
