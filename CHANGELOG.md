# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Security
- Add session revocation on logout (JWT blocklist) (#255)
- M3: Hash webhook/OAuth2/bot tokens and encrypt AT Protocol credentials (#173)
- M7: Add server-side HTML content sanitization (#177)
- M1: Add cleanup of pending AT Protocol OAuth state (#171)
- M10: Increase invite code length and add atomic use counting (#178)
- M2: Implement role hierarchy validation for permission assignment (#172)
- M4: Query IRC tokens by nickname instead of loading all (#174)
- Security audit remediation: fix 216 findings across all severity levels (#221)
- Add rate limiting on webhook execution endpoint to prevent argon2 DoS (#230)
- Fix IRC INVITE to check permissions instead of force-joining (#231)
- Security audit: remaining M/L findings remediation (#180)
- Security threat analysis and audit of full codebase (#152)
- Add per-IP connection limits and IRC command rate limiting (#200)
- H2: Add server membership checks to WS data operations (#160)
- H13: Prevent webhook username impersonation (#169)
- H12: Verify role belongs to server before update/delete (#168)
- H6: IRC line length limit, read timeout, per-IP connection limits (#163)
- H4: Add SSRF protection to embed unfurling and upload proxy (#162)
- H9: Enforce private channel access control (#166)

### Added — IRC Robustness (#198)
- CTCP ACTION (/me) support for IRC clients (#199)
- Per-IP connection limits (max 5) and per-connection command rate limiting (#200)
- KICK command for IRC moderators with permission checks (#201)
- AWAY/BACK presence mapping to engine status (#202)
- Complete WHOIS response with channel list (319) and away status (301) (#203)
- INVITE command to join users to channels (#204)
- Configurable MOTD via `[irc]` config section with on-demand MOTD command (#205)
- IRCv3 CAP negotiation with server-time, message-tags, and sasl capabilities (#206)
- SASL PLAIN authentication for IRC clients (#207)
- CTCP VERSION/PING/TIME responses (#208)
- Reply context prefix in PRIVMSG showing original author and preview (#209)
- Reaction formatting as CTCP ACTION for IRC visibility (#210)
- Role prefixes (@) in NAMES reply for server owners (#211)
- Attachment URLs sent as separate PRIVMSG lines for IRC clients

### Added — Server Management & Emoji UX (#148)
- Channel creation UI with "+" button on each category header
- Inline channel creation form with name, category, and private toggle
- Permission checks (MANAGE_CHANNELS) on channel create/delete
- ServerSettings expanded to 5 tabs: Overview, Channels, Roles, Categories, Emoji
- Server overview editing (rename, change icon URL) with UpdateServer WS command
- Emoji management panel (upload, list, delete) in server settings
- Custom server emoji tab in EmojiPicker with search support
- Emoji upload flow: file picker → PDS blob upload → create emoji

### Changed — AT Protocol Only (#149)
- Removed GitHub and Google OAuth providers — Bluesky login only
- Removed local disk storage fallback — all uploads require PDS blob storage
- Removed `oauth2` crate dependency
- Simplified login page to Bluesky handle input only
- Simplified auth config (no more provider sections)
- Upload endpoint returns 503 if PDS credentials are missing/expired

### Added — Phase 8: Integrations & Bots (#56)
- Webhook system (incoming POST endpoint + outgoing event subscriptions)
- Bot accounts with hashed API tokens and `Authorization: Bot <token>` auth
- Slash commands with options, autocomplete, and interaction dispatch
- Message components (buttons, select menus, action rows)
- OAuth2 application registration with authorization grants
- Rich embed format for bot messages
- IntegrationsPanel UI with Webhooks, Commands, Bots, and OAuth Apps tabs
- Public webhook execution endpoint: `POST /api/webhooks/{id}/{token}`
- Migration 012: bot_tokens, webhooks, webhook_events, slash_commands, interactions, oauth2_apps, oauth2_authorizations tables

### Added — Phase 7: Community & Discovery (#55)
- Invite links with configurable expiry and use limits
- Scheduled server events with RSVP (interested/going)
- Community settings (discovery, welcome message, rules text, category)
- Server discovery directory with category filtering
- Announcement channels with cross-posting via channel follows
- Server templates (snapshot and create-from-template)
- CommunityPanel UI with Invites, Events, Settings, and Discovery tabs
- Public REST endpoints: `GET /api/invite/{code}`, `GET /api/discover`
- Migration 011: invites, server_events, event_rsvps, channel_follows, server_templates tables

### Added — Phase 6: Moderation (#54)
- Kick members from servers with reason tracking
- Ban/unban members with optional message history deletion (0-7 days)
- Member timeout/mute with configurable duration
- Per-channel slow mode (configurable cooldown in seconds)
- Audit log capturing all mod actions, role changes, and server edits
- AutoMod system with keyword filter, mention spam detection, and link filter
- Configurable automod actions: delete, timeout, or flag
- Bulk message deletion (up to 100 messages)
- NSFW channel designation with age gate
- ModerationPanel UI with Bans, Audit Log, and AutoMod tabs
- Right-click context menu on members for kick/ban/timeout
- IRC NOTICE mapping for kick and ban events
- Migration 010: bans, audit_log, automod_rules tables

### Added — Phase 5: Threads & Pinning (#53)
- Message pinning with 50-per-channel limit
- Public threads spawned from messages
- Private threads with invite-only access
- Forum channels with tag-based categorization
- Personal message bookmarks with notes
- Thread auto-archive after inactivity
- IRC NOTICE mapping for pin/unpin and thread events
- Migration 009: pinned_messages, forum_tags, thread_tags, bookmarks tables

### Added — Phase 4: User Experience (#52)
- User presence status (online/idle/DND/invisible)
- Custom status with text and emoji
- User profiles with bio, pronouns, and banner
- Per-server display names (nicknames)
- Per-server and per-channel notification settings
- Browser desktop notifications
- Quick switcher (Ctrl+K) with fuzzy search
- Message search with FTS5 and filter operators (from:, in:, has:, before:, after:)
- Migration 008: user_presence, user_profiles, notification_settings tables; FTS5 virtual table

### Added — Phase 3: Organization & Permissions (#51)
- Custom roles with bitfield permissions (u64, 20+ permission flags)
- Channel categories with collapsible sections
- Drag-and-drop channel reordering
- Private channels with membership-based access control
- Channel permission overrides (per-role and per-user)
- Role colors displayed in chat and member list
- Server folders (client-side, localStorage-persisted)
- ServerSettings panel with roles and channels tabs
- Effective permission algorithm mirroring Discord's model
- Migration 007: roles, user_roles, channel_categories, channel_permission_overrides tables

### Added — Phase 2: Media & Files (#50)
- File and image upload system with configurable storage
- Image preview and lightbox viewer
- Inline video and audio players
- Link embed previews via Open Graph unfurling
- Custom server emoji with :name: rendering
- GIF support and optional Tenor GIF picker
- Voice/audio message recording with waveform playback
- AT Protocol PDS blob storage for Bluesky users
- Migration 004-006: attachments, embed_cache, custom_emoji tables

### Added — Phase 1: Core Messaging (#49)
- Message editing with "edited" indicator
- Message deletion (soft delete)
- Markdown and text formatting (bold, italic, code, blockquotes, spoilers)
- @mentions with notification highlighting
- Reply/quote to specific messages
- Emoji reactions on messages
- Typing indicators (ephemeral, 8s auto-expire)
- Read state and unread indicators
- Migration 003: edited_at, deleted_at, reply_to_id on messages; mentions, reactions, read_states tables

### Added — Foundation
- Admin bootstrap via config (#140)
- Multi-server architecture with server-aware ChatEngine
- React 19 + TypeScript + Vite + Tailwind 4 frontend
- AT Protocol (Bluesky) OAuth authentication with PDS blob storage
- IRC protocol support (RFC 2812 parser, TCP listener, command dispatch)
- SQLite persistence with WAL mode
- Chat engine with protocol-agnostic event system
- WebSocket handler with axum router

### Fixed
- Fix voice message playback - PDS blob MIME mismatch and redirect proxy (#147)
- Fix IRC MODE to reflect actual channel modes (private, slow, NSFW) (#262)
- Fix fire-and-forget DB writes — await critical message persistence (#249)
- Fix frontend partial upload failure losing all files (#272)
- Fix IRC WHO response field ordering per RFC 2812 (#263)
- Add batch chunking for dynamic IN-clause queries exceeding 999 params (#279)
- Fix split_sql_statements not handling semicolons in string literals (#281)
- Allow dots in nicknames for Bluesky handles (#197)
- Fix race conditions in DashMap check-then-act patterns (sessions, channels) (#247)
- Fix typing indicator timeout not resetting on re-type (#269)
- Fix Virtuoso startReached firing multiple duplicate history fetches (#273)
- Fix PDS blob serving by creating AT Protocol record to pin uploaded blobs (#144)
- Fix broken PDS blob URL missing did parameter (#135)
- Fix blank uploaded images - local message missing attachments (#126)
- Fix missing migration 4 and add emoji picker (#125)

### Changed
- Expand smoke test to cover all web and IRC features (#212)
- Multi-server polish and hardening (#45)
- Run linters on frontend and backend, fix issues (#213)
- Make Concord AT Protocol-centric (remove GitHub/Google OAuth) (#132)
- Feature: Animated avatars verification (#220)
- Feature: Custom stickers (#219)
- Feature: Cross-server emoji (#218)
- Feature: Per-server avatars (#217)
- Feature: Vanity invite URLs (#216)
- Feature: Configurable message and upload limits (#215)
- Migration 015: Premium-for-free database schema (#214)
- Update README with install instructions and ngrok setup (#143)
- Clean repo of build artifacts (#136)
- Persist AT Protocol signing key across server restarts (#133)
