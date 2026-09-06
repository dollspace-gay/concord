# Configuration and operations implementation evidence

Validated on 2026-09-05 against Git base
`cff0df246b91461f959d8dbe9154ba50bba2331c` with an uncommitted source tree.
The SHA-256 digest of the ordered files under `server/src`,
`server/migrations`, `web/src`, and `web/public` at the successful container
run was:

```text
4a36626fe8d24950dba8623548b58e740ca8429d13c4d0f5326be7ff93b7d19b
```

The SHA-256 digest of the binary Git diff for the `concord/` build context
was:

```text
2f4bcc6532641c5dcc6ea6605af27664e194d676990bbb1a52c656414472649b
```

## Configuration checks

- `cargo test --lib config::tests` passed all 7 tests.
- The loader reports TOML location without echoing secret values, validates
  environment selection and public URL origin requirements, enforces numeric
  bounds and TLS pairing, and rejects public plaintext IRC listeners.
- `concord-server init --config <path>` creates the persistent data, media,
  and secret directories and a stable admin identity configuration. Secret
  files are mode `0600`; private directories are mode `0700` on Unix.
- `concord-server validate-config --config <path>` exercises the same load and
  filesystem probes used by `serve` without starting listeners.

## Runtime and container qualification

`scripts/suites/container-smoke` passed with rootless Podman. It performed a
fresh locked release build using Rust 1.96.0 and Node 22.23.1, then verified:

1. the runtime process UID is `10001`;
2. `init` generated `/work/concord.toml` in an ephemeral bind mount;
3. the server started from that generated config;
4. `/health/live` returned success;
5. `/health/ready` returned success after its SQLite query and media-directory
   metadata probe;
6. `podman stop --time 10` produced exit code `0`, `shutdown signal received`,
   and `Concord server stopped`, proving the supervised drain completed without
   Podman's forced-kill fallback; and
7. cleanup succeeded when runtime-owned bind-mount files were removed inside
   the container user namespace.

The smoke runner retains `podman logs <container>` output at the path printed
on success (or `CONTAINER_SMOKE_LOG` when supplied) and rejects missing drain
markers or any nonzero exit code, including forced-stop codes 137 and 143.
The qualifying run built image ID
`c8f72e5656ddf95f6fe0eb2dcca582fde68a91159c15c84555ad3f666808d684`;
its retained log is `evidence/container-smoke.log` beside this document.
Its config and database paths are deliberately ephemeral (`/work/concord.toml`
and `/work/data/concord.db`) and are removed after qualification. The built
image is also removed, so the source digests above identify the tested
uncommitted snapshot rather than a retained image tag.

## Operational behavior

- Web and IRC sockets are bound before readiness is announced.
- Web, IRC, and cache-maintenance tasks run under one `JoinSet` supervisor.
- SIGINT/SIGTERM cancels the shared shutdown token; listeners stop accepting,
  readiness turns false, active work receives the cancellation signal, and a
  bounded drain aborts tasks that exceed the deadline.
- Browser CORS is restricted to the configured public origin and exact
  loopback development origins.
