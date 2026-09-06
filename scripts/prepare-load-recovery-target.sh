#!/usr/bin/env bash
set -euo pipefail
umask 077

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_root="${CONCORD_QUAL_TARGET_ROOT:?set an absolute fresh CONCORD_QUAL_TARGET_ROOT on the dedicated target}"
target_host="${CONCORD_QUAL_TARGET_HOST:?set the generator-reachable target DNS name or IPv4 address}"
tls_server_name="${CONCORD_QUAL_TLS_SERVER_NAME:-${target_host}}"
backend_port="${CONCORD_QUAL_BACKEND_PORT:-18080}"
web_port="${CONCORD_QUAL_WEB_PORT:-18443}"
irc_port="${CONCORD_QUAL_IRC_PORT:-16697}"
telemetry_port="${CONCORD_QUAL_TELEMETRY_PORT:-19095}"

[[ "${target_root}" == /* && "${target_host}" =~ ^[A-Za-z0-9.-]+$ \
  && "${tls_server_name}" =~ ^[A-Za-z0-9.-]+$ ]] || {
  echo "target root/host/TLS server name is invalid" >&2
  exit 64
}
for port in "${backend_port}" "${web_port}" "${irc_port}" "${telemetry_port}"; do
  [[ "${port}" =~ ^[0-9]+$ && "${port}" -ge 1024 && "${port}" -le 65535 ]] || {
    echo "qualification ports must be integers from 1024 through 65535" >&2
    exit 64
  }
done
[[ ! -e "${target_root}" ]] || {
  echo "CONCORD_QUAL_TARGET_ROOT must not already exist" >&2
  exit 1
}
for command in cargo rustc openssl curl python3 sha256sum findmnt flock nohup; do
  command -v "${command}" >/dev/null || {
    echo "required target command is unavailable: ${command}" >&2
    exit 1
  }
done

cpu_count="$(nproc)"
memory_bytes="$(awk '/^MemTotal:/ {print $2 * 1024}' /proc/meminfo | cut -d. -f1)"
python3 - "${cpu_count}" "${memory_bytes}" <<'PY'
import sys
cpu, memory = map(int, sys.argv[1:])
if cpu != 4 or not 7.5 * 1024**3 <= memory <= 9 * 1024**3:
    raise SystemExit("dedicated target must have 4 vCPUs and 7.5-9 GiB of reported memory")
PY

mkdir -p \
  "${target_root}/bin" "${target_root}/live/media" "${target_root}/logs" \
  "${target_root}/private" "${target_root}/run" "${target_root}/seed-output" \
  "${target_root}/export"

cleanup_failure() {
  status=$?
  if [[ "${status}" -ne 0 ]]; then
    for file in telemetry.pid tls-proxy.pid supervisor.pid server.pid; do
      if [[ -r "${target_root}/run/${file}" ]]; then
        pid="$(cat "${target_root}/run/${file}")"
        [[ "${pid}" =~ ^[0-9]+$ ]] && kill "${pid}" 2>/dev/null || true
      fi
    done
  fi
  exit "${status}"
}
trap cleanup_failure EXIT

(
  cd "${repository_root}/concord"
  cargo build --release --locked --features browser-fixtures \
    --bin concord-server --bin concord_operator --bin load_qualification_seed
)
install -m 755 "${repository_root}/concord/target/release/concord-server" "${target_root}/bin/concord-server"
install -m 755 "${repository_root}/concord/target/release/concord_operator" "${target_root}/bin/concord-operator"
install -m 755 "${repository_root}/concord/target/release/load_qualification_seed" "${target_root}/bin/load-qualification-seed"
install -m 755 "${repository_root}/scripts/load-recovery-target-control.sh" "${target_root}/bin/target-control"
install -m 755 "${repository_root}/scripts/serve-load-recovery-tls-proxy.py" "${target_root}/bin/tls-proxy"
install -m 755 "${repository_root}/scripts/serve-load-recovery-telemetry.py" "${target_root}/bin/telemetry"

openssl rand -hex 48 > "${target_root}/live/jwt.key"
openssl rand -hex 32 > "${target_root}/live/external.key"
openssl rand -hex 32 > "${target_root}/telemetry.token"

openssl req -x509 -newkey rsa:3072 -nodes -days 2 -sha256 \
  -subj "/CN=Concord qualification CA" \
  -keyout "${target_root}/private/ca.key" -out "${target_root}/qualification-ca.pem" \
  >/dev/null 2>&1
san_kind="DNS"
python3 - "${target_host}" >/dev/null 2>&1 <<'PY' && san_kind="IP" || true
import ipaddress, sys
ipaddress.ip_address(sys.argv[1])
PY
cat > "${target_root}/private/tls.cnf" <<EOF
[req]
prompt = no
distinguished_name = dn
req_extensions = ext
[dn]
CN = ${tls_server_name}
[ext]
subjectAltName = DNS:${tls_server_name},${san_kind}:${target_host}
extendedKeyUsage = serverAuth
EOF
openssl req -new -newkey rsa:3072 -nodes -sha256 \
  -config "${target_root}/private/tls.cnf" \
  -keyout "${target_root}/live/tls.key" -out "${target_root}/private/tls.csr" \
  >/dev/null 2>&1
openssl x509 -req -days 2 -sha256 \
  -in "${target_root}/private/tls.csr" \
  -CA "${target_root}/qualification-ca.pem" -CAkey "${target_root}/private/ca.key" \
  -CAcreateserial -extfile "${target_root}/private/tls.cnf" -extensions ext \
  -out "${target_root}/live/tls.crt" >/dev/null 2>&1
chmod 600 "${target_root}/live/"*.key "${target_root}/telemetry.token"

public_origin="https://${target_host}:${web_port}"
cat > "${target_root}/live/concord.toml" <<EOF
[server]
web_address = "127.0.0.1:${backend_port}"
irc_address = "0.0.0.0:${irc_port}"
irc_tls_cert = "${target_root}/live/tls.crt"
irc_tls_key = "${target_root}/live/tls.key"
shutdown_timeout_seconds = 10
[database]
url = "sqlite:${target_root}/live/concord.db?mode=rwc"
[auth]
jwt_secret_file = "${target_root}/live/jwt.key"
external_credentials_key_file = "${target_root}/live/external.key"
session_expiry_hours = 2
public_url = "${public_origin}"
[storage]
data_dir = "${target_root}/live"
media_dir = "${target_root}/live/media"
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
chmod 600 "${target_root}/live/concord.toml"

CONCORD_QUAL_SEED_PROFILE=full \
CONCORD_QUAL_SEED_DATABASE_URL="sqlite:${target_root}/live/concord.db?mode=rwc" \
CONCORD_QUAL_SEED_JWT_SECRET="$(cat "${target_root}/live/jwt.key")" \
CONCORD_QUAL_SEED_EXTERNAL_KEY_FILE="${target_root}/live/external.key" \
CONCORD_QUAL_SEED_OUTPUT_DIR="${target_root}/seed-output" \
  "${target_root}/bin/load-qualification-seed" > "${target_root}/logs/seed.log"
python3 - "${target_root}/live/concord.db" <<'PY'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as database:
    database.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    integrity = database.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = database.execute("PRAGMA foreign_key_check").fetchall()
    messages = database.execute("SELECT COUNT(*) FROM messages").fetchone()[0]
    journal = database.execute("PRAGMA journal_mode").fetchone()[0]
    synchronous = database.execute("PRAGMA synchronous").fetchone()[0]
if integrity != "ok" or foreign_keys or messages != 1_000_000 or journal.lower() != "wal" or synchronous != 2:
    raise SystemExit("seeded dataset failed FULL/WAL integrity or count validation")
PY

printf '%s\n' "${target_root}/live/concord.toml" > "${target_root}/run/active-config"
printf '%s\n' "${target_root}/live/concord.db" > "${target_root}/run/active-database"
printf '%s\n' "${target_root}/live/media" > "${target_root}/run/active-media"

runtime_env="${target_root}/runtime.env"
: > "${runtime_env}"
write_runtime() {
  printf '%s=%q\n' "$1" "$2" >> "${runtime_env}"
}
write_runtime TARGET_RUN_DIR "${target_root}/run"
write_runtime TARGET_LOG_DIR "${target_root}/logs"
write_runtime TARGET_PRIVATE_DIR "${target_root}/private"
write_runtime TARGET_SERVER_BIN "${target_root}/bin/concord-server"
write_runtime TARGET_OPERATOR_BIN "${target_root}/bin/concord-operator"
write_runtime TARGET_SERVER_PID_FILE "${target_root}/run/server.pid"
write_runtime TARGET_SUPERVISOR_PID_FILE "${target_root}/run/supervisor.pid"
write_runtime TARGET_REQUEST_FILE "${target_root}/run/supervisor.request"
write_runtime TARGET_ACK_FILE "${target_root}/run/supervisor.ack"
write_runtime TARGET_CONTROL_LOCK "${target_root}/run/control.lock"
write_runtime TARGET_ACTIVE_CONFIG_FILE "${target_root}/run/active-config"
write_runtime TARGET_ACTIVE_DATABASE_FILE "${target_root}/run/active-database"
write_runtime TARGET_ACTIVE_MEDIA_FILE "${target_root}/run/active-media"
write_runtime TARGET_BACKEND_BIND "127.0.0.1:${backend_port}"
write_runtime TARGET_BACKEND_ORIGIN "http://127.0.0.1:${backend_port}"
write_runtime TARGET_IRC_BIND "0.0.0.0:${irc_port}"
write_runtime TARGET_PUBLIC_ORIGIN "${public_origin}"
write_runtime TARGET_TLS_CERT "${target_root}/live/tls.crt"
write_runtime TARGET_TLS_KEY "${target_root}/live/tls.key"
write_runtime TARGET_SEED_RESULT "${target_root}/seed-output/seed-result.json"
chmod 600 "${runtime_env}"

database_sha256="$(sha256sum "${target_root}/live/concord.db" | cut -d ' ' -f 1)"
server_sha256="$(sha256sum "${target_root}/bin/concord-server" | cut -d ' ' -f 1)"
seed_sha256="$(sha256sum "${target_root}/bin/load-qualification-seed" | cut -d ' ' -f 1)"
config_sha256="$(sha256sum "${target_root}/live/concord.toml" | cut -d ' ' -f 1)"
query_sha256="$(sha256sum "${target_root}/seed-output/query-plan.json" | cut -d ' ' -f 1)"
filesystem="$(findmnt -n -o FSTYPE -T "${target_root}")"
storage="$(findmnt -n -o SOURCE,OPTIONS -T "${target_root}" | tr '\n' ' ')"
source_revision="$(git -C "${repository_root}" rev-parse HEAD)"
rustc_version="$(cd "${repository_root}/concord" && rustc --version)"
python3 - \
  "${target_root}/server-metadata.json" "${cpu_count}" "${memory_bytes}" \
  "${filesystem}" "${storage}" "${server_sha256}" "${source_revision}" \
  "${rustc_version}" "${seed_sha256}" "${config_sha256}" "${query_sha256}" \
  "${database_sha256}" <<'PY'
import json, os, pathlib, socket, sys
(
    output, cpu, memory, filesystem, storage, server_hash, revision, rustc,
    seed_hash, config_hash, query_hash, dataset_hash,
) = sys.argv[1:]
value = {
    "hostname": socket.gethostname(),
    "cpu_count": int(cpu),
    "memory_bytes": int(memory),
    "filesystem": filesystem,
    "storage": storage,
    "kernel": os.uname().release,
    "server_sha256": server_hash,
    "source_revision": revision,
    "rustc": rustc,
    "release_flags": "--release --locked --features browser-fixtures",
    "seed_sha256": seed_hash,
    "config_sha256": config_hash,
    "query_mix_sha256": query_hash,
    "dataset_sha256": dataset_hash,
    "seeded_messages": 1_000_000,
    "database_profile": "FULL/WAL",
    "configured_max_upload_bytes": 100 * 1024 * 1024,
}
pathlib.Path(output).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

nohup env CONCORD_QUAL_TARGET_ROOT="${target_root}" \
  "${target_root}/bin/target-control" serve \
  > "${target_root}/logs/supervisor.log" 2>&1 &
printf '%s\n' "$!" > "${target_root}/run/supervisor.launch.pid"
nohup "${target_root}/bin/tls-proxy" \
  --listen "0.0.0.0:${web_port}" --upstream "127.0.0.1:${backend_port}" \
  --certificate "${target_root}/live/tls.crt" --private-key "${target_root}/live/tls.key" \
  > "${target_root}/logs/tls-proxy.log" 2>&1 &
printf '%s\n' "$!" > "${target_root}/run/tls-proxy.pid"
nohup "${target_root}/bin/telemetry" \
  --listen "0.0.0.0:${telemetry_port}" \
  --server-pid-file "${target_root}/run/server.pid" \
  --database "${target_root}/live/concord.db" --web-port "${web_port}" --irc-port "${irc_port}" \
  --token-file "${target_root}/telemetry.token" \
  > "${target_root}/logs/telemetry.log" 2>&1 &
printf '%s\n' "$!" > "${target_root}/run/telemetry.pid"

for _ in $(seq 1 300); do
  curl --fail --silent --max-time 2 "http://127.0.0.1:${backend_port}/health/ready" >/dev/null && break
  sleep 0.1
done
curl --fail --silent --max-time 2 "http://127.0.0.1:${backend_port}/health/ready" >/dev/null
python3 - "${target_host}" "${web_port}" "${tls_server_name}" "${target_root}/qualification-ca.pem" <<'PY'
import socket, ssl, sys
host, port, server_name, ca_file = sys.argv[1:]
context = ssl.create_default_context(cafile=ca_file)
with socket.create_connection((host, int(port)), timeout=5) as raw:
    with context.wrap_socket(raw, server_hostname=server_name) as secured:
        if not secured.version():
            raise SystemExit("TLS qualification proxy did not negotiate a protocol")
PY

cp "${target_root}/server-metadata.json" "${target_root}/qualification-ca.pem" \
  "${target_root}/telemetry.token" "${target_root}/export/"
cp "${target_root}/seed-output/"*.json "${target_root}/export/"
python3 - "${target_root}/export/target-connection.json" "${target_host}" \
  "${tls_server_name}" "${web_port}" "${irc_port}" "${telemetry_port}" "${target_root}" <<'PY'
import json, pathlib, sys
output, host, tls_name, web, irc, telemetry, root = sys.argv[1:]
value = {
    "target_host": host,
    "tls_server_name": tls_name,
    "http_origin": f"https://{host}:{web}",
    "irc_port": int(irc),
    "telemetry_url": f"http://{host}:{telemetry}/",
    "remote_target_root": root,
}
pathlib.Path(output).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
chmod 600 "${target_root}/export/"*

trap - EXIT
printf 'qualification target ready; securely copy %s/export to the separate generator host\n' "${target_root}"
