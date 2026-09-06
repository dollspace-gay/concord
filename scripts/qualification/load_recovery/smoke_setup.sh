#!/usr/bin/env bash
# Sourced by run-load-recovery-qualification.sh; the caller owns cleanup.
    fixture_root="$(mktemp -d)"
    web_port="$(free_port)"
    irc_port="$(free_port)"
    database_url="sqlite:${fixture_root}/concord.db?mode=rwc"
    jwt_secret="load-smoke-secret-with-at-least-thirty-two-bytes"
    mkdir -p "${fixture_root}/media"
    printf '%064d\n' 0 > "${fixture_root}/external.key"
    printf '%s\n' "${jwt_secret}" > "${fixture_root}/jwt.key"
    chmod 600 "${fixture_root}/external.key" "${fixture_root}/jwt.key"
    cat > "${fixture_root}/concord.toml" <<EOF
[server]
web_address = "127.0.0.1:${web_port}"
irc_address = "127.0.0.1:${irc_port}"
shutdown_timeout_seconds = 5
[database]
url = "${database_url}"
[auth]
jwt_secret_file = "${fixture_root}/jwt.key"
external_credentials_key_file = "${fixture_root}/external.key"
session_expiry_hours = 1
public_url = "http://127.0.0.1:${web_port}"
[storage]
data_dir = "${fixture_root}"
media_dir = "${fixture_root}/media"
max_file_size_mb = 1
max_media_per_user_mb = 100
max_media_total_mb = 100
max_message_length = 4000
[admin]
admin_user_ids = ["browser-alice"]
[irc]
motd = []
[egress]
operator_allowed_origins = []
EOF
    (
      cd "${repository_root}/concord"
      cargo build --quiet --locked --features browser-fixtures \
        --bin browser_fixture_seed --bin concord-server --bin concord_operator
    )
    install -m 755 "${repository_root}/concord/target/debug/concord-server" "${fixture_root}/concord-server"
    install -m 755 "${repository_root}/concord/target/debug/browser_fixture_seed" "${fixture_root}/browser_fixture_seed"
    install -m 755 "${repository_root}/concord/target/debug/concord_operator" "${fixture_root}/concord-operator"
    CONCORD_FIXTURE_DATABASE_URL="${database_url}" CONCORD_FIXTURE_JWT_SECRET="${jwt_secret}" \
      CONCORD_FIXTURE_EXTERNAL_KEY_FILE="${fixture_root}/external.key" \
      "${fixture_root}/browser_fixture_seed" > "${fixture_root}/sessions.json"
    python3 - "${fixture_root}/concord.db" <<'PY'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as database:
    updated = database.execute(
        "UPDATE users SET is_system_admin=1 WHERE id='browser-alice'"
    ).rowcount
if updated != 1:
    raise SystemExit("smoke metrics principal was not persisted as a system admin")
PY
    smoke_dataset_sha256="$(sha256sum "${fixture_root}/concord.db" | cut -d ' ' -f 1)"
    export CONCORD_QUAL_DATASET_SHA256="${smoke_dataset_sha256}"
    token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["alice_irc"])' "${fixture_root}/sessions.json")"
    metrics_session="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["alice"])' "${fixture_root}/sessions.json")"
    python3 - "${fixture_root}/sessions.json" "${fixture_root}/web-sessions.json" <<'PY'
import json, sys
source, destination = sys.argv[1:]
sessions = json.load(open(source, encoding="utf-8"))
inventory = [
    {
        "cookie": sessions[principal],
        "subscriptions": [sessions["browser_conversation_id"]],
        "channels": ["#browser-fixture/general"],
        "server_id": "browser-server",
    }
    for principal in ("alice", "bob")
]
open(destination, "x", encoding="utf-8").write(json.dumps(inventory) + "\n")
PY
    cat > "${fixture_root}/query-plan.json" <<'EOF'
[{"session_index":0,"server_id":"browser-server","channel":"#general","query":"historical","expected_total":3,"history_min_count":3,"page_size":2,"interval_seconds":1.0}]
EOF
    cat > "${fixture_root}/permission-race-plan.json" <<'EOF'
{"session_index":1,"server_id":"browser-server","channel":"#general","denied_statuses":[403,404]}
EOF
    restart_request="${fixture_root}/restart.request"
    restart_ack="${fixture_root}/restart.ack"
    server_child_pid_file="${fixture_root}/server.child.pid"
    server_supervisor() (
      child_pid=""
      stop_child() {
        if [[ -n "${child_pid}" ]]; then
          kill -KILL "${child_pid}" 2>/dev/null || true
          wait "${child_pid}" 2>/dev/null || true
          child_pid=""
          rm -f "${server_child_pid_file}"
        fi
      }
      trap 'stop_child; exit 0' TERM INT
      start_child() {
        "${fixture_root}/concord-server" --config "${fixture_root}/concord.toml" \
          >> "${evidence_dir}/server.log" 2>&1 &
        child_pid=$!
        printf '%s\n' "${child_pid}" > "${server_child_pid_file}"
      }
      start_child
      while kill -0 "${child_pid}" 2>/dev/null; do
        if [[ -f "${restart_request}" ]]; then
          rm -f "${restart_request}" "${restart_ack}"
          stop_child
          sleep 0.2
          start_child
          ready=0
          for _ in $(seq 1 100); do
            if curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null; then ready=1; break; fi
            kill -0 "${child_pid}" 2>/dev/null || break
            sleep 0.1
          done
          [[ "${ready}" -eq 1 ]] || exit 1
          printf '{"action":"restart","ready":true}\n' > "${restart_ack}"
        fi
        sleep 0.02
      done
      wait "${child_pid}" 2>/dev/null || true
      exit 1
    )
    server_supervisor &
    server_pid=$!
    for _ in $(seq 1 100); do
      curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null && break
      kill -0 "${server_pid}" 2>/dev/null || { cat "${evidence_dir}/server.log" >&2; exit 1; }
      sleep 0.1
    done
    curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null
    telemetry_port="$(free_port)"
    telemetry_token="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
    printf '%s\n' "${telemetry_token}" > "${fixture_root}/telemetry.token"
    chmod 600 "${fixture_root}/telemetry.token"
    python3 "${repository_root}/scripts/serve-load-recovery-telemetry.py" \
      --listen "127.0.0.1:${telemetry_port}" \
      --server-pid-file "${server_child_pid_file}" \
      --database "${fixture_root}/concord.db" --web-port "${web_port}" --irc-port "${irc_port}" \
      --token-file "${fixture_root}/telemetry.token" \
      > "${evidence_dir}/telemetry.log" 2>&1 &
    telemetry_pid=$!
    for _ in $(seq 1 100); do
      if curl --silent --output /dev/null --max-time 2 \
        "http://127.0.0.1:${telemetry_port}/"; then
        break
      fi
      kill -0 "${telemetry_pid}" 2>/dev/null || {
        cat "${evidence_dir}/telemetry.log" >&2
        exit 1
      }
      sleep 0.05
    done
    curl --silent --output /dev/null --max-time 2 \
      "http://127.0.0.1:${telemetry_port}/"
    cat > "${fixture_root}/qualification-control" <<EOF
#!/usr/bin/env bash
set -euo pipefail
case "\${1:-}" in
  restart)
    rm -f "${restart_ack}"
    : > "${restart_request}"
    for _ in \$(seq 1 400); do
      if [[ -s "${restart_ack}" ]]; then cat "${restart_ack}"; exit 0; fi
      sleep 0.05
    done
    echo 'restart controller timed out' >&2
    exit 1
    ;;
  provider-status)
    python3 - "${fixture_root}/concord.db" "\${2:?webhook id required}" <<'PY'
import json, sqlite3, sys
database, webhook_id = sys.argv[1:]
with sqlite3.connect(database) as db:
    row = db.execute("SELECT d.state,d.attempt_count,d.safe_error_code,d.last_status,j.state,j.attempt_count,j.safe_error_code FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id WHERE d.webhook_id=? ORDER BY d.created_at DESC,d.id DESC LIMIT 1", (webhook_id,)).fetchone()
print(json.dumps({"found": row is not None, "delivery_state": row[0] if row else None, "delivery_attempts": row[1] if row else 0, "delivery_error": row[2] if row else None, "last_status": row[3] if row else None, "job_state": row[4] if row else None, "job_attempts": row[5] if row else 0, "job_error": row[6] if row else None}))
PY
    ;;
  provider-arm)
    python3 - "${fixture_root}/concord.db" "\${2:?webhook id required}" <<'PY'
import json, sqlite3, sys, uuid
database, webhook_id = sys.argv[1:]
with sqlite3.connect(database) as db:
    db.execute("INSERT OR IGNORE INTO webhook_events(id,webhook_id,event_type) VALUES(?,?,'message_create')", (str(uuid.uuid4()), webhook_id))
print(json.dumps({"armed": True}))
PY
    ;;
  provider-disarm)
    python3 - "${fixture_root}/concord.db" "\${2:?webhook id required}" <<'PY'
import json, sqlite3, sys
database, webhook_id = sys.argv[1:]
with sqlite3.connect(database) as db:
    db.execute("DELETE FROM webhook_events WHERE webhook_id=? AND event_type='message_create'", (webhook_id,))
    db.execute("UPDATE external_jobs SET state='cancelled',lease_owner=NULL,lease_token=NULL,lease_until=NULL,updated_at=datetime('now') WHERE id IN (SELECT external_job_id FROM webhook_deliveries WHERE webhook_id=?) AND state IN ('pending','leased')", (webhook_id,))
    db.execute("UPDATE webhook_deliveries SET state='cancelled',safe_error_code=COALESCE(safe_error_code,'qualification_provider_disarmed') WHERE webhook_id=? AND state IN ('pending','leased')", (webhook_id,))
print(json.dumps({"disarmed": True}))
PY
    ;;
  *) echo 'unsupported qualification control action' >&2; exit 64 ;;
esac
EOF
    chmod 700 "${fixture_root}/qualification-control"
    server_sha256="$(sha256sum "${fixture_root}/concord-server" | cut -d ' ' -f 1)"
    seed_sha256="$(sha256sum "${fixture_root}/browser_fixture_seed" | cut -d ' ' -f 1)"
    config_sha256="$(sha256sum "${fixture_root}/concord.toml" | cut -d ' ' -f 1)"
    query_mix_sha256="$(sha256sum "${fixture_root}/query-plan.json" | cut -d ' ' -f 1)"
    source_revision="$(git -C "${repository_root}" rev-parse HEAD)"
    server_toolchain="$(cd "${repository_root}/concord" && rustc --version)"
    export CONCORD_QUAL_SERVER_SHA256="${server_sha256}"
    export CONCORD_QUAL_SEED_SHA256="${seed_sha256}"
    export CONCORD_QUAL_CONFIG_SHA256="${config_sha256}"
    export CONCORD_QUAL_QUERY_MIX_SHA256="${query_mix_sha256}"
    export CONCORD_QUAL_SOURCE_REVISION="${source_revision}"
    export CONCORD_QUAL_SERVER_TOOLCHAIN="${server_toolchain}"
    export CONCORD_QUAL_SEEDED_MESSAGES=3
    export CONCORD_QUAL_IRC_HOST=127.0.0.1 CONCORD_QUAL_IRC_PORT="${irc_port}"
    export CONCORD_QUAL_IRC_TOKEN="${token}" CONCORD_QUAL_HTTP_ORIGIN="http://127.0.0.1:${web_port}"
    export CONCORD_QUAL_METRICS_SESSION="${metrics_session}"
    export CONCORD_QUAL_SERVER_TELEMETRY_URL="http://127.0.0.1:${telemetry_port}/"
    export CONCORD_QUAL_TELEMETRY_TOKEN="${telemetry_token}"
    export CONCORD_QUAL_SESSIONS="${CONCORD_QUAL_SESSIONS:-3}"
    export CONCORD_QUAL_MESSAGES="${CONCORD_QUAL_MESSAGES:-8}"
    export CONCORD_QUAL_DURATION_SECONDS="${CONCORD_QUAL_DURATION_SECONDS:-8}"
    export CONCORD_QUAL_SENDERS=1
    export CONCORD_QUAL_WEB_SESSIONS=2
    export CONCORD_QUAL_WEB_SESSIONS_FILE="${fixture_root}/web-sessions.json"
    export CONCORD_QUAL_QUERY_PLAN="${fixture_root}/query-plan.json"
    export CONCORD_QUAL_PERMISSION_RACE_PLAN="${fixture_root}/permission-race-plan.json"
    export CONCORD_QUAL_CONTROL_COMMAND="${fixture_root}/qualification-control"
    provider_webhook_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["provider_failure_webhook_id"])' "${fixture_root}/sessions.json")"
    export CONCORD_QUAL_PROVIDER_WEBHOOK_ID="${provider_webhook_id}"
    export CONCORD_QUAL_MAX_UPLOAD_BYTES=1048576
