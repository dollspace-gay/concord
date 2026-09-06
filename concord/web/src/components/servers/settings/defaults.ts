import type { CategoryInfo, ChannelInfo, ForumTagInfo, RoleInfo, StickerInfo } from '../../../api/types';

export const EMPTY_ROLES: RoleInfo[] = [];

export const EMPTY_CATEGORIES: CategoryInfo[] = [];

export const EMPTY_CHANNELS: ChannelInfo[] = [];

export const EMPTY_EMOJI: Record<string, { id: string; image_url: string }> = {};

export const EMPTY_STICKERS: StickerInfo[] = [];

export const EMPTY_FORUM_TAGS: Record<string, ForumTagInfo[]> = {};

export const EMPTY_MEMBER_ROLES: Record<string, string[]> = {};
