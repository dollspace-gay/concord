#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
run_tests="${repository_root}/scripts/run-cargo-tests-nonempty.sh"

cd "${repository_root}/concord"
"${run_tests}" --locked -p concord-server contract::tests --lib
"${repository_root}/scripts/check-contract.sh"
node "${repository_root}/scripts/check-contract-variants.mjs"
