# Concord

Open-source, self-hostable, IRC-compatible chat platform with a modern web UI.

## Architecture

Single Rust binary with two protocol interfaces feeding into one shared engine:
- **IRC listener** (TCP, RFC 2812) — traditional IRC clients connect here
- **WebSocket/HTTP API** (axum) — React web frontend connects here
- **ChatEngine** — protocol-agnostic core. Never imports IRC or web modules.

Dependency direction: `irc` → `engine` ← `web`. The engine knows nothing about protocols.

## Project Structure

```
concord/
  server/             Rust backend (cargo workspace member)
    src/
      engine/         Core chat logic (ChatEngine, events, sessions, channels)
      irc/            IRC protocol adapter (parser, formatter, listener)
      web/            HTTP/WebSocket adapter (axum router, handlers)
      db/             Database layer (sqlx, migrations, queries)
      auth/           Authentication (OAuth, tokens)
    static/           Static files served by axum (test HTML client)
  web/                React frontend (TypeScript, Vite, Tailwind)
```

## Build & Run

```bash
# Server
cd server
cargo build
cargo run              # Starts on 0.0.0.0:8080
cargo test             # Run all tests

# Frontend (once implemented)
cd web
npm install
npm run dev
```

## Key Conventions

- **Channel names** are always lowercase, prefixed with `#`. The engine normalizes them.
- **SessionId** = UUID v4, unique per connection (not per user).
- **ChatEvent** is the universal event type. Protocol adapters serialize it to their wire format.
- Each session has an `mpsc::UnboundedSender<ChatEvent>` for outbound messages.
- **DashMap** for concurrent state — no global mutexes.
- **SQLite** (WAL mode) is the default database. Schema supports PostgreSQL swap.
- All database queries use **parameterized** statements via sqlx. Never interpolate user input.
- IRC tokens are hashed with **argon2** before storage. Never log raw tokens.

## Testing

- Unit tests live alongside their modules in `#[cfg(test)]` blocks.
- Run `cargo test` from the workspace root.
- Cross-protocol tests: verify messages sent from IRC appear on WebSocket and vice versa.

## Environment Variables

- `RUST_LOG` — tracing filter (default: `info`). Example: `RUST_LOG=debug cargo run`
- `DATABASE_URL` — SQLite or PostgreSQL connection string (when db is wired up)

## Security Notes

- OAuth tokens for IRC: web UI generates them, IRC clients use them as PASS.
- CORS is permissive in dev. Lock down `allow_origin` for production.
- Validate/sanitize all user input at protocol boundaries.
- Rate limiting applies at both IRC and HTTP layers.
