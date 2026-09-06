# Client state ownership

Connection, pending commands, canonical entities, composer drafts, authentication, and UI state have explicit owners. chatStore.ts is the compatibility and coordination facade.

Use stable empty references in selectors. Preserve atomic domain updates, generation checks, no automatic lifecycle replay, and account-scoped private state.

Run frontend lint/build and all browser harness tests for store changes.
