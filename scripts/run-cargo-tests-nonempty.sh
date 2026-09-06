#!/usr/bin/env bash
set -euo pipefail

output="$(mktemp)"
cleanup() { rm -f "${output}"; }
trap cleanup EXIT

cargo test "$@" 2>&1 | tee "${output}"
if ! grep -Eq '^running [1-9][0-9]* tests?$' "${output}"; then
  echo "cargo test selection matched zero tests: $*" >&2
  exit 2
fi
