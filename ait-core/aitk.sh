#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/ait.sh"

PROFILE_DIR="$(cargo_profile_dir)"
ARTIFACT_DIR="$(cargo_target_dir)/${PROFILE_DIR}"
AITK_ARTIFACT="${ARTIFACT_DIR}/aitk"
if [[ ! -x "${AITK_ARTIFACT}" && -x "${AITK_ARTIFACT}.exe" ]]; then
  AITK_ARTIFACT="${AITK_ARTIFACT}.exe"
fi
if [[ ! -x "${AITK_ARTIFACT}" ]]; then
  printf 'aitk.sh: native artifact is missing: %s\n' "${AITK_ARTIFACT}" >&2
  printf 'Build it with: ./ait.sh core build\n' >&2
  exit 1
fi

exec "${AITK_ARTIFACT}" "$@"
