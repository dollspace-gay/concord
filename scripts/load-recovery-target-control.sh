#!/usr/bin/env bash
set -euo pipefail

target_root="${CONCORD_QUAL_TARGET_ROOT:?set CONCORD_QUAL_TARGET_ROOT to the prepared target root}"
[[ "${target_root}" == /* && -r "${target_root}/runtime.env" ]] || {
  echo "qualification target root is not prepared" >&2
  exit 64
}
# This file is created mode 0600 by prepare-load-recovery-target.sh and contains
# only shell-quoted paths and the fixed qualification network configuration.
# shellcheck source=/dev/null
source "${target_root}/runtime.env"

mkdir -p "${TARGET_RUN_DIR}" "${TARGET_LOG_DIR}" "${TARGET_PRIVATE_DIR}"

stop_child() {
  if [[ ! -r "${TARGET_SERVER_PID_FILE}" ]]; then
    return 0
  fi
  local child_pid
  child_pid="$(cat "${TARGET_SERVER_PID_FILE}")"
  if [[ "${child_pid}" =~ ^[0-9]+$ ]] && kill -0 "${child_pid}" 2>/dev/null; then
    kill "${child_pid}" 2>/dev/null || true
    for _ in $(seq 1 300); do
      kill -0 "${child_pid}" 2>/dev/null || break
      sleep 0.05
    done
    if kill -0 "${child_pid}" 2>/dev/null; then
      kill -KILL "${child_pid}" 2>/dev/null || true
    fi
    wait "${child_pid}" 2>/dev/null || true
  fi
  rm -f "${TARGET_SERVER_PID_FILE}"
}

start_child() {
  local active_config child_pid
  active_config="$(cat "${TARGET_ACTIVE_CONFIG_FILE}")"
  "${TARGET_SERVER_BIN}" --config "${active_config}" >> "${TARGET_LOG_DIR}/server.log" 2>&1 &
  child_pid=$!
  printf '%s\n' "${child_pid}" > "${TARGET_SERVER_PID_FILE}"
}

wait_ready() {
  for _ in $(seq 1 300); do
    if curl --fail --silent --max-time 2 "${TARGET_BACKEND_ORIGIN}/health/ready" >/dev/null; then
      return 0
    fi
    if [[ ! -r "${TARGET_SERVER_PID_FILE}" ]] || ! kill -0 "$(cat "${TARGET_SERVER_PID_FILE}")" 2>/dev/null; then
      return 1
    fi
    sleep 0.1
  done
  return 1
}

supervisor() {
  local child_pid request
  trap 'stop_child; rm -f "${TARGET_SUPERVISOR_PID_FILE}"; exit 0' TERM INT
  printf '%s\n' "$$" > "${TARGET_SUPERVISOR_PID_FILE}"
  start_child
  while true; do
    if [[ -s "${TARGET_REQUEST_FILE}" ]]; then
      request="$(cat "${TARGET_REQUEST_FILE}")"
      rm -f "${TARGET_REQUEST_FILE}" "${TARGET_ACK_FILE}"
      case "${request}" in
        restart)
          stop_child
          start_child
          if wait_ready; then
            printf '{"action":"restart","ready":true}\n' > "${TARGET_ACK_FILE}"
          else
            printf '{"action":"restart","ready":false}\n' > "${TARGET_ACK_FILE}"
          fi
          ;;
        stop)
          stop_child
          printf '{"action":"stop","stopped":true}\n' > "${TARGET_ACK_FILE}"
          ;;
        start)
          stop_child
          start_child
          if wait_ready; then
            printf '{"action":"start","ready":true}\n' > "${TARGET_ACK_FILE}"
          else
            printf '{"action":"start","ready":false}\n' > "${TARGET_ACK_FILE}"
          fi
          ;;
        *)
          printf '{"error":"unsupported supervisor request"}\n' > "${TARGET_ACK_FILE}"
          ;;
      esac
    fi
    if [[ -r "${TARGET_SERVER_PID_FILE}" ]] && ! kill -0 "$(cat "${TARGET_SERVER_PID_FILE}")" 2>/dev/null; then
      exit 1
    fi
    sleep 0.05
  done
}

request_supervisor() {
  local request="$1" expected="$2"
  if [[ ! -r "${TARGET_SUPERVISOR_PID_FILE}" ]] \
    || ! kill -0 "$(cat "${TARGET_SUPERVISOR_PID_FILE}")" 2>/dev/null; then
    echo "qualification supervisor is unavailable" >&2
    return 1
  fi
  rm -f "${TARGET_ACK_FILE}"
  printf '%s\n' "${request}" > "${TARGET_REQUEST_FILE}"
  for _ in $(seq 1 800); do
    if [[ -s "${TARGET_ACK_FILE}" ]]; then
      python3 - "${TARGET_ACK_FILE}" "${expected}" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
if value.get(sys.argv[2]) is not True:
    raise SystemExit("supervisor request failed")
print(json.dumps(value, sort_keys=True))
PY
      return 0
    fi
    sleep 0.05
  done
  echo "qualification supervisor request timed out" >&2
  return 1
}

active_database() {
  cat "${TARGET_ACTIVE_DATABASE_FILE}"
}

render_config() {
  local destination="$1" database="$2" media="$3" jwt_file="$4" external_key="$5"
  cat > "${destination}" <<EOF
[server]
web_address = "${TARGET_BACKEND_BIND}"
irc_address = "${TARGET_IRC_BIND}"
irc_tls_cert = "${TARGET_TLS_CERT}"
irc_tls_key = "${TARGET_TLS_KEY}"
shutdown_timeout_seconds = 10
[database]
url = "sqlite:${database}?mode=rwc"
[auth]
jwt_secret_file = "${jwt_file}"
external_credentials_key_file = "${external_key}"
session_expiry_hours = 2
public_url = "${TARGET_PUBLIC_ORIGIN}"
[storage]
data_dir = "$(dirname "${database}")"
media_dir = "${media}"
max_file_size_mb = 100
max_media_per_user_mb = 200
max_media_total_mb = 1024
max_message_length = 4000
[admin]
admin_user_ids = ["load-web-000"]
[irc]
motd = []
[egress]
operator_allowed_origins = []
EOF
  chmod 600 "${destination}"
}

restore_target() {
  local marker="$1" expected_main="$2" original_config backup_dir restore_root restore_log
  [[ "${marker}" =~ ^load-[0-9a-f-]+-$ && "${expected_main}" =~ ^[0-9]+$ ]] || {
    echo "restore verification arguments are invalid" >&2
    return 64
  }
  original_config="$(cat "${TARGET_ACTIVE_CONFIG_FILE}")"
  request_supervisor stop stopped >/dev/null
  python3 - "$(active_database)" "${marker}" "${TARGET_SEED_RESULT}" <<'PY'
import json, sqlite3, sys, uuid
database, marker, seed_result = sys.argv[1:]
webhook_id = json.load(open(seed_result, encoding="utf-8"))["provider_failure_webhook_id"]
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
  backup_dir="${TARGET_PRIVATE_DIR}/backup-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  restore_root="${TARGET_PRIVATE_DIR}/restored-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  restore_log="${TARGET_LOG_DIR}/restore-$(date -u +%Y%m%dT%H%M%SZ)-$$.log"
  "${TARGET_OPERATOR_BIN}" --config "${original_config}" \
    backup-create --destination "${backup_dir}" > "${TARGET_LOG_DIR}/backup-create.log" 2>&1
  "${TARGET_OPERATOR_BIN}" --config "${original_config}" \
    backup-verify --backup "${backup_dir}" > "${TARGET_LOG_DIR}/backup-verify.log" 2>&1
  mkdir -p "${restore_root}/media"
  umask 077
  openssl rand -hex 48 > "${restore_root}/jwt.key"
  render_config \
    "${restore_root}/concord.toml" "${restore_root}/concord.db" \
    "${restore_root}/media" "${restore_root}/jwt.key" "${restore_root}/external.key"
  "${TARGET_OPERATOR_BIN}" --config "${restore_root}/concord.toml" \
    backup-restore --backup "${backup_dir}" > "${restore_log}" 2>&1
  printf '%s\n' "${restore_root}/concord.toml" > "${TARGET_ACTIVE_CONFIG_FILE}"
  printf '%s\n' "${restore_root}/concord.db" > "${TARGET_ACTIVE_DATABASE_FILE}"
  printf '%s\n' "${restore_root}/media" > "${TARGET_ACTIVE_MEDIA_FILE}"
  request_supervisor start ready >/dev/null

  python3 - "${restore_root}/concord.db" "${restore_root}/media" \
    "${restore_log}" "${marker}" "${expected_main}" \
    "${TARGET_BACKEND_ORIGIN}" "${TARGET_SEED_RESULT}" <<'PY'
import json, pathlib, re, sqlite3, sys, urllib.error, urllib.request
database, media_root, restore_log, marker, expected_main, origin, seed_result = sys.argv[1:]
expected_main = int(expected_main)
restore_output = pathlib.Path(restore_log).read_text(encoding="utf-8")
with sqlite3.connect(database) as db:
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    contents = [row[0] for row in db.execute("SELECT content FROM messages WHERE content LIKE ?", (marker + "%",))]
    main_messages = [value for value in contents if re.fullmatch(re.escape(marker) + r"\d+", value)]
    provider_messages = [value for value in contents if value == marker + "provider-failure"]
    media_messages = [value for value in contents if value == marker + "media-stress"]
    restart_messages = db.execute("SELECT COUNT(*) FROM messages WHERE content LIKE 'restartprobe%'").fetchone()[0]
    active_external = db.execute("SELECT COUNT(*) FROM external_jobs WHERE state IN ('pending','leased')").fetchone()[0]
    restore_jobs = db.execute("SELECT j.state,j.safe_error_code,d.state,d.safe_error_code FROM external_jobs j JOIN webhook_deliveries d ON d.external_job_id=j.id AND d.delivery_id=j.resource_id WHERE j.deduplication_key=? AND j.operation_type='webhook_delivery'", ("qualification-restore:" + marker,)).fetchall()
    duplicates = db.execute("SELECT COUNT(*) FROM (SELECT deduplication_key FROM external_jobs WHERE deduplication_key IS NOT NULL GROUP BY deduplication_key HAVING COUNT(*)>1)").fetchone()[0]
    media_keys = [row[0] for row in db.execute("SELECT storage_key FROM attachments WHERE media_state IN ('ready','attached')")]
missing_media = [key for key in media_keys if not (pathlib.Path(media_root) / key).is_file()]
cookie = json.load(open(seed_result, encoding="utf-8"))["metrics_session"]
request = urllib.request.Request(origin + "/api/me", headers={"Cookie": "concord_session=" + cookie})
try:
    with urllib.request.urlopen(request, timeout=5) as response:
        old_session_status = response.status
except urllib.error.HTTPError as error:
    old_session_status = error.code
accepted = (
    len(main_messages) == expected_main
    and len(provider_messages) == 1
    and len(media_messages) == 1
    and restart_messages == 1
    and old_session_status == 401
    and restore_jobs == [("failed", "restore_reconciliation_required", "failed", "restore_reconciliation_required")]
)
report = {
    "action": "restore",
    "ready": True,
    "declared_restore_point": restore_output.strip().split()[0] if restore_output.strip() else "",
    "integrity": integrity,
    "foreign_key_violations": len(foreign_keys),
    "missing_media": len(missing_media),
    "external_jobs_paused": "external_jobs_paused=true" in restore_output and active_external == 0,
    "restored_pending_jobs_reconciled": len(restore_jobs),
    "duplicate_publications": duplicates,
    "accepted_messages_verified": accepted,
    "restored_main_messages": len(main_messages),
    "restored_provider_messages": len(provider_messages),
    "restored_media_stress_messages": len(media_messages),
    "restored_restart_messages": restart_messages,
    "old_session_invalidated": old_session_status == 401,
}
if integrity != "ok" or foreign_keys or missing_media or not report["external_jobs_paused"] or duplicates or not accepted:
    raise SystemExit("restored instance verification failed")
print(json.dumps(report, sort_keys=True))
PY
}

action="${1:-}"
shift || true
case "${action}" in
  serve)
    supervisor
    ;;
  restart)
    exec 9> "${TARGET_CONTROL_LOCK}"
    flock -w 30 9
    request_supervisor restart ready
    ;;
  provider-status)
    webhook_id="${1:-}"
    [[ "${webhook_id}" =~ ^[A-Za-z0-9._:-]+$ ]] || exit 64
    python3 - "$(active_database)" "${webhook_id}" <<'PY'
import json, sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    row = db.execute("SELECT d.state,d.attempt_count,d.safe_error_code,d.last_status,j.state,j.attempt_count,j.safe_error_code FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id WHERE d.webhook_id=? ORDER BY d.created_at DESC,d.id DESC LIMIT 1", (sys.argv[2],)).fetchone()
print(json.dumps({"found": row is not None, "delivery_state": row[0] if row else None, "delivery_attempts": row[1] if row else 0, "delivery_error": row[2] if row else None, "last_status": row[3] if row else None, "job_state": row[4] if row else None, "job_attempts": row[5] if row else 0, "job_error": row[6] if row else None}, sort_keys=True))
PY
    ;;
  provider-arm|provider-disarm)
    webhook_id="${1:-}"
    [[ "${webhook_id}" =~ ^[A-Za-z0-9._:-]+$ ]] || exit 64
    python3 - "$(active_database)" "${webhook_id}" "${action}" <<'PY'
import json, sqlite3, sys, uuid
database, webhook_id, action = sys.argv[1:]
with sqlite3.connect(database) as db:
    if action == "provider-arm":
        db.execute("INSERT OR IGNORE INTO webhook_events(id,webhook_id,event_type) VALUES(?,?,'message_create')", (str(uuid.uuid4()), webhook_id))
        result = {"armed": True}
    else:
        db.execute("DELETE FROM webhook_events WHERE webhook_id=? AND event_type='message_create'", (webhook_id,))
        db.execute("UPDATE external_jobs SET state='cancelled',lease_owner=NULL,lease_token=NULL,lease_until=NULL,updated_at=datetime('now') WHERE id IN (SELECT external_job_id FROM webhook_deliveries WHERE webhook_id=?) AND state IN ('pending','leased')", (webhook_id,))
        db.execute("UPDATE webhook_deliveries SET state='cancelled',safe_error_code=COALESCE(safe_error_code,'qualification_provider_disarmed') WHERE webhook_id=? AND state IN ('pending','leased')", (webhook_id,))
        result = {"disarmed": True}
print(json.dumps(result, sort_keys=True))
PY
    ;;
  restore)
    exec 9> "${TARGET_CONTROL_LOCK}"
    flock -w 30 9
    restore_target "${1:-}" "${2:-}"
    ;;
  shutdown)
    exec 9> "${TARGET_CONTROL_LOCK}"
    flock -w 30 9
    request_supervisor stop stopped >/dev/null
    supervisor_pid="$(cat "${TARGET_SUPERVISOR_PID_FILE}")"
    kill "${supervisor_pid}" 2>/dev/null || true
    for pid_file in "${target_root}/run/tls-proxy.pid" "${target_root}/run/telemetry.pid"; do
      if [[ -r "${pid_file}" ]]; then
        service_pid="$(cat "${pid_file}")"
        [[ "${service_pid}" =~ ^[0-9]+$ ]] && kill "${service_pid}" 2>/dev/null || true
      fi
    done
    printf '{"action":"shutdown","stopped":true}\n'
    ;;
  *)
    echo "action must be serve, restart, provider-status, provider-arm, provider-disarm, restore, or shutdown" >&2
    exit 64
    ;;
esac
