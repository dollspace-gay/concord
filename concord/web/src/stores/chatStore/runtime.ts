

/** Tracks per-user typing indicator timeouts so they can be cleared on re-type. */
export const typingTimeouts = new Map<string, ReturnType<typeof setTimeout>>();

export const retriedCommands = new Set<string>();

export const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();

export const pendingCommandOwners = new Map<string, number>();

export const recoveredThisConnection = new Set<string>();

/** Prevent repeated channel-list projections from refetching identical server bootstrap data. */
export const hydratedServerMetadata = new Set<string>();

export const draftsByAccount = new Map<string, Record<string, string>>();
