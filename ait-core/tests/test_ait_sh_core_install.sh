#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${AIT_SHARED_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-${ROOT_DIR}/.ait/cargo-target}}"
PROFILE="${AIT_CORE_BUILD_PROFILE:-release}"
ARTIFACT_DIR="${TARGET_DIR}/${PROFILE}"
TEMP_ROOT="$(mktemp -d)"
trap 'rm -rf -- "${TEMP_ROOT}"' EXIT

artifact_path() {
  local name="$1"
  if [[ -f "${ARTIFACT_DIR}/${name}" ]]; then
    printf '%s\n' "${ARTIFACT_DIR}/${name}"
  else
    printf '%s\n' "${ARTIFACT_DIR}/${name}.exe"
  fi
}

"${ROOT_DIR}/ait.sh" core install --bin-dir "${TEMP_ROOT}/bin" --skip-build

cmp "$(artifact_path ait-cli)" "${TEMP_ROOT}/bin/ait"
cmp "$(artifact_path ait-agent)" "${TEMP_ROOT}/bin/ait-agent"
cmp "$(artifact_path ait-agent-worker)" "${TEMP_ROOT}/bin/ait-agent-worker"
test -x "${TEMP_ROOT}/bin/ait"
test -x "${TEMP_ROOT}/bin/ait-agent"
test -x "${TEMP_ROOT}/bin/ait-agent-worker"
"${TEMP_ROOT}/bin/ait" --version | grep -Eq '^ait [0-9]+\.[0-9]+\.[0-9]+$'

if "${ROOT_DIR}/ait.sh" core install --unknown-option >/dev/null 2>&1; then
  printf 'Unknown install options must fail closed.\n' >&2
  exit 1
fi
