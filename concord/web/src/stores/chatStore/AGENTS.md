# Chat store implementation

types.ts defines the facade contract; action factories receive typed set/get context. Event handling, projections, synchronization, defaults, runtime timers, and resets have separate modules.

Children must not import the facade at runtime. Keep the coordinated setter as the write path; pass get explicitly for asynchronous revalidation. Shared timer/maps live in runtime.ts and are cleared at lifecycle boundaries.

Run all browser harness tests, especially atomic facade updates, replay, retries, account changes, and notification races.
