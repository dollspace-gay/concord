# Concord

An open-source, self-hostable chat platform with native IRC compatibility and a modern web UI.

Any IRC client (HexChat, irssi, WeeChat) connects alongside web users — messages flow seamlessly between protocols.

## Features

- **Multi-server architecture**: Discord-style servers with isolated channels, members, and permissions
- **Dual protocol**: WebSocket (browser) + IRC (RFC 2812) on the same instance
- **Protocol-agnostic engine**: Core chat logic never imports protocol-specific code
- **OAuth authentication**: GitHub, Google, and Bluesky (AT Protocol) login
- **Role-based permissions**: Owner, Admin, Moderator, and Member roles per server
- **IRC access tokens**: Web users generate argon2-hashed tokens to connect from any IRC client
- **Persistent history**: SQLite (WAL mode) with paginated message history
- **Rate limiting**: Token-bucket rate limiter on messages (per-user)
- **Direct messages**: Cross-protocol DMs between any connected users
- **Modern web UI**: React + TypeScript with a Discord-like layout
- **Self-hostable**: Single binary + static files, or use Docker

## Quick Start

### Prerequisites

- Rust 1.84+ (for the server)
- Node.js 22+ (for the web frontend)

### Build from source

```bash
# Build the frontend
cd web
npm ci
npm run build
cp -r dist/* ../server/static/
cd ..

# Build the server
cd server
cargo build --release
```

### Run

```bash
# From the server directory
../target/release/concord-server
```

The server starts on:
- **Web UI**: http://localhost:8080
- **IRC**: localhost:6667

### Docker

```bash
# Copy and edit the config
cp concord.example.toml concord.toml

# Build and run
docker compose up -d
```

## Configuration

Concord loads configuration from `concord.toml` (see `concord.example.toml`). Environment variables override TOML values.

| Setting | Env Variable | Default |
|---|---|---|
| Web listen address | `WEB_ADDRESS` | `0.0.0.0:8080` |
| IRC listen address | `IRC_ADDRESS` | `0.0.0.0:6667` |
| Database URL | `DATABASE_URL` | `sqlite:concord.db?mode=rwc` |
| JWT secret | `JWT_SECRET` | `concord-dev-secret-change-me` |
| Session expiry | `SESSION_EXPIRY_HOURS` | `720` (30 days) |
| Public URL | `PUBLIC_URL` | `http://localhost:8080` |
| GitHub OAuth | `GITHUB_CLIENT_ID` / `GITHUB_CLIENT_SECRET` | — |
| Google OAuth | `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` | — |

Bluesky login requires no configuration — it uses the AT Protocol OAuth flow with your instance's public URL.

## IRC Usage

1. Log in via the web UI (OAuth)
2. Go to Settings and generate an IRC access token
3. Connect your IRC client:

```
Server: your-server-address
Port: 6667
Password: <your-token>
Nickname: <your-username>
```

In HexChat, set the server password to your token. Concord validates the token and maps you to your web account.

### Multi-server channels over IRC

IRC clients can join channels on non-default servers using the `#server-name/channel` syntax:

```
/join #general            → default server, #general
/join #my-guild/general   → "my-guild" server, #general
```

## Architecture

```
IRC Clients ──TCP──▸ ┌─────────────────────┐ ◂──WS── Web Browsers
                     │     Rust Server      │
                     │  ┌─────────────────┐ │
                     │  │  IRC Adapter    │ │
                     │  ├─────────────────┤ │
                     │  │  Chat Engine    │ │  ← protocol-agnostic
                     │  │  (multi-server, │ │
                     │  │   permissions,  │ │
                     │  │   rate limits)  │ │
                     │  ├─────────────────┤ │
                     │  │  WS / HTTP API  │ │
                     │  ├─────────────────┤ │
                     │  │    SQLite       │ │
                     │  └─────────────────┘ │
                     └─────────────────────┘
```

Dependency direction: `irc` → `engine` ← `web`. The engine knows nothing about protocols.

## Tech Stack

| Layer | Technology |
|---|---|
| Backend | Rust (tokio, axum, sqlx) |
| Frontend | React 19, TypeScript, Vite, Zustand, Tailwind CSS 4 |
| Database | SQLite (WAL mode) |
| IRC | Custom RFC 2812 parser and formatter |
| Auth | OAuth2 (GitHub, Google, Bluesky/AT Protocol), JWT sessions, argon2 IRC tokens |
| Concurrency | DashMap, tokio mpsc channels |

## REST API

All endpoints are under `/api`. Authenticated endpoints require a `concord_session` cookie (set by OAuth login).

### Public
- `GET /api/auth/status` — available OAuth providers
- `GET /api/channels?server_id=` — list channels
- `GET /api/channels/{name}/messages?server_id=` — message history
- `GET /api/users/{nickname}` — public profile lookup

### Authenticated
- `GET /api/me` — current user profile
- `GET /api/servers` — list your servers
- `POST /api/servers` — create a server
- `GET /api/servers/{id}` — server info
- `DELETE /api/servers/{id}` — delete server (owner only)
- `GET /api/servers/{id}/channels` — list channels in server
- `GET /api/servers/{id}/channels/{name}/messages` — channel history
- `GET /api/servers/{id}/members` — list server members
- `GET /api/tokens` — list your IRC tokens
- `POST /api/tokens` — generate an IRC token
- `DELETE /api/tokens/{id}` — revoke an IRC token

### Admin
- `GET /api/admin/servers` — list all servers
- `DELETE /api/admin/servers/{id}` — delete any server
- `PUT /api/admin/users/{id}/admin` — toggle system admin flag

## Development

```bash
# Run the server (with hot reload via cargo-watch)
cd server
cargo watch -x run

# Run the frontend dev server (proxies API to :8080)
cd web
npm run dev
```

### Running tests

```bash
cd server
cargo test
```

42 tests covering the chat engine, IRC parser/formatter, JWT auth, and token hashing.

## License

MIT
