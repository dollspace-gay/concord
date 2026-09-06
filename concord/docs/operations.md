# Concord installation and operations

This is the canonical installation and configuration reference for the Concord service.

## Build from source

The repository pins Rust 1.96.0 in `rust-toolchain.toml`. Build from the `concord/` workspace root so Cargo uses the committed lockfile. The source layout below is the supported example: the service runs with `/opt/concord/share/concord` as its working directory because the current server resolves browser assets from `./static`. Cargo names the operator artifact `concord_operator`; the install step deliberately exposes it as `concord-operator`.

```bash
cd concord/web
npm ci
npm run contracts:check
npm run build

cd ..
cargo build --workspace --release --locked --bin concord-server --bin concord_operator

sudo install -d -o root -g root -m 0755 /opt/concord/bin /opt/concord/share/concord/static
sudo install -d -o concord -g concord -m 0700 /etc/concord /var/lib/concord
sudo install -m 0755 target/release/concord-server /opt/concord/bin/concord-server
sudo install -m 0755 target/release/concord_operator /opt/concord/bin/concord-operator
sudo cp -R web/dist/. /opt/concord/share/concord/static/
sudo chown -R root:root /opt/concord
```

The generated browser artifact includes `protocol-version.json`. It must travel with the server built from the same checkout. Add `/opt/concord/bin` to the administrator's `PATH`, or use the absolute executable paths shown below.

## First-run setup

Concord never invents a temporary signing secret during normal startup. Initialize an empty deployment explicitly:

```bash
sudo -u concord /opt/concord/bin/concord-server init --config /etc/concord/concord.toml
sudo mv /etc/concord/data/* /var/lib/concord/
sudo rmdir /etc/concord/data
sudo sed -i \
  -e 's|sqlite:data/concord.db|sqlite:/var/lib/concord/concord.db|' \
  -e 's|data/secrets/jwt.key|/var/lib/concord/secrets/jwt.key|' \
  -e 's|data/secrets/external-credentials.key|/var/lib/concord/secrets/external-credentials.key|' \
  -e 's|data/media|/var/lib/concord/media|' \
  -e 's|data_dir = "data"|data_dir = "/var/lib/concord"|' \
  /etc/concord/concord.toml
```

Initialization creates a new configuration and random JWT signing secret without replacing existing files. On Unix, secret/config files are mode `0600` and data/secret directories are mode `0700`. Move the generated files only while preserving those permissions and update paths in the configuration if needed.

Start the service after reviewing the generated public URL, listener addresses, and data paths:

```bash
cd /opt/concord/share/concord
sudo -u concord /opt/concord/bin/concord-server --config /etc/concord/concord.toml validate-config
sudo -u concord /opt/concord/bin/concord-server --config /etc/concord/concord.toml serve
```

Normal startup rejects a missing configuration, weak/known sample secret, secret files accessible by group or others, malformed or insecure public origins, invalid numeric bounds, partial TLS configuration, and unusable data/media/database paths before binding listeners.

## Stable administrator bootstrap

Set `admin.admin_user_ids` to verified stable IDs. For the current AT Protocol login, these are DIDs such as `did:plc:...`. A configured identity is granted administration during its next verified login, including the first login after startup. Handles and usernames are presentation values and never confer administration.

After an administrator has logged in and verified access, remove unneeded bootstrap IDs and restart with the reviewed configuration. Use the stopped-service operator commands below for later transfer or recovery; do not substitute mutable usernames or direct unaudited SQL.

## Configuration

`concord.example.toml` documents every current file setting. Environment overrides are `WEB_ADDRESS`, `IRC_ADDRESS`, `IRC_TLS_CERT`, `IRC_TLS_KEY`, `DATABASE_URL`, `JWT_SECRET`, `JWT_SECRET_FILE`, `SESSION_EXPIRY_HOURS`, `PUBLIC_URL`, `DATA_DIR`, `MEDIA_DIR`, `MAX_FILE_SIZE_MB`, `MAX_MESSAGE_LENGTH`, `SHUTDOWN_TIMEOUT_SECONDS`, and `ADMIN_USER_IDS`.

Prefer `JWT_SECRET_FILE` over an inline secret. It points to a file containing only the secret and must not be readable by group or others. `JWT_SECRET` and `JWT_SECRET_FILE` are mutually exclusive after environment overrides. External-provider credentials use the separate `auth.external_credentials_key_file`; a missing, malformed, or wrong established key fails closed.

Use `/opt/concord/bin/concord-operator` for external-secret, historical-media, publication, and backup lifecycles. These examples abbreviate that installed absolute path as `concord-operator`:

```text
concord-operator key-init --key-file /secure/new-external.key
concord-operator --config concord.toml secrets-migrate
concord-operator --config concord.toml secrets-rotate --new-key-file /secure/new-external.key
concord-operator --config concord.toml media-inventory
concord-operator --config concord.toml media-retry ATTACHMENT_ID
concord-operator --config concord.toml media-import
concord-operator --config concord.toml atproto-publication-inventory
concord-operator --config concord.toml atproto-publication-reconcile PUBLICATION_ID
concord-operator --config concord.toml migration-status
concord-operator --config concord.toml migration-apply
concord-operator --config concord.toml admin-inventory
concord-operator --config concord.toml admin-transfer --from-user-id OLD_STABLE_ID --to-user-id NEW_STABLE_ID --reason "TICKET"
concord-operator --config concord.toml admin-recover --user-id STABLE_ID --reason "TICKET"
concord-operator --config concord.toml credential-revoke-all --user-id STABLE_ID --reason "TICKET"
concord-operator --config concord.toml jobs-inspect --state failed --limit 100
concord-operator --config concord.toml job-retry JOB_ID --reason "TICKET"
concord-operator --config concord.toml backup-create --destination /secure/backups/BACKUP_ID
concord-operator --config concord.toml backup-verify --backup /secure/backups/BACKUP_ID
```

Rotation rewrites account, signing, and pending OAuth envelopes in one database transaction, preserves the old key beside the configured key as a recovery copy, then atomically activates the replacement. Keep that recovery copy offline until the restarted service and pending OAuth recovery have been verified.

Stop Concord before administrator, credential, migration, or job recovery. These commands acquire the same exclusion lock as the server and write an operator audit record for every state-changing administrator, credential, and job action. `admin-transfer` requires two verified human accounts and refuses to demote an ID still present in `admin.admin_user_ids`, because the next login would otherwise restore its privilege. `admin-recover` only promotes a verified human account.

`credential-revoke-all` atomically revokes active browser and IRC credentials, delegated OAuth access and refresh tokens and their grants, pending authorization codes, and pending consent requests for the stable user ID. A disabled verified human can still be revoked. Restart Concord, then verify that previous browser/IRC sessions, OAuth bearer access, and refresh fail before closing the incident.

`jobs-inspect` deliberately omits job payloads and destination grants. `job-retry` currently accepts an eligible failed outgoing-webhook delivery and revalidates its source policy in the dispatcher. Reconcile AT Protocol publications with `atproto-publication-inventory` and `atproto-publication-reconcile`; the generic job command refuses them.

## Dedicated load and recovery qualification

The release-scale qualifier uses two hosts. The measured target must report exactly four vCPUs and 7.5–9 GiB of memory; the load generator must be a separate host. The repository prepares the complete disposable target: a locked release build, 800 IRC credentials, 200 browser sessions, 50 channels, one million messages, a controlled failing webhook, TLS, process supervision, authenticated resource telemetry, and stopped-service backup/restore control. The 20 logical IRC senders are drawn from the 800 IRC sessions, so the steady workload has exactly 1,000 long-lived sessions rather than 1,020 hidden connections.

Use a fresh target root. Preparation refuses an existing path and leaves all credentials, the database, backups, and restored data under that private root. `CONCORD_QUAL_TARGET_HOST` must resolve from both hosts to the target address used by the generator.

```bash
export CONCORD_QUAL_TARGET_ROOT=/var/lib/concord-load-qualification
export CONCORD_QUAL_TARGET_HOST=qual-target.example.internal
./scripts/prepare-load-recovery-target.sh
```

The target script requires the declared host size plus Cargo, the pinned Rust toolchain, OpenSSL, Python, curl, `findmnt`, `flock`, and `nohup`. It builds from `concord/`, so `rust-toolchain.toml` controls the compiler even when the script is launched at repository root. It generates a two-day fixture CA and uses the same certificate for Concord IRC TLS and the included target-local web TLS terminator. The terminator gives backend WebSocket connections distinct loopback peers so Concord's ordinary per-address admission remains active. It does not add a trusted forwarding header or bypass the application policy.

Securely copy the printed `export` directory to a mode-0700 directory on the generator. It contains live IRC credentials, browser cookies, the telemetry bearer token, and the fixture CA. Do not attach it to CI artifacts, qualification evidence, tickets, or logs. The target's credential-bearing backup stays under its private root; retained evidence contains only hashes, redacted inventory counts, quantitative measurements, and the sanitized restore report.

The generator host needs at least 200 already assigned, locally bindable source addresses. Address assignment is host/network provisioning and must match that environment; the preparation script validates every address by binding it before the hour starts. Then create the private environment file and run the qualifier:

```bash
export CONCORD_QUAL_BUNDLE_DIR=/secure/concord-load-export
export CONCORD_QUAL_SSH_TARGET=concord-qual@qual-target.example.internal
export CONCORD_QUAL_SOURCE_IPS="$(cat /secure/concord-source-addresses.csv)"
export CONCORD_QUAL_ENV_FILE=/secure/concord-load.env
./scripts/prepare-load-recovery-generator.sh
source /secure/concord-load.env
./scripts/run-load-recovery-qualification.sh
```

The SSH account must be able to execute `<target-root>/bin/target-control` and access only the prepared root. The generated wrapper permits the fixed restart, provider status/arm/disarm, restore, and shutdown actions. The runner's mandatory analyzer and telemetry self-tests execute before either smoke or full setup. Full preflight checks the exact inventories, hashes, source-address capacity, TLS roots, 4-vCPU memory envelope, FULL/WAL profile, one-million-message dataset, and 100 MiB upload limit before starting the hour.

The measured workload holds 800 IRC plus 200 WebSocket sessions, sends at least 72,000 accepted messages for at least 3,600 seconds through 20 of those IRC sessions, and verifies exact recipients at mean fanout 100. It runs concurrent stable history/search pagination, bounded 200-session reconnect and 10% slow-reader recovery, malformed/fragmented/rate abuse, a permission race, terminal provider retry classification, four overlapping 100 MiB uploads with a correlated chat acknowledgment while all four are admitted, process restart and exact retry, and a private stopped-service restore. Once the controlled provider failure has been recorded, disarming its disposable webhook cancels only that webhook's retained qualification delivery and job before resource-reclamation sampling. After the server stops, the restore controller clones that valid registered webhook operation into one separately identified pending delivery and external job. The restored database must contain both linked rows as failed with `restore_reconciliation_required`, so paused-job verification covers publishable work. Authenticated telemetry and `/metrics` samples cover warmup, steady, stress, and post-disconnect phases; stress sampling includes the held four-upload window. The analyzer rejects a missing or unauthorized sample, missing occupancy, stress peaks, resource reclamation, integrity, media, pending-job reconciliation, or exact-message evidence.

After retaining the generated evidence directory, stop the disposable services without deleting the private recovery material:

```bash
source /secure/concord-load.env
"${CONCORD_QUAL_CONTROL_COMMAND}" shutdown
```

`load-recovery smoke: PASS` proves the same generator, telemetry, analyzer, restart, and restore contracts against a bounded local fixture. Only `load-recovery full qualification: PASS` from the declared separate-host hour is release-scale evidence.

## Backup and restore

Stop Concord before running backup or restore. Both commands acquire the same database exclusion lock as the server and refuse to run while another server or maintenance command owns it. `backup-create` checkpoints the WAL and uses SQLite's coordinated `VACUUM INTO` snapshot while the service is stopped. It copies local media, the source configuration, the external-credential key, and the configured JWT secret file into a private destination. The manifest records sizes and SHA-256 checksums. Creation finishes only after the command reopens the snapshot read-only and verifies checksums, SQLite integrity and foreign keys, the exact supported schema and database generation, every referenced local media checksum, and active encrypted credentials.

Store the resulting directory as one unit on protected storage. It contains credentials and may contain an inline JWT secret in `config/source.toml`; restrict it as you would the live data directory. A successful local command proves the bytes in that backup at that time. Copying a live database file, copying only the database, or retaining a backup without periodically executing a restore does not establish recovery.

Restore uses a freshly initialized destination configuration so deployment-specific addresses and a fresh JWT signing key are deliberate. Stop the prior writer and keep the restored instance offline. Remove the newly initialized empty database, media contents, and external-credential key, while retaining the fresh destination configuration and JWT key. Then run:

```bash
concord-operator --config /restore/concord.toml \
  backup-restore --backup /secure/backups/BACKUP_ID
```

The destination database, media directory, and external-credential key path must be empty. The command verifies the complete backup, streams files into private sibling staging paths, and reconciles the staged database before activation. A durable `*.concord-restore-pending` marker is written first. Concord refuses to start while that marker exists. If the operator is interrupted during copying, rerunning the same command discards incomplete staged copies and starts them again. If activation is interrupted, rerunning it completes the remaining atomic renames before removing the marker.

Reconciliation assigns new database and operation generations with integer expiry epochs, revokes restored browser sessions, and moves pending or leased webhook deliveries and external jobs to `failed` with `restore_reconciliation_required`. The command prints `activation_required=true external_jobs_paused=true` only after all staged paths are durable and the fail-closed marker has been removed. Inspect and reconcile those jobs against their external destinations before using the existing retry controls. Validate the restored configuration, start only this restored writer, check `/health/ready`, sign in again, and sample messages and media before directing traffic to it.

The repository's locally runnable drill is:

```bash
./scripts/run-required-suite.sh packaging-restore
```

That suite builds the locked release binaries and executes a populated database/media backup and empty-destination restore. It is local qualification; the deployment operator must still run the same drill against representative storage, permissions, supervision, and backup transport used by the target host.

`egress.operator_allowed_origins` is an exact origin allowlist for operator-controlled private receivers. It does not apply to user-driven previews or provider discovery. Include scheme and port and omit paths, queries, and fragments.

`PUBLIC_URL` must be a single HTTP(S) origin with no credentials, path, query, or fragment. Plain HTTP is accepted only for the exact loopback hosts `localhost`, `127.0.0.1`, and `::1`.

IRC TLS is enabled only when both certificate and private-key paths are present and readable. The private key follows the same restrictive permission check as other secrets. Plaintext IRC is accepted only on a loopback listener; binding IRC to a container or host interface requires TLS.

For a host service account, install the certificate read-only and the key private to `concord`, set both `server.irc_tls_cert` and `server.irc_tls_key`, and keep `WorkingDirectory=/opt/concord/share/concord` in the supervisor. The service needs write access to `/var/lib/concord` and read access to `/etc/concord/concord.toml`, its secret files, and the TLS pair. It does not need write access to `/opt/concord`.

For an update, stop the service, build both locked binaries and the browser from one checkout, stage them beside the installed paths, rename the two binaries and the complete `static` directory into place, then start and check `/health/ready`. Keep the prior binaries and static directory together for rollback; database rollback is limited by the schema floor printed by the release notes and must never be guessed. The repository exercises this layout, the Cargo underscore-to-hyphen rename, non-root startup, browser assets, an actual TLS handshake, operator invocation, and a stopped-service artifact replacement with:

```bash
./scripts/run-source-install-smoke.sh
```

## Container

The image builds the browser with npm's lockfile and the Rust workspace with Cargo's lockfile, then runs as UID/GID 10001. `/var/lib/concord` is the writable database/media volume; mount configuration and secret files read-only under `/etc/concord`. The image contains no deployment secret.

Prepare host files before `podman compose up` or `docker compose up`. The sample Compose file expects `./concord.toml` and `./secrets` mounts. Keep the service behind HTTPS for non-loopback browser deployments and configure both IRC TLS files when exposing IRC directly.

The image also installs the matching stopped-service operator at `/app/concord-operator`. Compose reuses the service's configuration, database/media volume, and read-only key mounts for an operator run. Stop the writer first, run the command without starting its dependencies, then restart only after reviewing the result:

```bash
docker compose stop concord
docker compose run --rm --no-deps --entrypoint /app/concord-operator concord \
  --config /etc/concord/concord.toml migration-status
docker compose start concord
```

Replace `migration-status` with one of the documented operator commands as needed. Backup destinations require an additional protected mount that is outside the live media directory. Do not run a second `concord-server` container against the same database volume.

## Health and shutdown

`/health/live` reports that the process can serve HTTP. `/health/ready` becomes successful only while admission is open, both required listeners are bound, the database is at the exact supported schema, a short admitted write transaction can begin and roll back, and the media directory passes a create/write/fsync/delete/directory-fsync probe. The HTTP readiness request waits at most two seconds. Its owned dependency work retains the single-flight admission guard and continues through cleanup after a request timeout or disconnect; overlapping readiness requests receive 503 until that work finishes. A required listener or local storage failure clears readiness. Provider availability is deliberately separate so an unavailable external service does not take healthy local chat out of service.

`/metrics` returns Prometheus text with bounded collection only to a current browser session whose user is a persisted system administrator. Missing or invalid credentials receive 401 and an ordinary member receives 403. Send the session as the `concord_session` cookie over HTTPS; do not grant access based on client IP or forwarded headers. Keep the scrape credential in the collector's secret environment or secret store rather than its command line, configuration committed to source control, logs, or evidence output. The load/recovery qualifier reads `CONCORD_QUAL_METRICS_SESSION` from its environment, sends it only as the request cookie, and records the returned aggregate metrics without recording the credential.

A database collection failure returns HTTP 503 and publishes an unsuccessful collection result instead of presenting partial database gauges as healthy. Metrics cover the supported schema, durable receipt and dispatcher progress, pending and oldest outbox work, external-job states and attempts, active web sessions, and upload permits. Labels are limited to fixed health components, attachment states, and external-job states; user, server, channel, message, URL, and provider values are never metric labels.

Only one database metrics collection runs at a time. Successful scrapes share a one-second cache; while a refresh is active, another authorized scrape may receive the most recent snapshot if it is no more than 30 seconds old, otherwise it receives 503. A timed-out or disconnected scrape does not abandon its admitted collection. Runtime counters and cumulative latency histograms use fixed operation labels for command admission and acknowledgment, message commits, outbound queue attempts and overflow, resynchronization, replay/snapshot work, database-write admission, uploads, job dispatch, readiness probes, metrics collection, and migrations, with success/failure outcomes. These counters measure process-lifetime events and reset on process restart.

Every `backup-restore` invocation emits one newline-delimited JSON operation record after it finishes. Success records are written to standard output and failure records to standard error. The fixed fields are `kind="concord_operator_operation"`, `operation="restore"`, `outcome` (`success` or `failure`), and monotonic `duration_seconds`. The record contains no backup identifier, path, credential, content, or error text; the ordinary human-readable result or error remains a separate line. Capture both streams in the stopped-service operator log to measure restore results without expecting the later server process to retain another process's counters.

SIGINT and SIGTERM stop admission and wait for supervised listeners, connections, and maintenance work up to `server.shutdown_timeout_seconds`; exceeding the deadline returns a failure. Target-host restore, supervision, metrics scraping, retention, and alert delivery still require qualification in the deployment environment.
