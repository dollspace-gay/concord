# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- Phase 5: React Frontend (#14)
- Phase 4: OAuth Authentication (#13)
- Phase 3: IRC Protocol — RFC 2812 parser, TCP listener, command dispatch, numeric replies (#12)
- Phase 2: Database and Persistence — SQLite with WAL mode, message/channel/membership persistence (#11)
- Phase 1: Chat Engine + WebSocket foundation (#2)
- Plan Concord - open source IRC-compatible chat platform (#1)

### Fixed

### Changed
- Frontend UI components for multi-server (#44)
- Frontend stores + types for multi-server support (#43)
- IRC protocol adaptation for multi-server channels (#42)
- WebSocket + REST protocol update for multi-server (#41)
- ChatEngine refactoring for server-aware architecture (#40)
- Database schema migration for multi-server support (#39)
- Wire frontend build output into Rust server static serving (#36)
- Initialize Vite + React + TypeScript + Tailwind project (#35)
- Build and test OAuth authentication (#34)
- Add user profile and token management REST endpoints (#33)
- Implement IRC PASS token validation (#32)
- Implement auth middleware for REST and WebSocket (#31)
- Implement session/token management (JWT, IRC tokens) (#30)
- Implement OAuth provider config and flow handlers (#29)
- Add OAuth and auth dependencies (oauth2, argon2, jsonwebtoken) (#28)
- Build and verify two-tab chat works (#10)
- Create HTML test page for WebSocket testing (#9)
- Implement main.rs entry point and config (#8)
- Implement WebSocket handler and axum router (#7)
- Implement ChatEngine core with message routing (#6)
- Implement UserSession and Channel state types (#5)
- Implement ChatEvent enum and core types (#4)
- Initialize Cargo workspace and project structure (#3)
- Build and test IRC protocol with cargo test (#27)
- Implement PING/PONG keepalive and registration flow (#26)
- Implement IRC command dispatch (NICK, USER, JOIN, PART, PRIVMSG, etc.) (#25)
- Implement IRC TCP listener and connection handler (#24)
- Implement IRC numeric reply constants (#23)
- Implement IRC message parser and formatter (RFC 2812) (#22)
- Add channel and membership persistence (#21)
- Add REST endpoint for message history (#20)
- Wire persistence into ChatEngine (#19)
- Implement db module with models and query functions (#18)
- Add sqlx + SQLite dependencies and create migration files (#17)
