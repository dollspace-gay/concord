#!/usr/bin/env bash
# Sourced by run-load-recovery-qualification.sh; the caller owns cleanup.
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
