# Durable shared credential authority implementation evidence

## Delivered bounded slice

Migration 018 introduces one authoritative `auth_credentials` ledger for web sessions,
IRC tokens, and bot tokens. Each actor carries a private stable user ID, credential ID,
credential kind, scopes, expiry, and version. Authority construction is confined to
successful authentication; adapters receive getters but cannot replace actor fields.

Web JWT issuance records its `jti` before returning the cookie. JWT signature/expiry is
necessary but insufficient: every HTTP extraction and WebSocket upgrade also loads the
matching active database record and user state. Unregistered legacy JWTs therefore require
login again. Logout commits revocation before clearing the cookie, and that revocation
survives service/process reconstruction.

New IRC and bot secrets contain a nonsecret indexed token ID and are hashed with Argon2 on
a bounded blocking worker pool. Existing IRC tokens remain compatible through a
nickname/alias-scoped `DISTINCT` lookup. Existing `bot_<user-id>.<secret>` tokens use their
embedded stable user hint; malformed legacy forms fall back to a hard maximum of 32
candidates. Verification saturation fails closed. No new production credential uses the
legacy scan path.

`Actor` revalidation checks current principal, kind, scopes, version, expiry, revocation,
and account disablement. `validate_actor_in(&mut SqliteConnection, &Actor)` performs the
same check on a caller-owned transaction connection for the actor-scoped authorization
slice. Database triggers require a credential version increment for scope, expiry, or
revocation changes and make credential identity/hash fields immutable.

HTTP, WebSocket, IRC, and bot extractors/call sites use `AuthService`. WebSocket upgrades
require an exact configured `Origin` before cookie authentication. WebSocket command
dispatch and final event serialization revalidate credentials. IRC command dispatch,
event projection, and the final socket writer revalidate them. Live connections hold
bounded reference-counted credential leases; revocation cancels every matching live lease,
idle connections also stop at expiry, and the last disconnect removes registry state.

Bot creation records a stable owner. Only that owner can issue, list, or revoke bot
credentials. Historical bots receive an explicit `repair_required` ownership row with no
guessed owner; server management/install membership does not become credential ownership.

## Verification evidence

- `cargo test --test session_authority` — **verified 16/16**. It covers durable restart,
  rejected unregistered JWTs, expiry, disabled accounts, durable/live revocation, indexed
  IRC/bot authentication, legacy bot user hints, changed IRC handles, real HTTP and
  WebSocket-upgrade rejection, WebSocket Origin and cookie-mutation rejection, HTTP logout,
  accepted live WebSocket logout/revocation, durable-expiry command rejection, shared
  shutdown cleanup, live IRC engine-session cleanup, stable bot ownership, already-expired
  timer behavior, and concurrent issue/revoke-all linearization.
- `cargo test --test irc_streams` — **verified 7/7** after the authority integration.
- `cargo test --lib` — **verified 782/782** after the authority integration.
- `cargo clippy --all-targets --all-features -- -D warnings` reaches an unrelated existing
  `clippy::derivable_impls` failure in `server/src/config.rs`; it reports no authority-slice
  finding before stopping.
- `cargo fmt --all --check` is currently blocked by formatting in concurrent authorization
  edits (`channels.rs`, `chat_engine.rs`, and `irc/commands.rs`). The authority-owned files
  were formatted directly and pass `git diff --check`.

## Remaining design scope

This is not full G01, G12, or G20. Provider credential encryption/rotation/recovery,
configuration secret persistence and bootstrap qualification, delegated OAuth grants,
bot installation grants and scoped application routes, full TLS/manual IRC qualification,
and actor-scoped domain authorization remain in their assigned slices. The tests exercise
real negative and accepted WebSocket handshakes, live logout/revocation/expiry/shutdown
cleanup, and a real live IRC revocation journey. An application route consuming `BotAuth`
remains for the S7 bot application slice, along with the final stable-tree format/lint gate.
