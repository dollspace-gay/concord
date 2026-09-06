#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${CONCORD_QUALIFICATION_MODE:-smoke}"
evidence_root="${CONCORD_QUALIFICATION_EVIDENCE:-${repository_root}/.design/concord-remediation/evidence/load-recovery}"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
evidence_dir="${evidence_root}/${run_id}"
mkdir -p "${evidence_dir}"

python3 "${repository_root}/scripts/test-load-recovery-modules.py"
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
    source "${repository_root}/scripts/qualification/load_recovery/smoke_setup.sh"
    ;;
  full)
    source "${repository_root}/scripts/qualification/load_recovery/full_setup.sh"
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
