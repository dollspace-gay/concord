# Concord

An open-source, self-hostable chat platform with native IRC compatibility and a modern web UI.

Any IRC client (HexChat, irssi, WeeChat) connects alongside web users — messages flow seamlessly between protocols.

## Features

- **Dual protocol**: WebSocket (browser) + IRC (RFC 2812) on the same server
- **Protocol-agnostic engine**: Core chat logic never imports protocol-specific code
- **OAuth authentication**: GitHub and Google login for the web UI
- **IRC access tokens**: Web users generate tokens to connect from any IRC client
- **Persistent history**: SQLite (WAL mode) with message history and channel persistence
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
cp -r dist/ ../server/static/
cd ..

# Build the server
cd server
cargo build --release
```

### Run

```bash
# From the server directory
./target/release/concord-server
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

## Architecture

```
IRC Clients ──TCP──▸ ┌─────────────────┐ ◂──WS── Web Browsers
                     │   Rust Server    │
                     │  ┌─────────────┐ │
                     │  │ IRC Adapter  │ │
                     │  ├─────────────┤ │
                     │  │ Chat Engine  │ │  ← protocol-agnostic
                     │  ├─────────────┤ │
                     │  │  WS / HTTP   │ │
                     │  ├─────────────┤ │
                     │  │   SQLite     │ │
                     │  └─────────────┘ │
                     └─────────────────┘
```

## Tech Stack

| Layer | Technology |
|---|---|
| Backend | Rust (tokio, axum, sqlx) |
| Frontend | React 19, TypeScript, Vite, Zustand, Tailwind CSS |
| Database | SQLite (WAL mode) |
| IRC | Custom RFC 2812 implementation |
| Auth | OAuth2 (GitHub, Google), JWT sessions |

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

## License

MIT
