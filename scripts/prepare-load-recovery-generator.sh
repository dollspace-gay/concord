#!/usr/bin/env bash
set -euo pipefail
umask 077

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_dir="${CONCORD_QUAL_BUNDLE_DIR:?set CONCORD_QUAL_BUNDLE_DIR to the securely copied target export}"
ssh_target="${CONCORD_QUAL_SSH_TARGET:?set the SSH target used to control the dedicated server}"
source_ips="${CONCORD_QUAL_SOURCE_IPS:?set comma-separated generator source addresses already assigned by the host network}"
output_file="${CONCORD_QUAL_ENV_FILE:-${bundle_dir}/run-full.env}"

[[ -d "${bundle_dir}" && "${output_file}" == /* ]] || {
  echo "bundle directory must exist and CONCORD_QUAL_ENV_FILE must be absolute" >&2
  exit 64
}
for file in \
  target-connection.json server-metadata.json qualification-ca.pem telemetry.token \
  irc-tokens.json web-sessions.json channel-inventory.json query-plan.json \
  permission-race-plan.json seed-result.json; do
  [[ -r "${bundle_dir}/${file}" ]] || {
    echo "target export is incomplete: ${file}" >&2
    exit 1
  }
done

python3 - "${bundle_dir}" "${source_ips}" <<'PY'
import hashlib, ipaddress, json, socket, sys
from pathlib import Path
root = Path(sys.argv[1])
source_ips = [value for value in sys.argv[2].split(",") if value]
if len(source_ips) < 200 or len(source_ips) != len(set(source_ips)):
    raise SystemExit("generator host requires at least 200 unique assigned source addresses")
for value in source_ips:
    if ipaddress.ip_address(value).version != 4:
        raise SystemExit("generator source inventory currently requires IPv4 addresses")
    with socket.socket(socket.AF_INET) as probe:
        probe.bind((value, 0))
metadata = json.load(open(root / "server-metadata.json", encoding="utf-8"))
required = {
    "hostname", "cpu_count", "memory_bytes", "filesystem", "storage", "kernel",
    "server_sha256", "source_revision", "rustc", "release_flags", "seed_sha256",
    "config_sha256", "query_mix_sha256", "dataset_sha256", "seeded_messages",
    "database_profile", "configured_max_upload_bytes",
}
if required - metadata.keys():
    raise SystemExit("server metadata is incomplete")
if hashlib.sha256((root / "query-plan.json").read_bytes()).hexdigest() != metadata["query_mix_sha256"]:
    raise SystemExit("query plan hash does not match target metadata")
tokens = json.load(open(root / "irc-tokens.json", encoding="utf-8"))
web = json.load(open(root / "web-sessions.json", encoding="utf-8"))
channels = json.load(open(root / "channel-inventory.json", encoding="utf-8"))
seed = json.load(open(root / "seed-result.json", encoding="utf-8"))
if len(tokens) != 800 or len(web) != 200 or len(channels) != 50:
    raise SystemExit("credential/channel inventories do not have exact full-qualification counts")
if any(not isinstance(token, str) or not token for token in tokens):
    raise SystemExit("IRC inventory contains an empty credential")
if not seed.get("metrics_session") or not seed.get("provider_failure_webhook_id"):
    raise SystemExit("seed result lacks runtime credentials")
PY

readarray -t connection < <(python3 - "${bundle_dir}/target-connection.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
for field in ("target_host", "tls_server_name", "http_origin", "irc_port", "telemetry_url", "remote_target_root"):
    print(value[field])
PY
)
readarray -t seed < <(python3 - "${bundle_dir}/seed-result.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
print(value["metrics_session"])
print(value["provider_failure_webhook_id"])
PY
)
telemetry_token="$(cat "${bundle_dir}/telemetry.token")"

control_wrapper="${bundle_dir}/qualification-control"
install -m 700 "${repository_root}/scripts/load-recovery-control-ssh.py" "${control_wrapper}"

: > "${output_file}"
write_export() {
  printf 'export %s=%q\n' "$1" "$2" >> "${output_file}"
}
write_export CONCORD_QUALIFICATION_MODE full
write_export CONCORD_QUAL_IRC_HOST "${connection[0]}"
write_export CONCORD_QUAL_IRC_PORT "${connection[3]}"
write_export CONCORD_QUAL_HTTP_ORIGIN "${connection[2]}"
write_export CONCORD_QUAL_SERVER_TELEMETRY_URL "${connection[4]}"
write_export CONCORD_QUAL_TELEMETRY_TOKEN "${telemetry_token}"
write_export CONCORD_QUAL_METRICS_SESSION "${seed[0]}"
write_export CONCORD_QUAL_IRC_TOKENS_FILE "${bundle_dir}/irc-tokens.json"
write_export CONCORD_QUAL_CHANNELS_FILE "${bundle_dir}/channel-inventory.json"
write_export CONCORD_QUAL_WEB_SESSIONS_FILE "${bundle_dir}/web-sessions.json"
write_export CONCORD_QUAL_QUERY_PLAN "${bundle_dir}/query-plan.json"
write_export CONCORD_QUAL_PERMISSION_RACE_PLAN "${bundle_dir}/permission-race-plan.json"
write_export CONCORD_QUAL_CONTROL_COMMAND "${control_wrapper}"
write_export CONCORD_QUAL_PROVIDER_WEBHOOK_ID "${seed[1]}"
write_export CONCORD_QUAL_MAX_UPLOAD_BYTES 104857600
write_export CONCORD_QUAL_SERVER_METADATA "${bundle_dir}/server-metadata.json"
write_export CONCORD_QUAL_DATASET_SHA256 "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["dataset_sha256"])' "${bundle_dir}/server-metadata.json")"
write_export CONCORD_QUAL_IRC_CA_FILE "${bundle_dir}/qualification-ca.pem"
write_export CONCORD_QUAL_IRC_TLS_SERVER_NAME "${connection[1]}"
write_export CONCORD_QUAL_HTTP_CA_FILE "${bundle_dir}/qualification-ca.pem"
write_export CONCORD_QUAL_SOURCE_IPS "${source_ips}"
write_export CONCORD_QUAL_SSH_TARGET "${ssh_target}"
write_export CONCORD_QUAL_REMOTE_TARGET_ROOT "${connection[5]}"
chmod 600 "${output_file}"

set +u
# shellcheck source=/dev/null
source "${output_file}"
set -u
"${CONCORD_QUAL_CONTROL_COMMAND}" provider-status "${CONCORD_QUAL_PROVIDER_WEBHOOK_ID}" >/dev/null
printf 'generator environment ready: source %s, then run %s/scripts/run-load-recovery-qualification.sh\n' \
  "${output_file}" "${repository_root}"
