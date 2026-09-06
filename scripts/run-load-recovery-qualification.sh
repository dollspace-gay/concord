#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${CONCORD_QUALIFICATION_MODE:-smoke}"
evidence_root="${CONCORD_QUALIFICATION_EVIDENCE:-${repository_root}/.design/concord-remediation/evidence/load-recovery}"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
evidence_dir="${evidence_root}/${run_id}"
mkdir -p "${evidence_dir}"

python3 "${repository_root}/scripts/analyze-load-recovery-evidence.py" --self-test
python3 "${repository_root}/scripts/serve-load-recovery-telemetry.py" --self-test

fixture_root=""
server_pid=""
server_child_pid_file=""
telemetry_pid=""
stop_server_processes() {
  if [[ -n "${server_pid}" ]]; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
    server_pid=""
  fi
  if [[ -n "${server_child_pid_file}" && -r "${server_child_pid_file}" ]]; then
    local child_pid child_executable
    child_pid="$(cat "${server_child_pid_file}")"
    child_executable="$(readlink "/proc/${child_pid}/exe" 2>/dev/null || true)"
    if [[ -n "${child_pid}" && "${child_executable}" == "${fixture_root}/concord-server" ]]; then
      kill -KILL "${child_pid}" 2>/dev/null || true
      for _ in $(seq 1 100); do
        [[ "$(readlink "/proc/${child_pid}/exe" 2>/dev/null || true)" == "${fixture_root}/concord-server" ]] || break
        sleep 0.05
      done
      if [[ "$(readlink "/proc/${child_pid}/exe" 2>/dev/null || true)" == "${fixture_root}/concord-server" ]]; then
        echo "smoke server child did not stop" >&2
        return 1
      fi
    fi
    rm -f "${server_child_pid_file}"
  fi
}
cleanup() {
  status=$?
  stop_server_processes || status=1
  if [[ -n "${telemetry_pid}" ]]; then
    kill "${telemetry_pid}" 2>/dev/null || true
    wait "${telemetry_pid}" 2>/dev/null || true
  fi
  if [[ -n "${fixture_root}" ]]; then
    rm -f "${fixture_root}/sessions.json" "${fixture_root}/jwt.key" "${fixture_root}/external.key"
    rm -rf "${fixture_root}"
  fi
  exit "${status}"
}
trap cleanup EXIT

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

case "${mode}" in
  smoke)
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
    ;;
  full)
    : "${CONCORD_QUAL_IRC_HOST:?full qualification requires the measured server host}"
    : "${CONCORD_QUAL_IRC_PORT:?full qualification requires the server IRC port}"
    : "${CONCORD_QUAL_HTTP_ORIGIN:?full qualification requires the server HTTP origin}"
    : "${CONCORD_QUAL_SERVER_METADATA:?full qualification requires target-host metadata JSON}"
    : "${CONCORD_QUAL_DATASET_SHA256:?full qualification requires the one-million-message dataset hash}"
    : "${CONCORD_QUAL_SERVER_TELEMETRY_URL:?full qualification requires measured-host telemetry}"
    : "${CONCORD_QUAL_TELEMETRY_TOKEN:?full qualification requires the telemetry bearer token}"
    : "${CONCORD_QUAL_METRICS_SESSION:?full qualification requires a current system-admin web session in the environment}"
    : "${CONCORD_QUAL_IRC_TOKENS_FILE:?full qualification requires a JSON array of dedicated IRC credentials}"
    : "${CONCORD_QUAL_CHANNELS_FILE:?full qualification requires a JSON array of 50 IRC channel aliases}"
    : "${CONCORD_QUAL_WEB_SESSIONS_FILE:?full qualification requires dedicated WebSocket session inventory}"
    : "${CONCORD_QUAL_QUERY_PLAN:?full qualification requires a stable expected-results history/search plan}"
    : "${CONCORD_QUAL_PERMISSION_RACE_PLAN:?full qualification requires a disposable authorization-revocation race plan}"
    : "${CONCORD_QUAL_CONTROL_COMMAND:?full qualification requires an executable target control/status adapter}"
    : "${CONCORD_QUAL_PROVIDER_WEBHOOK_ID:?full qualification requires a disposable controlled-failure webhook ID}"
    : "${CONCORD_QUAL_MAX_UPLOAD_BYTES:?full qualification requires the configured maximum upload bytes}"
    [[ -x "${CONCORD_QUAL_CONTROL_COMMAND}" ]] || {
      echo "qualification target control/status adapter is not executable" >&2
      exit 1
    }
    : "${CONCORD_QUAL_IRC_CA_FILE:?full qualification requires the IRC TLS trust root}"
    : "${CONCORD_QUAL_IRC_TLS_SERVER_NAME:?full qualification requires the verified IRC TLS server name}"
    : "${CONCORD_QUAL_HTTP_CA_FILE:?full qualification requires the HTTPS trust root}"
    [[ -r "${CONCORD_QUAL_IRC_CA_FILE}" ]] || {
      echo "IRC TLS trust root is not readable" >&2
      exit 1
    }
    [[ -r "${CONCORD_QUAL_HTTP_CA_FILE}" ]] || {
      echo "HTTPS trust root is not readable" >&2
      exit 1
    }
    export CONCORD_QUAL_SESSIONS="${CONCORD_QUAL_SESSIONS:-800}"
    export CONCORD_QUAL_WEB_SESSIONS="${CONCORD_QUAL_WEB_SESSIONS:-200}"
    export CONCORD_QUAL_MESSAGES="${CONCORD_QUAL_MESSAGES:-72000}"
    export CONCORD_QUAL_DURATION_SECONDS="${CONCORD_QUAL_DURATION_SECONDS:-3600}"
    export CONCORD_QUAL_SENDERS="${CONCORD_QUAL_SENDERS:-20}"
    : "${CONCORD_QUAL_SOURCE_IPS:?full qualification requires comma-separated generator source IPs}"
    python3 - <<'PY'
import math, os
sessions = (
    int(os.environ["CONCORD_QUAL_SESSIONS"])
    + int(os.environ["CONCORD_QUAL_WEB_SESSIONS"])
)
addresses = [value for value in os.environ["CONCORD_QUAL_SOURCE_IPS"].split(",") if value]
if len(addresses) < math.ceil(sessions / 5):
    raise SystemExit("source IP inventory cannot satisfy Concord's five-connections-per-IP bound")
PY
    python3 - <<'PY'
import json, os
tokens = json.load(open(os.environ["CONCORD_QUAL_IRC_TOKENS_FILE"], encoding="utf-8"))
required = int(os.environ["CONCORD_QUAL_SESSIONS"])
if not isinstance(tokens, list) or len(tokens) < required or any(not isinstance(value, str) or not value for value in tokens):
    raise SystemExit(f"credential inventory must contain at least {required} non-empty tokens")
PY
    python3 - <<'PY'
import json, os
channels = json.load(open(os.environ["CONCORD_QUAL_CHANNELS_FILE"], encoding="utf-8"))
if not isinstance(channels, list) or len(channels) != 50:
    raise SystemExit("channel inventory must contain exactly 50 channels")
if any(not isinstance(value, str) or not value.startswith("#") for value in channels):
    raise SystemExit("every channel inventory entry must be an IRC channel alias")
if len(set(channels)) != len(channels):
    raise SystemExit("channel inventory entries must be unique")
PY
    python3 - "${CONCORD_QUAL_SERVER_METADATA}" "${CONCORD_QUAL_IRC_HOST}" <<'PY'
import json, socket, sys
metadata_path, server_host = sys.argv[1:]
metadata = json.load(open(metadata_path, encoding="utf-8"))
required = {"hostname", "cpu_count", "memory_bytes", "filesystem", "storage", "kernel", "server_sha256", "source_revision", "rustc", "release_flags", "seed_sha256", "config_sha256", "query_mix_sha256", "dataset_sha256", "seeded_messages", "database_profile", "configured_max_upload_bytes"}
missing = sorted(required - metadata.keys())
if missing:
    raise SystemExit("target-host metadata is incomplete: " + ", ".join(missing))
memory_bytes = int(metadata["memory_bytes"])
if int(metadata["cpu_count"]) != 4 or not 7.5 * 1024**3 <= memory_bytes <= 9 * 1024**3:
    raise SystemExit("full qualification target must have 4 vCPUs and 7.5-9 GiB of memory")
local_names = {socket.gethostname(), socket.getfqdn()}
if metadata["hostname"] in local_names:
    raise SystemExit("load generator and measured server must run on separate hosts")
if metadata["dataset_sha256"] != __import__("os").environ["CONCORD_QUAL_DATASET_SHA256"]:
    raise SystemExit("dataset hash does not match measured-host metadata")
if int(metadata["seeded_messages"]) < 1_000_000:
    raise SystemExit("measured-host dataset contains fewer than one million seeded messages")
if metadata["database_profile"] != "FULL/WAL":
    raise SystemExit("full qualification requires the durable FULL/WAL database profile")
if int(metadata["configured_max_upload_bytes"]) != int(__import__("os").environ["CONCORD_QUAL_MAX_UPLOAD_BYTES"]):
    raise SystemExit("configured maximum upload bytes do not match measured-host metadata")
PY
    cp "${CONCORD_QUAL_SERVER_METADATA}" "${evidence_dir}/server-metadata.json"
    cp "${CONCORD_QUAL_CHANNELS_FILE}" "${evidence_dir}/channel-inventory.json"
    cp "${CONCORD_QUAL_QUERY_PLAN}" "${evidence_dir}/query-plan.json"
    python3 - "${CONCORD_QUAL_IRC_TOKENS_FILE}" "${evidence_dir}/credential-inventory.redacted.json" <<'PY'
import json, sys
source, destination = sys.argv[1:]
tokens = json.load(open(source, encoding="utf-8"))
open(destination, "x", encoding="utf-8").write(json.dumps({"credential_count": len(tokens)}) + "\n")
PY
    ;;
  *)
    echo "CONCORD_QUALIFICATION_MODE must be smoke or full" >&2
    exit 64
    ;;
esac

export CONCORD_QUALIFICATION_MODE="${mode}"
export CONCORD_QUAL_EVIDENCE_DIR="${evidence_dir}"
python3 "${repository_root}/scripts/load-recovery-generator.py"

if [[ "${mode}" == smoke ]]; then
  stop_server_processes
  python3 - "${fixture_root}/concord.db" "${evidence_dir}/summary.json" "${fixture_root}/sessions.json" <<'PY'
import json, sqlite3, sys, uuid
database, summary_path, sessions_path = sys.argv[1:]
marker = json.load(open(summary_path, encoding="utf-8"))["marker_prefix"]
webhook_id = json.load(open(sessions_path, encoding="utf-8"))["provider_failure_webhook_id"]
with sqlite3.connect(database) as db:
    source = db.execute(
        "SELECT j.operation_type,j.resource_id,j.resource_version,j.destination_grant,j.payload_json,d.webhook_id,d.event_sequence,d.event_type,d.event_version,d.payload_json FROM external_jobs j JOIN webhook_deliveries d ON d.external_job_id=j.id WHERE d.webhook_id=? ORDER BY d.created_at DESC,d.id DESC LIMIT 1",
        (webhook_id,),
    ).fetchone()
    if source is None:
        raise SystemExit("controlled provider delivery is unavailable for the restore probe")
    job_id = str(uuid.uuid4())
    delivery_id = "qualification-restore:" + str(uuid.uuid4())
    db.execute(
        "INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) VALUES(?,?,?,?,?,?,?)",
        (job_id, "qualification-restore:" + marker, source[0], delivery_id, *source[2:5]),
    )
    db.execute(
        "INSERT INTO webhook_deliveries(id,webhook_id,event_sequence,external_job_id,delivery_id,event_type,event_version,payload_json) VALUES(?,?,?,?,?,?,?,?)",
        (str(uuid.uuid4()), source[5], source[6], job_id, delivery_id, *source[7:]),
    )
    linked = db.execute(
        "SELECT COUNT(*) FROM external_jobs j JOIN webhook_deliveries d ON d.external_job_id=j.id AND d.delivery_id=j.resource_id WHERE j.id=? AND j.operation_type='webhook_delivery'",
        (job_id,),
    ).fetchone()[0]
    if linked != 1:
        raise SystemExit("restore probe is not a dispatchable registered webhook operation")
PY
  backup_dir="${fixture_root}/restore-backup"
  backup_create_log="${evidence_dir}/backup-create.log"
  backup_create_error="${fixture_root}/backup-create.error"
  backup_created=0
  for _ in $(seq 1 100); do
    if "${fixture_root}/concord-operator" --config "${fixture_root}/concord.toml" \
      backup-create --destination "${backup_dir}" > "${backup_create_log}" \
      2> "${backup_create_error}"; then
      backup_created=1
      break
    fi
    if ! grep -Fq "another Concord server or maintenance command is active" \
      "${backup_create_error}"; then
      cat "${backup_create_error}" >&2
      exit 1
    fi
    sleep 0.1
  done
  if [[ "${backup_created}" -ne 1 ]]; then
    cat "${backup_create_error}" >&2
    echo "database exclusion lock was not released within 10 seconds" >&2
    exit 1
  fi
  "${fixture_root}/concord-operator" --config "${fixture_root}/concord.toml" \
    backup-verify --backup "${backup_dir}" > "${evidence_dir}/backup-verify.log"
  restore_root="${fixture_root}/restored"
  mkdir -p "${restore_root}/media"
  cat > "${restore_root}/concord.toml" <<EOF
[server]
web_address = "127.0.0.1:${web_port}"
irc_address = "127.0.0.1:${irc_port}"
shutdown_timeout_seconds = 5
[database]
url = "sqlite:${restore_root}/concord.db?mode=rwc"
[auth]
jwt_secret_file = "${fixture_root}/jwt.key"
external_credentials_key_file = "${restore_root}/external.key"
session_expiry_hours = 1
public_url = "http://127.0.0.1:${web_port}"
[storage]
data_dir = "${restore_root}"
media_dir = "${restore_root}/media"
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
  "${fixture_root}/concord-operator" --config "${restore_root}/concord.toml" \
    backup-restore --backup "${backup_dir}" > "${evidence_dir}/backup-restore.log"
  "${fixture_root}/concord-server" --config "${restore_root}/concord.toml" \
    >> "${evidence_dir}/restored-server.log" 2>&1 &
  server_pid=$!
  printf '%s\n' "${server_pid}" > "${server_child_pid_file}"
  for _ in $(seq 1 100); do
    if curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null; then break; fi
    kill -0 "${server_pid}" 2>/dev/null || { cat "${evidence_dir}/restored-server.log" >&2; exit 1; }
    sleep 0.1
  done
  curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null
  old_session_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
    --cookie "concord_session=${metrics_session}" "http://127.0.0.1:${web_port}/api/me")"
  [[ "${old_session_status}" == 401 ]] || {
    echo "restored database did not invalidate the pre-restore session" >&2
    exit 1
  }
  python3 - "${restore_root}/concord.db" "${restore_root}/media" \
    "${evidence_dir}/summary.json" "${evidence_dir}/backup-restore.log" \
    "${evidence_dir}/restore-report.json" <<'PY'
import json, pathlib, re, sqlite3, sys
database, media_root, summary_path, restore_log_path, report_path = sys.argv[1:]
summary = json.load(open(summary_path, encoding="utf-8"))
restore_output = pathlib.Path(restore_log_path).read_text(encoding="utf-8")
with sqlite3.connect(database) as db:
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    contents = [row[0] for row in db.execute("SELECT content FROM messages WHERE content LIKE ?", (summary["marker_prefix"] + "%",))]
    main_messages = [value for value in contents if re.fullmatch(re.escape(summary["marker_prefix"]) + r"\d+", value)]
    provider_messages = [value for value in contents if value == summary["marker_prefix"] + "provider-failure"]
    media_messages = [value for value in contents if value == summary["marker_prefix"] + "media-stress"]
    restart_messages = db.execute("SELECT COUNT(*) FROM messages WHERE content LIKE 'restartprobe%'").fetchone()[0]
    active_external = db.execute("SELECT COUNT(*) FROM external_jobs WHERE state IN ('pending','leased')").fetchone()[0]
    restore_jobs = db.execute("SELECT j.state,j.safe_error_code,d.state,d.safe_error_code FROM external_jobs j JOIN webhook_deliveries d ON d.external_job_id=j.id AND d.delivery_id=j.resource_id WHERE j.deduplication_key=? AND j.operation_type='webhook_delivery'", ("qualification-restore:" + summary["marker_prefix"],)).fetchall()
    duplicate_publications = db.execute("SELECT COUNT(*) FROM (SELECT deduplication_key FROM external_jobs GROUP BY deduplication_key HAVING COUNT(*)>1)").fetchone()[0]
    media_keys = [row[0] for row in db.execute("SELECT storage_key FROM attachments WHERE media_state IN ('ready','attached')")]
missing_media = [key for key in media_keys if not (pathlib.Path(media_root) / key).is_file()]
expected_main = summary["exact_fanout"]["sent"]
accepted_verified = len(main_messages) == expected_main and len(provider_messages) == 1 and len(media_messages) == 1 and restart_messages == 1 and restore_jobs == [("failed", "restore_reconciliation_required", "failed", "restore_reconciliation_required")]
report = {
    "action": "restore",
    "ready": True,
    "declared_restore_point": restore_output.strip().split()[0] if restore_output.strip() else "",
    "integrity": integrity,
    "foreign_key_violations": len(foreign_keys),
    "missing_media": len(missing_media),
    "external_jobs_paused": "external_jobs_paused=true" in restore_output and active_external == 0,
    "restored_pending_jobs_reconciled": len(restore_jobs),
    "duplicate_publications": duplicate_publications,
    "accepted_messages_verified": accepted_verified,
    "restored_main_messages": len(main_messages),
    "restored_provider_messages": len(provider_messages),
    "restored_media_stress_messages": len(media_messages),
    "restored_restart_messages": restart_messages,
    "old_session_invalidated": True,
}
if integrity != "ok" or foreign_keys or missing_media or not report["external_jobs_paused"] or duplicate_publications or not accepted_verified:
    raise SystemExit("restored instance verification failed: " + json.dumps(report, sort_keys=True))
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
else
  mapfile -t restore_arguments < <(python3 - "${evidence_dir}/summary.json" <<'PY'
import json, sys
summary = json.load(open(sys.argv[1], encoding="utf-8"))
print(summary["marker_prefix"])
print(summary["exact_fanout"]["sent"])
PY
)
  "${CONCORD_QUAL_CONTROL_COMMAND}" restore \
    "${restore_arguments[0]}" "${restore_arguments[1]}" \
    > "${evidence_dir}/restore-report.json"
fi

python3 - "${evidence_dir}/summary.json" "${evidence_dir}/restore-report.json" "${mode}" <<'PY'
import json, os, pathlib, sys, tempfile
summary_path, report_path, mode = sys.argv[1:]
summary = json.load(open(summary_path, encoding="utf-8"))
report = json.load(open(report_path, encoding="utf-8"))
required = {
    "ready": True,
    "external_jobs_paused": True,
    "duplicate_publications": 0,
    "accepted_messages_verified": True,
    "restored_pending_jobs_reconciled": 1,
    "restored_provider_messages": 1,
    "restored_media_stress_messages": 1,
    "restored_restart_messages": 1,
    "old_session_invalidated": True,
}
for key, expected in required.items():
    if report.get(key) != expected:
        raise SystemExit(f"restore report failed {key}: expected={expected!r} actual={report.get(key)!r}")
if report.get("integrity") != "ok" or report.get("foreign_key_violations") != 0 or report.get("missing_media") != 0:
    raise SystemExit("restore report did not prove coherent local references")
summary["restart_restore"].update({"passed": True, "restore": True, "restore_recovered": True, "duplicate_publications": 0, "accepted_messages_verified": True, "restore_result": report})
summary["no_duplicate_accepted_messages"] = True
summary["unverified_acceptance_areas"] = (["dedicated 4-vCPU 8-GiB separate-host one-hour scale"] if mode == "smoke" else [])
summary["acceptance_status"] = "bounded-local-smoke" if mode == "smoke" else "full-acceptance-candidate"
summary["full_acceptance_claimed"] = mode == "full"
destination = pathlib.Path(summary_path)
with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=destination.parent, delete=False) as output:
    json.dump(summary, output, indent=2, sort_keys=True)
    output.write("\n")
    temporary = output.name
os.replace(temporary, destination)
PY

python3 "${repository_root}/scripts/analyze-load-recovery-evidence.py" --mode "${mode}" --evidence "${evidence_dir}"

if [[ "${mode}" == smoke ]]; then
  printf 'load-recovery smoke: PASS; bounded local evidence=%s; full one-hour external-host qualification remains unverified\n' "${evidence_dir}"
else
  printf 'load-recovery full qualification: PASS; evidence=%s\n' "${evidence_dir}"
fi
