#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$(mktemp -d)"
evidence_root="${CONCORD_AUTH_BROWSER_EVIDENCE:-${repository_root}/.design/concord-remediation/evidence/authenticated-browser}"
evidence_dir="${evidence_root}/$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "${evidence_dir}"
server_pid=""
receiver_pid=""
cleanup() {
  status=$?
  if [[ -n "${receiver_pid}" ]]; then
    kill "${receiver_pid}" 2>/dev/null || true
    wait "${receiver_pid}" 2>/dev/null || true
  fi
  if [[ -f "${fixture_root}/server-child.pid" ]]; then
    fixture_child_pid="$(cat "${fixture_root}/server-child.pid")"
    kill "${fixture_child_pid}" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "${fixture_child_pid}" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "${fixture_child_pid}" 2>/dev/null || true
    wait "${fixture_child_pid}" 2>/dev/null || true
  fi
  if [[ -n "${server_pid}" ]]; then
    kill "${server_pid}" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "${server_pid}" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 "${server_pid}" 2>/dev/null; then kill -KILL "${server_pid}" 2>/dev/null || true; fi
    wait "${server_pid}" 2>/dev/null || true
  fi
  if [[ ${status} -eq 0 ]]; then
    rm -rf "${fixture_root}"
  else
    rm -f "${fixture_root}/sessions.json" "${fixture_root}/jwt.key" \
      "${fixture_root}/external.key" "${fixture_root}/concord.toml" "${fixture_root}/concord.db"*
    echo "sanitized browser fixture evidence retained at ${fixture_root}" >&2
  fi
  printf 'authenticated-browser status=%s evidence=%s\n' "${status}" "${evidence_dir}" >&2
  exit "${status}"
}
trap cleanup EXIT

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}
backend_port="$(free_port)"
frontend_port="$(free_port)"
irc_port="$(free_port)"
receiver_port="$(free_port)"
receiver_log="${fixture_root}/webhook-receiver.jsonl"

mkdir -p "${fixture_root}/media" "${fixture_root}/static" "${fixture_root}/bin"
database_url="sqlite:${fixture_root}/concord.db?mode=rwc"
jwt_secret="browser-fixture-secret-with-at-least-thirty-two-bytes"
printf '%064d\n' 0 > "${fixture_root}/external.key"
printf '%s\n' "${jwt_secret}" > "${fixture_root}/jwt.key"
chmod 600 "${fixture_root}/external.key" "${fixture_root}/jwt.key"
: > "${receiver_log}"

python3 -u - "${receiver_port}" "${receiver_log}" <<'PY' &
import http.server
import json
import pathlib
import sys
import threading

port = int(sys.argv[1])
log_path = pathlib.Path(sys.argv[2])
lock = threading.Lock()
requests_by_path = {}

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(204)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length).decode("utf-8")
        with lock:
            attempt = requests_by_path.get(self.path, 0) + 1
            requests_by_path[self.path] = attempt
            status = 400 if self.path == "/fail-once" and attempt == 1 else 204
            record = {
                "path": self.path,
                "attempt": attempt,
                "status": status,
                "headers": {key.lower(): value for key, value in self.headers.items()},
                "body": body,
            }
            with log_path.open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(record, sort_keys=True) + "\n")
                stream.flush()
        self.send_response(status)
        self.end_headers()

    def log_message(self, _format, *_args):
        return None

server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
server.serve_forever()
PY
receiver_pid=$!
for _ in $(seq 1 50); do
  if curl --silent --output /dev/null "http://127.0.0.1:${receiver_port}/ready"; then break; fi
  if ! kill -0 "${receiver_pid}" 2>/dev/null; then exit 1; fi
  sleep 0.1
done

cat > "${fixture_root}/concord.toml" <<EOF
[server]
web_address = "127.0.0.1:${backend_port}"
irc_address = "127.0.0.1:${irc_port}"
shutdown_timeout_seconds = 5
[database]
url = "${database_url}"
[auth]
jwt_secret_file = "${fixture_root}/jwt.key"
external_credentials_key_file = "${fixture_root}/external.key"
session_expiry_hours = 1
public_url = "http://127.0.0.1:${frontend_port}"
[storage]
data_dir = "${fixture_root}"
media_dir = "${fixture_root}/media"
max_file_size_mb = 10
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
  cargo build --quiet --locked \
    --features browser-fixtures --bin browser_fixture_seed --bin concord-server
)
rustc_version="$(cd "${repository_root}/concord" && rustc --version)"
install -m 755 "${repository_root}/concord/target/debug/concord-server" "${fixture_root}/bin/concord-server"
install -m 755 "${repository_root}/concord/target/debug/browser_fixture_seed" "${fixture_root}/bin/browser_fixture_seed"
server_sha256="$(sha256sum "${fixture_root}/bin/concord-server" | cut -d ' ' -f 1)"
seed_sha256="$(sha256sum "${fixture_root}/bin/browser_fixture_seed" | cut -d ' ' -f 1)"
printf 'immutable_browser_fixture server_sha256=%s seed_sha256=%s\n' "${server_sha256}" "${seed_sha256}"
python3 - "${server_sha256}" "${seed_sha256}" "${rustc_version}" \
  > "${evidence_dir}/artifact-manifest.json" <<'PY'
import json
import sys

server_sha256, seed_sha256, rustc = sys.argv[1:]
print(json.dumps({
    "server_sha256": server_sha256,
    "seed_sha256": seed_sha256,
    "rustc": rustc,
}, sort_keys=True))
PY
CONCORD_FIXTURE_DATABASE_URL="${database_url}" CONCORD_FIXTURE_JWT_SECRET="${jwt_secret}" \
  CONCORD_FIXTURE_EXTERNAL_KEY_FILE="${fixture_root}/external.key" \
  "${fixture_root}/bin/browser_fixture_seed" > "${fixture_root}/sessions.json"
restart_request="${fixture_root}/restart.request"
restart_ack="${fixture_root}/restart.ack"
server_supervisor() (
  child_pid=""
  stop_child() {
    if [[ -n "${child_pid}" ]]; then
      kill "${child_pid}" 2>/dev/null || true
      wait "${child_pid}" 2>/dev/null || true
    fi
  }
  trap 'stop_child; exit 0' TERM INT
  start_child() {
    CONCORD_BROWSER_EGRESS_FIXTURE_ADDR="127.0.0.1:${receiver_port}" \
      "${fixture_root}/bin/concord-server" --config "${fixture_root}/concord.toml" \
      >> "${fixture_root}/server.log" 2>&1 &
    child_pid=$!
    printf '%s\n' "${child_pid}" > "${fixture_root}/server-child.pid"
  }
  start_child
  while kill -0 "${child_pid}" 2>/dev/null; do
    if [[ -f "${restart_request}" ]]; then
      rm -f "${restart_request}" "${restart_ack}"
      stop_child
      # Keep a deterministic outage window so browser actions can prove their
      # disconnected behavior instead of racing an immediate local restart.
      sleep 1
      start_child
      ready=0
      for _ in $(seq 1 100); do
        if curl --fail --silent "http://127.0.0.1:${backend_port}/health/ready" >/dev/null; then ready=1; break; fi
        kill -0 "${child_pid}" 2>/dev/null || break
        sleep 0.1
      done
      if [[ "${ready}" -ne 1 ]]; then
        stop_child
        exit 1
      fi
      printf '%s\n' "${child_pid}" > "${restart_ack}"
    fi
    sleep 0.05
  done
  wait "${child_pid}" 2>/dev/null || true
  exit 1
)
server_supervisor &
server_pid=$!
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:${backend_port}/health/ready" >/dev/null; then break; fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then cat "${fixture_root}/server.log" >&2; exit 1; fi
  sleep 0.1
done
curl --fail --silent "http://127.0.0.1:${backend_port}/health/ready" >/dev/null


CONCORD_AUTH_SESSIONS_FILE="${fixture_root}/sessions.json" \
CONCORD_BACKEND_URL="http://127.0.0.1:${backend_port}" CONCORD_FRONTEND_PORT="${frontend_port}" \
CONCORD_IRC_PORT="${irc_port}" CONCORD_RESTART_REQUEST="${restart_request}" CONCORD_RESTART_ACK="${restart_ack}" \
CONCORD_WEBHOOK_RECEIVER_PORT="${receiver_port}" CONCORD_WEBHOOK_RECEIVER_LOG="${receiver_log}" \
  bash -c 'cd "$1" && shift && npx playwright test --config playwright.authenticated.config.ts "$@"' \
  _ "${repository_root}/concord/web" "$@" 2>&1 | tee "${fixture_root}/playwright.log"
cp "${fixture_root}/playwright.log" "${evidence_dir}/playwright.log"
cp "${receiver_log}" "${evidence_dir}/webhook-receiver.jsonl"
