# IRC stream correctness implementation evidence

## Scope

This change implements the transport-local portion of R08/G08 and the S1/S4 IRC work:
bounded incremental inbound framing, deterministic malformed/overlong input handling,
registration and heartbeat deadlines, writer/queue failure propagation, listener
cancellation, connection-task ownership, and per-IP counter cleanup.

It does not claim the complete G08 gate. TLS/manual client qualification, stable alias
storage, durable direct conversations, cross-protocol multi-connection identity, and
reconnect/history behavior depend on later auth, engine, data, and operator work.

## Repair classification and behavior

- **Root-cause repair — framing stall:** `IrcLineDecoder` owns buffered bytes and awaits
  new reads only after consuming the bytes already received. Representative PASS/NICK/USER
  commands are decoded over a Tokio duplex stream at every byte split position.
- **Root-cause repair — framing bounds:** the 4096-byte cap includes CRLF or LF. A line
  exactly at the bound is accepted and one byte over is rejected. A partial line at EOF
  is distinct from clean EOF.
- **Local correctness repair — UTF-8 boundary:** decoding happens only after a complete
  byte-framed line exists. Split multibyte text succeeds; malformed UTF-8 rejects its own
  line while a following complete line remains available in decoder state.
- **Root-cause repair — lifecycle:** listener cancellation reaches accepted plain/TLS
  connections. The listener owns connection tasks in a `JoinSet`, waits for them at
  shutdown, and holds per-IP accounting in an RAII guard. Engine disconnect remains on
  the common connection exit path.
- **Root-cause repair — outbound stalls:** full/closed outbound queues and writer I/O
  failure cancel the connection loop. Cleanup drops its engine event receiver and sender,
  then gives the writer a bounded drain before aborting and observing it.
- **Local correctness repair — heartbeat/registration:** unauthenticated registration has
  a single 60-second deadline. Registered connections receive server PING probes and only
  the matching PONG nonce clears the deadline.

Bare LF remains accepted for compatibility with the previous reader. Terminators are
removed before command parsing. Malformed input and oversized lines close the connection
deterministically; the decoder-level preservation guarantee is tested for malformed lines.

## Verification

- `cargo test -p concord-server --test irc_streams` — **verified**, 7 passed.
- `cargo test -p concord-server --lib irc::` — **verified**, 138 passed.
- `cargo fmt --all -- --check` — **failed due to concurrent, out-of-scope formatting in
  `server/src/contract.rs` and `server/src/db/pool.rs`; the owned IRC files were formatted
  directly with rustfmt.**
- `cargo clippy -p concord-server --all-targets --all-features -- -D warnings` — **failed
  on out-of-scope warnings in `server/src/db/pool.rs` and pre-existing clone-to-slice
  findings in `server/src/integration_tests.rs`; no IRC finding was emitted.**

No dependency was added and no manifest change is required.
