#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
install_root="$(mktemp -d)"
snapshot_root="${install_root}/source-snapshot"
build_target="${install_root}/cargo-target"
artifact_root="${install_root}/artifacts"
evidence_root="${CONCORD_SOURCE_INSTALL_EVIDENCE:-${repository_root}/.design/concord-remediation/evidence/source-install}"
evidence_dir="${evidence_root}/$(date -u +%Y%m%dT%H%M%SZ)-$$"
provenance_root="${evidence_dir}/provenance"
server_pid=""
mkdir -p "${provenance_root}"
if [[ "$(id -u)" -eq 0 ]]; then
  echo "source install smoke must run as a non-root service account" >&2
  exit 2
fi
cleanup() {
  status=$?
  if [[ -n "${server_pid}" ]]; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  if [[ ${status} -eq 0 ]]; then rm -rf "${install_root}"; else echo "source install evidence retained at ${install_root}" >&2; fi
  exit "${status}"
}
trap cleanup EXIT

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

# Capture the complete build inputs once. This is deliberately a working-tree
# snapshot: remediation qualification often runs before commit, so provenance
# must identify that state instead of attributing it to HEAD alone.
mkdir -p "${snapshot_root}"
build_inputs=(
  concord/Cargo.toml
  concord/Cargo.lock
  concord/rust-toolchain.toml
  concord/server/Cargo.toml
  concord/server/build.rs
  concord/server/src
  concord/server/migrations
  concord/web/package.json
  concord/web/package-lock.json
  concord/web/tsconfig.json
  concord/web/tsconfig.app.json
  concord/web/tsconfig.node.json
  concord/web/vite.config.ts
  concord/web/index.html
  concord/web/public
  concord/web/src
)
(
  cd "${repository_root}"
  rsync -aR "${build_inputs[@]}" "${snapshot_root}/"
)
git -C "${repository_root}" rev-parse HEAD > "${provenance_root}/git-head.txt"
git -C "${repository_root}" status --porcelain=v1 -- "${build_inputs[@]}" \
  > "${provenance_root}/git-status.txt"
if [[ -s "${provenance_root}/git-status.txt" ]]; then
  printf 'working-tree-snapshot-dirty\n' > "${provenance_root}/source-state.txt"
else
  printf 'committed-clean-tree\n' > "${provenance_root}/source-state.txt"
fi
(
  cd "${snapshot_root}"
  find . -type f -print0 | sort -z | xargs -0 sha256sum
) > "${provenance_root}/source-manifest.sha256"
cp "${provenance_root}/source-manifest.sha256" \
  "${provenance_root}/source-manifest.before-build.sha256"

cd "${snapshot_root}/concord/web"
npm ci --ignore-scripts
npm run build
cd "${snapshot_root}/concord"
CARGO_TARGET_DIR="${build_target}" \
  cargo build --workspace --release --locked --bin concord-server --bin concord_operator

# The build may create only excluded output directories. Any source/input
# mutation makes the artifact provenance invalid.
(
  cd "${snapshot_root}"
  find . -type f \
    ! -path './concord/web/node_modules/*' \
    ! -path './concord/web/dist/*' \
    -print0 | sort -z | xargs -0 sha256sum
) > "${provenance_root}/source-manifest.after-build.sha256"
cmp "${provenance_root}/source-manifest.before-build.sha256" \
  "${provenance_root}/source-manifest.after-build.sha256"

for release in install update; do
  release_root="${artifact_root}/${release}"
  install -d -m 0755 "${release_root}/bin" "${release_root}/static"
  install -m 0555 "${build_target}/release/concord-server" "${release_root}/bin/concord-server"
  install -m 0555 "${build_target}/release/concord_operator" "${release_root}/bin/concord-operator"
  cp -R "${snapshot_root}/concord/web/dist/." "${release_root}/static/"
  find "${release_root}/static" -type f -exec chmod 0444 {} +
  (
    cd "${release_root}"
    find . -type f -print0 | sort -z | xargs -0 sha256sum
  ) > "${provenance_root}/${release}-artifacts.sha256"
done
cmp "${provenance_root}/install-artifacts.sha256" \
  "${provenance_root}/update-artifacts.sha256"
cp "${provenance_root}/install-artifacts.sha256" \
  "${provenance_root}/install-artifacts.before.sha256"
cp "${provenance_root}/update-artifacts.sha256" \
  "${provenance_root}/update-artifacts.before.sha256"

install -d -m 0755 "${install_root}/bin" "${install_root}/share/concord/static" "${install_root}/etc"
install -d -m 0700 "${install_root}/var" "${install_root}/var/media" "${install_root}/var/secrets" "${install_root}/etc/tls"
install -m 0755 "${artifact_root}/install/bin/concord-server" "${install_root}/bin/concord-server"
install -m 0755 "${artifact_root}/install/bin/concord-operator" "${install_root}/bin/concord-operator"
cp -R "${artifact_root}/install/static/." "${install_root}/share/concord/static/"
"${install_root}/bin/concord-server" init --config "${install_root}/etc/concord.toml"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=localhost \
  -keyout "${install_root}/etc/tls/key.pem" -out "${install_root}/etc/tls/cert.pem" >/dev/null 2>&1
chmod 0600 "${install_root}/etc/tls/key.pem"

web_port="$(free_port)"
irc_port="$(free_port)"
python3 - "${install_root}" "${web_port}" "${irc_port}" <<'PY'
import pathlib, sys
root, web_port, irc_port = sys.argv[1:]
path = pathlib.Path(root) / "etc/concord.toml"
text = path.read_text()
replacements = {
    'web_address = "0.0.0.0:8080"': f'web_address = "127.0.0.1:{web_port}"',
    'irc_address = "127.0.0.1:6667"': f'irc_address = "127.0.0.1:{irc_port}"\nirc_tls_cert = "{root}/etc/tls/cert.pem"\nirc_tls_key = "{root}/etc/tls/key.pem"',
    'url = "sqlite:data/concord.db?mode=rwc"': f'url = "sqlite:{root}/var/concord.db?mode=rwc"',
    'jwt_secret_file = "data/secrets/jwt.key"': f'jwt_secret_file = "{root}/etc/data/secrets/jwt.key"',
    'external_credentials_key_file = "data/secrets/external-credentials.key"': f'external_credentials_key_file = "{root}/etc/data/secrets/external-credentials.key"',
    'public_url = "http://localhost:8080"': f'public_url = "http://127.0.0.1:{web_port}"',
    'data_dir = "data"': f'data_dir = "{root}/var"',
    'media_dir = "data/media"': f'media_dir = "{root}/var/media"',
}
for old, new in replacements.items():
    if old not in text:
        raise SystemExit(f"generated configuration no longer contains {old!r}")
    text = text.replace(old, new)
path.write_text(text)
PY

start_server() {
  (cd "${install_root}/share/concord" && "${install_root}/bin/concord-server" \
    --config "${install_root}/etc/concord.toml" serve) >> "${evidence_dir}/server.log" 2>&1 &
  server_pid=$!
  for _ in $(seq 1 100); do
    curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null && return
    kill -0 "${server_pid}" 2>/dev/null || { cat "${evidence_dir}/server.log" >&2; exit 1; }
    sleep 0.1
  done
  echo "installed service did not become ready" >&2
  exit 1
}

start_server
curl --fail --silent "http://127.0.0.1:${web_port}/" | grep -Fq '<div id="root"></div>'
printf '' | openssl s_client -connect "127.0.0.1:${irc_port}" -servername localhost \
  -CAfile "${install_root}/etc/tls/cert.pem" -verify_return_error >/dev/null 2>&1
kill "${server_pid}"
wait "${server_pid}"
server_pid=""
"${install_root}/bin/concord-operator" --config "${install_root}/etc/concord.toml" \
  migration-inventory > "${evidence_dir}/operator-migration-inventory.json"
python3 - "${evidence_dir}/operator-migration-inventory.json" <<'PY'
import json, sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
if not isinstance(report, dict):
    raise SystemExit("installed operator migration report is not a JSON object")
if not isinstance(report.get("source_version"), int) or report["source_version"] < 1:
    raise SystemExit("installed operator migration report lacks a valid source_version")
if not isinstance(report.get("target_version"), int) or report["target_version"] < 1:
    raise SystemExit("installed operator migration report lacks a valid target_version")
if not isinstance(report.get("findings"), list):
    raise SystemExit("installed operator migration report lacks a findings list")
if report.get("source_version") != report.get("target_version"):
    raise SystemExit("installed operator reports a pending schema migration")
if any(item.get("blocks_upgrade") for item in report.get("findings", [])):
    raise SystemExit("installed operator reports a blocking migration finding")
PY

# Exercise the documented stopped-service update: stage each artifact beside
# its destination, then rename it into the fixed runtime layout.
install -m 0755 "${artifact_root}/update/bin/concord-server" "${install_root}/bin/concord-server.next"
install -m 0755 "${artifact_root}/update/bin/concord-operator" "${install_root}/bin/concord-operator.next"
rm -rf "${install_root}/share/concord/static.next"
install -d -m 0755 "${install_root}/share/concord/static.next"
cp -R "${artifact_root}/update/static/." "${install_root}/share/concord/static.next/"
mv -f "${install_root}/bin/concord-server.next" "${install_root}/bin/concord-server"
mv -f "${install_root}/bin/concord-operator.next" "${install_root}/bin/concord-operator"
rm -rf "${install_root}/share/concord/static.previous"
mv "${install_root}/share/concord/static" "${install_root}/share/concord/static.previous"
mv "${install_root}/share/concord/static.next" "${install_root}/share/concord/static"
start_server
curl --fail --silent "http://127.0.0.1:${web_port}/health/ready" >/dev/null
(
  cd "${artifact_root}/install"
  sha256sum --check "${provenance_root}/install-artifacts.before.sha256"
)
(
  cd "${artifact_root}/update"
  sha256sum --check "${provenance_root}/update-artifacts.before.sha256"
)
printf 'source-install-smoke: PASS evidence=%s source_state=%s source_manifest_sha256=%s server_sha256=%s operator_sha256=%s protocol_sha256=%s operator_schema=%s\n' \
  "${evidence_dir}" \
  "$(tr -d '\n' < "${provenance_root}/source-state.txt")" \
  "$(sha256sum "${provenance_root}/source-manifest.sha256" | cut -d ' ' -f1)" \
  "$(sha256sum "${install_root}/bin/concord-server" | cut -d ' ' -f1)" \
  "$(sha256sum "${install_root}/bin/concord-operator" | cut -d ' ' -f1)" \
  "$(sha256sum "${install_root}/share/concord/static/protocol-version.json" | cut -d ' ' -f1)" \
  "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["source_version"])' "${evidence_dir}/operator-migration-inventory.json")"
