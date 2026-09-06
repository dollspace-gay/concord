# S3 transactional messaging and replay architecture

## Objective and boundaries

S3 makes every production channel-message mutation durable before acknowledgement and
recoverable after connection loss. WebSocket, IRC, incoming-webhook, and non-ephemeral
interaction-response ingress call one asynchronous `MessagingService`; edit, delete,
reaction, and read-state operations use the same transaction owner. Each accepted command
atomically records canonical state, an idempotency receipt, entity version, and a durable
event descriptor. Fanout is only a post-commit optimization.

Migration 020 owns generic conversations and channel-message metadata. A conversation may
be `channel` or `direct`; server/channel columns are nullable so migration 022 can add the
canonical direct pair, participants, blocks, and preferences without fake default-server
rows. S3 creates and backfills channel conversations. Historical ambiguous/orphan rows stay
nullable and are recorded for repair. New S3 writes require a resolved conversation.
Migration 021 owns command receipts, entity versions, the durable event log, and dispatcher
state. Message IDs remain unchanged. Per-conversation and global sequences are stored as
SQLite integers and serialized as decimal strings.

S4 owns multi-connection indexes, aliases, and DM participant policy. S5 owns private media
storage. S3 validates existing staged attachment ownership and atomically links accepted
attachments, without inventing the later media state machine. Bot application routes remain
S7. Optional embed/provider work runs after commit and cannot alter the acceptance result.

## Preferred design

`MessagingService` owns the admitted SQLite write transaction. Admission permits at most 128
pending operations, 32 active transaction contenders, and five seconds of lock wait. Before taking it, adapters
bound identifiers, content, attachment count, mention count, and canonical JSON size. In the
transaction it revalidates the durable credential and channel permission through
`AuthorizationService::authorize_actor_in`, then checks timeout, archive state, slow mode,
AutoMod, reply ownership, and attachments from that same connection. It checks
`(principal_id, operation_generation, client_message_id)` using a canonical SHA-256 payload
fingerprint. An identical committed retry returns the stored result after current access is
rechecked; conflicting reuse returns `IDEMPOTENCY_CONFLICT`.

The transaction allocates the conversation sequence, writes the message/mutation, mention
rows and attachment claims, advances entity versions, inserts the durable event descriptor,
and stores the receipt. Only a successful commit produces an acknowledgement or dispatcher
wake-up. Stable typed errors distinguish invalid input, unavailable resources, conflicts,
rate/slow-mode rejection, dependency failure, resynchronization, and internal failure.

The event log stores descriptors rather than historical protected payloads. Projection loads
current state under current authorization: an old create/edit projects the newest visible
entity, and a deleted message projects a tombstone. Replay cursors are signed opaque tokens
bound to database generation, principal, credential generation, and subscription scope.
Snapshot reads and captures high-water mark H in one read transaction, then replay begins
after H, preventing a snapshot/live gap. Expired retention, changed database generation,
changed credential generation, or incompatible scope returns `RESYNC_REQUIRED`.

`DeliveryDispatcher` consumes bounded wake-up hints and also polls the event table, so a
missed hint cannot lose delivery. It batches descriptors. Each connection retains the
existing bounded event queue plus an explicit byte budget; durable overflow marks the
connection desynchronized through a separate control path and closes if the control frame
cannot be written. One dispatcher task serves all events; it does not create a task for each
recipient.

## Alternative considered

A broker-first event stream would provide native consumer offsets and scalable fanout, but
it would create a second durable authority and a cross-store commit problem. A transaction
outbox could bridge the stores, yet restore and self-hosted operation would now depend on
broker reconciliation. SQLite message, receipt, and descriptor rows in one transaction give
the required acceptance and recovery semantics with fewer failure states. The schema keeps
the dispatcher boundary explicit so a broker can replace polling later without changing
command receipts or wire cursors.

A second alternative was to keep current synchronous `ChatEngine::send_message` and append
receipts/events from its detached persistence task. That cannot make acknowledgement follow
commit, cannot atomically claim attachments, and preserves divergent webhook/interaction
paths, so it is rejected.

## Delivery increments and evidence

1. Migration 020/021 plus upgrade fixtures: conversation/message backfill, stable ordering,
   unresolved preservation, receipts/events constraints, and no thread cascade on soft
   deletion.
2. Transaction service: send retry/conflict, forced message/attachment/receipt/event failure,
   concurrent sequence allocation, timeout/slow-mode/AutoMod/permission checks, and
   commit-before-result tests.
3. Mutation service: edit/delete/reaction/read monotonicity, current-state tombstones,
   entity-version ordering, and cross-conversation rejection.
4. Replay and dispatcher: snapshot plus concurrent write, duplicate/reordered descriptor
   convergence, current authorization filtering, generation/retention resync, missed-hint
   polling, bounded overflow, and cleanup.
5. Production adapters and wire: WebSocket v2 handshake/correlated results/resume, explicit
   legacy translation, IRC submission/projection, webhook and interaction convergence,
   generated schema/TypeScript validation, and full ingress journey tests.

The final verdict is `ready` only after these increments and their production ingress tests
pass. Until then the implementation verdict is `changes required`.
