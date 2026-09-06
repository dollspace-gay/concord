#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
generated_directory="${repository_root}/concord/web/src/api/generated"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "${temporary_directory}"' EXIT

"${repository_root}/scripts/generate-contract.sh" "${temporary_directory}"
diff --recursive --unified "${generated_directory}" "${temporary_directory}"
