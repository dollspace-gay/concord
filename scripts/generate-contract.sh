#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_directory="${1:-${repository_root}/concord/web/src/api/generated}"
mkdir -p "$(dirname "${output_directory}")"
temporary_directory="$(mktemp -d "${output_directory}.tmp.XXXXXX")"
trap 'rm -rf "${temporary_directory}"' EXIT

mkdir -p "${output_directory}"
cd "${repository_root}/concord"
cargo run --quiet --locked --manifest-path "${repository_root}/concord/Cargo.toml" \
  --bin generate_contract -- "${temporary_directory}/contract.schema.json"
node "${repository_root}/scripts/generate-contract.mjs" \
  "${temporary_directory}/contract.schema.json" "${temporary_directory}"
for artifact in contract.schema.json contract.ts validator.ts; do
  mv "${temporary_directory}/${artifact}" "${output_directory}/${artifact}"
done
