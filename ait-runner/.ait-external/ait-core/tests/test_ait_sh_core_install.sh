#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(bash -c 'source "$1"; cargo_target_dir' _ "${ROOT_DIR}/ait.sh")"
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
cmp "$(artifact_path aitk)" "${TEMP_ROOT}/bin/aitk"
test -x "${TEMP_ROOT}/bin/ait"
test -x "${TEMP_ROOT}/bin/ait-agent"
test -x "${TEMP_ROOT}/bin/ait-agent-worker"
test -x "${TEMP_ROOT}/bin/aitk"
SEMVER_PATTERN='[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?'
"${TEMP_ROOT}/bin/ait" --version | grep -Eq "^ait ${SEMVER_PATTERN}$"
"${TEMP_ROOT}/bin/ait-agent" --version | grep -Eq "^ait-agent ${SEMVER_PATTERN}$"
"${TEMP_ROOT}/bin/ait-agent-worker" --version | grep -Eq "^ait-agent-worker ${SEMVER_PATTERN}$"
"${TEMP_ROOT}/bin/aitk" --version | grep -Eq "^aitk ${SEMVER_PATTERN}$"

if "${ROOT_DIR}/ait.sh" core install --unknown-option >/dev/null 2>&1; then
  printf 'Unknown install options must fail closed.\n' >&2
  exit 1
fi
