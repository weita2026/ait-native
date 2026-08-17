#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(bash -c 'source "$1"; cargo_target_dir' _ "${ROOT_DIR}/ait.sh")"
PROFILE="${AIT_CORE_BUILD_PROFILE:-release}"
ARTIFACT_DIR="${TARGET_DIR}/${PROFILE}"
TEMP_ROOT="$(mktemp -d)"
trap 'rm -rf -- "${TEMP_ROOT}"' EXIT

mkdir -p "${TEMP_ROOT}/bin"
printf 'obsolete native command\n' >"${TEMP_ROOT}/bin/ait-agent"
printf 'obsolete Windows command\n' >"${TEMP_ROOT}/bin/ait-agent.exe"
chmod +x "${TEMP_ROOT}/bin/ait-agent" "${TEMP_ROOT}/bin/ait-agent.exe"

artifact_path() {
  local name="$1"
  if [[ -f "${ARTIFACT_DIR}/${name}" ]]; then
    printf '%s\n' "${ARTIFACT_DIR}/${name}"
  else
    printf '%s\n' "${ARTIFACT_DIR}/${name}.exe"
  fi
}

"${ROOT_DIR}/ait.sh" core install --bin-dir "${TEMP_ROOT}/bin" --skip-build

test ! -e "${TEMP_ROOT}/bin/ait-agent"
test ! -e "${TEMP_ROOT}/bin/ait-agent.exe"
cmp "$(artifact_path ait-cli)" "${TEMP_ROOT}/bin/ait"
cmp "$(artifact_path ait-agent-worker)" "${TEMP_ROOT}/bin/ait-agent-worker"
test -x "${TEMP_ROOT}/bin/ait"
test -x "${TEMP_ROOT}/bin/ait-agent-worker"
"${TEMP_ROOT}/bin/ait" --version |
  grep -Eq '^ait [0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)*$'

if "${ROOT_DIR}/ait.sh" core install --unknown-option >/dev/null 2>&1; then
  printf 'Unknown install options must fail closed.\n' >&2
  exit 1
fi

mkdir -p "${TEMP_ROOT}/protected-bin/ait-agent"
printf 'must remain\n' >"${TEMP_ROOT}/protected-bin/ait-agent/marker"
if "${ROOT_DIR}/ait.sh" core install \
  --bin-dir "${TEMP_ROOT}/protected-bin" --skip-build >/dev/null 2>&1; then
  printf 'A non-file retired command path must fail closed.\n' >&2
  exit 1
fi
test -f "${TEMP_ROOT}/protected-bin/ait-agent/marker"

mkdir -p "${TEMP_ROOT}/artifact-dir"
printf 'obsolete build artifact\n' >"${TEMP_ROOT}/artifact-dir/ait-agent"
printf 'obsolete Windows build artifact\n' >"${TEMP_ROOT}/artifact-dir/ait-agent.exe"
bash -c 'source "$1"; remove_retired_agent_artifacts "$2"' \
  _ "${ROOT_DIR}/ait.sh" "${TEMP_ROOT}/artifact-dir" >/dev/null
test ! -e "${TEMP_ROOT}/artifact-dir/ait-agent"
test ! -e "${TEMP_ROOT}/artifact-dir/ait-agent.exe"
