#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$(mktemp -d)"
report_path="${CONCORD_IRC_QUALIFICATION_REPORT:-/tmp/concord-irc-client-qualification.log}"
server_pid=""
client_pids=()

cleanup() {
  status=$?
  for pid in "${client_pids[@]}"; do
    kill "${pid}" 2>/dev/null || true
    wait "${pid}" 2>/dev/null || true
  done
  if [[ -n "${server_pid}" ]]; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  if [[ ${status} -ne 0 ]]; then
    {
      printf 'Automated IRC real-client qualification failed (status %s)\n' "${status}"
      if [[ -x "${fixture_root}/bin/concord-server" ]]; then
        printf 'Server sha256=%s\n' "$(sha256sum "${fixture_root}/bin/concord-server" | cut -d' ' -f1)"
      fi
      if [[ -f "${fixture_root}/irssi.raw.log" ]]; then
        printf '%s\n' 'Sanitized Irssi protocol observations:'
        grep -E '(^| )(001|322|323) |JOIN |PRIVMSG |PING :|PONG :' "${fixture_root}/irssi.raw.log" | tail -n 40 || true
      fi
      if [[ -f "${fixture_root}/server.log" ]]; then
        printf '%s\n' 'Sanitized server lifecycle observations:'
        grep -E 'TLS enabled|IRC client (connected|disconnected)|session (connected|disconnected)' "${fixture_root}/server.log" | tail -n 40 || true
      fi
      if [[ -f "${fixture_root}/weechat.log" ]]; then
        printf '%s\n' 'Sanitized WeeChat observations:'
        grep -E 'irc:|error|warning|#[^ ]+|qualification-' "${fixture_root}/weechat.log" | tail -n 60 || true
      fi
      if [[ -f "${fixture_root}/concord.db" ]]; then
        python3 - "${fixture_root}/concord.db" <<'PY'
import sqlite3
import sys

with sqlite3.connect(sys.argv[1]) as connection:
    rows = connection.execute(
        "SELECT m.sender_id,m.conversation_id,m.content FROM messages m "
        "WHERE m.content LIKE 'qualification-%' ORDER BY m.created_at,m.id"
    ).fetchall()
print(f"Persisted qualification messages: {rows!r}")
PY
      fi
    } > "${report_path}"
    echo "sanitized failure evidence: ${report_path}" >&2
  fi
  python3 - "${fixture_root}" <<'PY'
import shutil
import sys

shutil.rmtree(sys.argv[1], ignore_errors=True)
PY
  exit "${status}"
}
trap cleanup EXIT

for command in curl irssi jq openssl script timeout weechat-headless; do
  command -v "${command}" >/dev/null || {
    echo "required qualification dependency is unavailable: ${command}" >&2
    exit 1
  }
done

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

backend_port="$(free_port)"
irc_port="$(free_port)"
database_url="sqlite:${fixture_root}/concord.db?mode=rwc"
jwt_secret="irc-qualification-secret-with-at-least-thirty-two-bytes"
mkdir -p "${fixture_root}/media" "${fixture_root}/static" "${fixture_root}/irssi" "${fixture_root}/weechat"
printf '%064d\n' 0 > "${fixture_root}/external.key"
printf '%s\n' "${jwt_secret}" > "${fixture_root}/jwt.key"
chmod 600 "${fixture_root}/external.key" "${fixture_root}/jwt.key"

openssl req -x509 -newkey rsa:2048 -nodes -days 1 -sha256 \
  -subj '/CN=127.0.0.1' -addext 'subjectAltName=IP:127.0.0.1' \
  -keyout "${fixture_root}/irc.key" -out "${fixture_root}/irc.crt" >/dev/null 2>&1
chmod 600 "${fixture_root}/irc.key"

cat > "${fixture_root}/concord.toml" <<EOF
[server]
web_address = "127.0.0.1:${backend_port}"
irc_address = "127.0.0.1:${irc_port}"
irc_tls_cert = "${fixture_root}/irc.crt"
irc_tls_key = "${fixture_root}/irc.key"
shutdown_timeout_seconds = 5
[database]
url = "${database_url}"
[auth]
jwt_secret_file = "${fixture_root}/jwt.key"
external_credentials_key_file = "${fixture_root}/external.key"
session_expiry_hours = 1
public_url = "http://127.0.0.1:${backend_port}"
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
mkdir -p "${fixture_root}/bin"
cp "${repository_root}/concord/target/debug/concord-server" "${fixture_root}/bin/concord-server"
cp "${repository_root}/concord/target/debug/browser_fixture_seed" "${fixture_root}/bin/browser_fixture_seed"
CONCORD_FIXTURE_DATABASE_URL="${database_url}" CONCORD_FIXTURE_JWT_SECRET="${jwt_secret}" \
  CONCORD_FIXTURE_EXTERNAL_KEY_FILE="${fixture_root}/external.key" \
  "${fixture_root}/bin/browser_fixture_seed" > "${fixture_root}/sessions.json"
irc_token="$(jq -er '.bob_irc' "${fixture_root}/sessions.json")"
weechat_irc_token="$(jq -er '.alice_irc' "${fixture_root}/sessions.json")"

"${fixture_root}/bin/concord-server" --config "${fixture_root}/concord.toml" \
  > "${fixture_root}/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 100); do
  curl --fail --silent "http://127.0.0.1:${backend_port}/health/ready" >/dev/null && break
  kill -0 "${server_pid}" 2>/dev/null || { cat "${fixture_root}/server.log" >&2; exit 1; }
  sleep 0.1
done
curl --fail --silent "http://127.0.0.1:${backend_port}/health/ready" >/dev/null

marker="qualification-$(date +%s)-$$"
irssi_input="${fixture_root}/irssi.input"
cat > "${fixture_root}/irssi/config" <<EOF
servers = (
  {
    address = "127.0.0.1";
    chatnet = "Concord";
    port = "${irc_port}";
    password = "${irc_token}";
    use_tls = "yes";
    tls_verify = "yes";
    tls_cafile = "${fixture_root}/irc.crt";
    autoconnect = "yes";
  }
);
chatnets = {
  Concord = { type = "IRC"; nick = "bob"; autosendcmd = ""; };
};
settings = {
  core = { real_name = "Concord qualification"; user_name = "bob"; nick = "bob"; };
};
EOF
chmod 600 "${fixture_root}/irssi/config"
cat > "${irssi_input}" <<EOF
/rawlog open ${fixture_root}/irssi.raw.log
/join #browser-fixture/general
/quote LIST #browser-fixture/*
/msg #browser-fixture/general ${marker}-irssi
/msg alice ${marker}-dm
/quote HISTORY #browser-fixture/general 20
EOF

# Irssi requires a terminal. `script` supplies a disposable PTY while the timed
# input stream exercises the packaged client binary against Concord TLS.
(
  sleep 2
  cat "${irssi_input}"
  sleep 55
  printf '/msg #browser-fixture/general %s-offline\n' "${marker}"
  sleep 10
  printf '/quit\n'
) | TERM=xterm timeout 72 script -qefc \
  "irssi --home=${fixture_root}/irssi --config=${fixture_root}/irssi/config" \
  "${fixture_root}/irssi.typescript" >/dev/null 2>&1 &
client_pids+=("$!")

# WeeChat is a separately implemented client. Its headless frontend records
# protocol traffic on stdout and reconnects after the first client disconnects.
# WeeChat expands the environment reference internally.
# shellcheck disable=SC2016
CONCORD_QUAL_IRC_TOKEN="${weechat_irc_token}" timeout 72 weechat-headless --stdout --dir "${fixture_root}/weechat" \
  --run-command "/set weechat.network.gnutls_ca_user ${fixture_root}/irc.crt" \
  --run-command "/set logger.file.auto_log on" \
  --run-command "/set logger.level.irc 9" \
  --run-command "/server add concord 127.0.0.1/${irc_port} -tls" \
  --run-command "/set irc.server.concord.tls_verify on" \
  --run-command '/set irc.server.concord.password ${env:CONCORD_QUAL_IRC_TOKEN}' \
  --run-command "/set irc.server.concord.nicks alice" \
  --run-command "/connect concord" \
  --run-command "/wait 3 /join -server concord #browser-fixture/general" \
  --run-command "/wait 5 /msg -server concord #browser-fixture/general ${marker}-weechat" \
  --run-command "/wait 6 /quote -server concord HISTORY #browser-fixture/general 20" \
  --run-command "/wait 55 /disconnect concord" \
  --run-command "/wait 58 /connect concord" \
  --run-command "/wait 60 /join -server concord #browser-fixture/general" \
  --run-command "/wait 62 /quote -server concord HISTORY #browser-fixture/general 20" \
  --run-command "/wait 66 /quit" \
  > "${fixture_root}/weechat.log" 2>&1 &
client_pids+=("$!")

for pid in "${client_pids[@]}"; do
  wait "${pid}"
done
client_pids=()

require_fixed() {
  pattern="$1"
  path="$2"
  description="$3"
  grep -Fq "${pattern}" "${path}" || {
    echo "missing IRC qualification observation: ${description}" >&2
    exit 1
  }
}
require_extended() {
  pattern="$1"
  path="$2"
  description="$3"
  grep -Eq "${pattern}" "${path}" || {
    echo "missing IRC qualification observation: ${description}" >&2
    exit 1
  }
}
require_fixed "${marker}-weechat" "${fixture_root}/irssi.raw.log" "Irssi received WeeChat's channel message"
require_extended '(^| )001 ' "${fixture_root}/irssi.raw.log" "registration numeric 001"
require_extended '(^| )322 ' "${fixture_root}/irssi.raw.log" "LIST row numeric 322"
require_extended '(^| )323 ' "${fixture_root}/irssi.raw.log" "LIST completion numeric 323"
require_fixed 'PING :' "${fixture_root}/irssi.raw.log" "server heartbeat PING"
grep -R -Fq -- "${marker}-offline" "${fixture_root}/weechat/logs" || {
  echo "missing IRC qualification observation: WeeChat recovered the disconnected-window message through HISTORY" >&2
  exit 1
}
python3 - "${fixture_root}/irssi.raw.log" <<'PY'
import re
import sys

raw = open(sys.argv[1], encoding="utf-8", errors="replace").read()
ping_tokens = re.findall(r"(?:^|\n)>> PING :([^\r\n ]+)", raw)
pong_tokens = set(re.findall(r"(?:^|\n)<< PONG :([^\r\n ]+)", raw))
if not ping_tokens or not any(token in pong_tokens for token in ping_tokens):
    raise SystemExit("real client did not return a token-correlated PONG")
PY
python3 - "${fixture_root}/concord.db" "${marker}-dm" "${marker}-irssi" "${marker}-weechat" "${marker}-offline" <<'PY'
import sqlite3
import sys

database, dm_marker, irssi_marker, weechat_marker, offline_marker = sys.argv[1:]
with sqlite3.connect(database) as connection:
    dm_count = connection.execute(
        "SELECT COUNT(*) FROM messages WHERE content=? AND conversation_id='browser-dm'",
        (dm_marker,),
    ).fetchone()[0]
    channel_count = connection.execute(
        "SELECT COUNT(*) FROM messages m JOIN channels c "
        "ON m.conversation_id='channel:' || hex(c.id) "
        "WHERE c.id='browser-general' AND m.content IN (?,?,?)",
        (irssi_marker, weechat_marker, offline_marker),
    ).fetchone()[0]
if dm_count != 1:
    raise SystemExit(f"expected exactly one persisted IRC direct message, observed {dm_count}")
if channel_count != 3:
    raise SystemExit(f"expected both clients plus offline-window channel messages, observed {channel_count}")
PY
require_fixed "TLS enabled" "${fixture_root}/server.log" "server TLS listener"
require_fixed "IRC client connected" "${fixture_root}/server.log" "accepted IRC connection"
connection_count="$(grep -Fc 'IRC client connected' "${fixture_root}/server.log")"
if (( connection_count < 3 )); then
  echo "expected two clients plus a real reconnect, observed ${connection_count} connections" >&2
  exit 1
fi

{
  printf 'Automated IRC real-client qualification passed\n'
  printf 'Irssi: %s sha256=%s\n' "$(irssi --version | head -n 1)" "$(sha256sum "$(command -v irssi)" | cut -d' ' -f1)"
  printf 'WeeChat: %s sha256=%s\n' "$(weechat-headless --version | head -n 1)" "$(sha256sum "$(command -v weechat-headless)" | cut -d' ' -f1)"
  printf 'Concord server sha256=%s\n' "$(sha256sum "${fixture_root}/bin/concord-server" | cut -d' ' -f1)"
  printf 'Rust toolchain: %s\n' "${rustc_version}"
  printf 'Observed: verified fixture CA, TLS registration 001, JOIN, LIST 322/323, channel sends, persisted DM, HISTORY, token-correlated PING/PONG, disconnect/reconnect, post-reconnect history request\n'
  printf 'Manual interactive-client observation remains a separate release record.\n'
  grep -E '(^| )(001|322|323) |PING :|PONG :' "${fixture_root}/irssi.raw.log" | tail -n 20
} > "${report_path}"
cat "${report_path}"
