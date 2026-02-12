# Concord

An open-source, self-hostable chat platform with native IRC compatibility and a modern web UI.

Any IRC client (HexChat, irssi, WeeChat) connects alongside web users — messages flow seamlessly between protocols.

## Features

### Core
- **Multi-server architecture** — Discord-style servers with isolated channels, members, and permissions
- **Dual protocol** — WebSocket (browser) + IRC (RFC 2812) on the same instance
- **Protocol-agnostic engine** — core chat logic never imports protocol-specific code
- **AT Protocol authentication** — sign in with any Bluesky account, files stored as PDS blobs
- **Self-hostable** — single binary + static files, or use Docker

### Messaging
- Message editing, deletion, and reply threads
- Markdown formatting (bold, italic, code, blockquotes, spoilers)
- @mentions with notification highlighting
- Emoji reactions and custom server emoji
- Typing indicators and read state tracking
- File uploads with image/video/audio preview
- Link embed previews via Open Graph
- Message pinning (50 per channel)
- Full-text search with filter operators (`from:`, `in:`, `has:`, `before:`, `after:`)

### Organization
- Channel categories with collapsible sections
- Drag-and-drop channel reordering
- Private channels with membership-based access
- Custom roles with bitfield permissions (20+ flags)
- Channel permission overrides (per-role and per-user)
- Role colors in chat and member list
- Client-side server folders

### Threads & Forums
- Public and private threads spawned from messages
- Forum channels with tag-based categorization
- Thread auto-archive after inactivity
- Personal message bookmarks with notes

### Moderation
- Kick, ban, and timeout members
- Per-channel slow mode
- Audit log for all moderation actions
- AutoMod with keyword filter, mention spam detection, and link filter
- Bulk message deletion
- NSFW channel designation

### Community
- Invite links with expiry and use limits
- Scheduled events with RSVP
- Server discovery directory
- Announcement channels with cross-posting
- Customizable welcome screen and rules
- Server templates

### Integrations
- Incoming and outgoing webhooks
- Bot accounts with API tokens
- Slash commands with autocomplete
- Message components (buttons, select menus)
- OAuth2 application registration
- Rich embed format for bot messages

### User Experience
- Presence status (online, idle, DND, invisible)
- Custom status with text and emoji
- User profiles with bio, pronouns, and banner
- Per-server display names
- Per-server and per-channel notification settings
- Browser desktop notifications
- Quick switcher (Ctrl+K)

## Prerequisites

| Dependency | Version | Purpose |
|---|---|---|
| [Rust](https://www.rust-lang.org/tools/install) | 1.84+ | Server compilation |
| [Node.js](https://nodejs.org/) | 22+ | Frontend build |
| [Git](https://git-scm.com/) | any | Clone the repo |

**Optional:**

| Tool | Purpose |
|---|---|
| [ngrok](https://ngrok.com/) | Expose local server for AT Protocol OAuth and mobile testing |
| [Docker](https://www.docker.com/) | Containerized deployment |
| [cargo-watch](https://crates.io/crates/cargo-watch) | Auto-restart server on code changes |

## Installation

### 1. Clone the repository

```bash
git clone https://github.com/your-org/concord.git
cd concord
```

### 2. Build the frontend

```bash
cd concord/web
npm ci
npm run build
```

This produces a `dist/` directory with the compiled React app.

### 3. Copy frontend assets to the server

The Rust server serves the frontend as static files from its `static/` directory:

```bash
# From the repository root
cp -r concord/web/dist/* concord/server/static/
```

On Windows (PowerShell):
```powershell
Copy-Item -Recurse concord/web/dist/* concord/server/static/
```

### 4. Configure the server

```bash
cd concord/server
cp ../concord.example.toml concord.toml
```

Edit `concord.toml`:

```toml
[server]
web_address = "0.0.0.0:8080"
irc_address = "0.0.0.0:6667"

[database]
url = "sqlite:concord.db?mode=rwc"

[auth]
jwt_secret = "change-me-to-a-random-secret"   # CHANGE THIS
session_expiry_hours = 720                      # 30 days
public_url = "http://localhost:8080"            # or your ngrok/production URL

[storage]
max_file_size_mb = 100

[admin]
admin_users = []   # usernames auto-promoted to system admin
```

Every TOML value can be overridden with an environment variable (see [Configuration Reference](#configuration-reference)).

### 5. Build and run the server

```bash
cd concord/server
cargo build --release
../target/release/concord-server
```

On Windows:
```powershell
.\target\release\concord-server.exe
```

The server starts on:
- **Web UI**: http://localhost:8080
- **IRC**: localhost:6667

### Docker

```bash
cd concord

# Copy and edit the config
cp concord.example.toml concord.toml
# Edit concord.toml with your settings

# Build and run
docker compose up -d
```

The Docker image is a multi-stage build (Rust compile + Node build + slim Debian runtime). Data is persisted in a named volume.

## Development Setup

### Running the frontend dev server

The Vite dev server provides hot module replacement and proxies API/WebSocket requests to the Rust backend:

```bash
cd concord/web
npm install
npm run dev
```

This starts Vite on http://localhost:3000, proxying `/api` and `/ws` to `http://localhost:8080`.

### Running the backend with auto-reload

```bash
cd concord/server
cargo install cargo-watch   # one-time
cargo watch -x run
```

### ngrok Setup (for AT Protocol OAuth and external access)

AT Protocol OAuth requires a publicly reachable callback URL. During local development, [ngrok](https://ngrok.com/) provides a stable HTTPS tunnel to your machine.

#### 1. Install ngrok

**macOS:**
```bash
brew install ngrok
```

**Windows (winget):**
```powershell
winget install ngrok.ngrok
```

**Windows (Chocolatey):**
```powershell
choco install ngrok
```

**Linux:**
```bash
curl -sSL https://ngrok-agent.s3.amazonaws.com/ngrok.asc \
  | sudo tee /etc/apt/trusted.gpg.d/ngrok.asc >/dev/null
echo "deb https://ngrok-agent.s3.amazonaws.com buster main" \
  | sudo tee /etc/apt/sources.list.d/ngrok.list
sudo apt update && sudo apt install ngrok
```

Or download directly from https://ngrok.com/download.

#### 2. Authenticate ngrok

Sign up at https://dashboard.ngrok.com and copy your auth token:

```bash
ngrok config add-authtoken YOUR_AUTH_TOKEN
```

#### 3. Reserve a free static domain (recommended)

Free ngrok accounts can reserve one static domain, which gives you a stable URL that doesn't change between restarts:

1. Go to https://dashboard.ngrok.com/domains
2. Click **Create Domain** — you'll get something like `your-name-here.ngrok-free.dev`

#### 4. Start the tunnel

```bash
# With a reserved static domain (recommended)
ngrok http --url=your-name-here.ngrok-free.dev 8080

# Or with a random URL (changes every restart)
ngrok http 8080
```

#### 5. Configure Concord to use the ngrok URL

Set `public_url` in `concord.toml` to your ngrok domain:

```toml
[auth]
public_url = "https://your-name-here.ngrok-free.dev"
```

Or via environment variable:
```bash
PUBLIC_URL=https://your-name-here.ngrok-free.dev cargo run
```

This is required so that:
- AT Protocol OAuth callback URLs point to the correct host
- Session cookies are set with the `Secure` flag (ngrok uses HTTPS)
- The DPoP-bound token flow can complete successfully

No additional OAuth app registration is needed — AT Protocol auto-discovers your server's client metadata at `/api/auth/atproto/client-metadata.json`. Just ensure `public_url` is set correctly.

### Running tests

```bash
cd concord/server
cargo test
```

742 tests covering the chat engine, IRC parser/formatter, JWT auth, token hashing, permissions, moderation, integrations, and community features.

## Configuration Reference

Concord loads configuration from `concord.toml`. Environment variables override TOML values.

| Setting | TOML Path | Env Variable | Default |
|---|---|---|---|
| Web listen address | `server.web_address` | `WEB_ADDRESS` | `0.0.0.0:8080` |
| IRC listen address | `server.irc_address` | `IRC_ADDRESS` | `0.0.0.0:6667` |
| Database URL | `database.url` | `DATABASE_URL` | `sqlite:concord.db?mode=rwc` |
| JWT secret | `auth.jwt_secret` | `JWT_SECRET` | `concord-dev-secret-change-me` |
| Session expiry | `auth.session_expiry_hours` | `SESSION_EXPIRY_HOURS` | `720` (30 days) |
| Public URL | `auth.public_url` | `PUBLIC_URL` | `http://localhost:8080` |
| Max file size (MB) | `storage.max_file_size_mb` | `MAX_FILE_SIZE_MB` | `100` |
| Admin users | `admin.admin_users` | `ADMIN_USERS` (comma-separated) | `[]` |

## IRC Usage

1. Log in via the web UI (Bluesky / AT Protocol)
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
                     └──────────────────────┘
```

Dependency direction: `irc` → `engine` ← `web`. The engine knows nothing about protocols.

## Tech Stack

| Layer | Technology |
|---|---|
| Backend | Rust (tokio, axum, sqlx) |
| Frontend | React 19, TypeScript, Vite, Zustand, Tailwind CSS 4 |
| Database | SQLite (WAL mode) |
| IRC | Custom RFC 2812 parser and formatter |
| Auth | AT Protocol (Bluesky) OAuth, JWT sessions, argon2 IRC tokens |
| Concurrency | DashMap, tokio mpsc channels |

## REST API

All endpoints are under `/api`. Authenticated endpoints require a `concord_session` cookie (set by AT Protocol login).

### Public
- `GET /api/auth/status` — authentication provider info
- `GET /api/invite/{code}` — invite link preview
- `GET /api/discover` — server discovery directory

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
- `GET /api/channels?server_id=` — list channels
- `GET /api/channels/{name}/messages?server_id=` — message history
- `GET /api/users/{nickname}` — public profile lookup

### Admin
- `GET /api/admin/servers` — list all servers
- `DELETE /api/admin/servers/{id}` — delete any server
- `PUT /api/admin/users/{id}/admin` — toggle system admin flag

## License

MIT
