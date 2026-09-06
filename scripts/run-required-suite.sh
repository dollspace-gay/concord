#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
suite="${1:-}"
runner="${repository_root}/scripts/suites/${suite}"

case "${suite}" in
  application-policy|browser-socket|deterministic-jobs|migration-fixtures|storage-faults|packaging-restore|load-recovery|container-smoke) ;;
  *)
    echo "unknown required suite: ${suite:-<missing>}" >&2
    exit 64
    ;;
esac

if [[ ! -x "${runner}" ]]; then
  echo "required ${suite} suite has not been implemented; qualification cannot pass" >&2
  exit 2
fi

exec "${runner}"
