#!/usr/bin/env bash
set -euo pipefail

# Release control plane test entrypoint. Every ci/release_*_test.sh and
# ci/release_*_test.mjs is a blocking Patchset gate for this repository.
repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "${repo_root}"
for command in bash jq lsof node tar; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done
failed=0
started=${SECONDS}
# The canonical family is an unbound template. Public control tests that read
# a bound manifest receive a fixture bound to placeholder Snapshots.
gate_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-native-gate.XXXXXX")
composition_pid=
cleanup() {
  # Never remove a worker's scratch tree before its process has returned.
  if [[ -n ${composition_pid} ]]; then wait "${composition_pid}" || true; fi
  rm -rf -- "${gate_root}"
}
trap cleanup EXIT
jq -n --arg version "$(jq -er '.family.version' ait-release-family.json)" '
  {contract: "ait.release.family-binding/v1", version: $version, tag: ("v" + $version),
   components: {"ait-core": "SNP-010101010101", "ait-server": "SNP-020202020202",
     "ait-runner": "SNP-030303030303", "ait-python": "SNP-040404040404", "ait-node": "SNP-050505050505"}}
' >"${gate_root}/binding.json"
node "${repo_root}/ci/release_family_bind.mjs" --template "${repo_root}/ait-release-family.json" \
  --binding "${gate_root}/binding.json" --output "${gate_root}/bound-family.json" >/dev/null
export AIT_RELEASE_FAMILY_MANIFEST=${gate_root}/bound-family.json
run_test() (
  trap - EXIT
  local test=$1
  shift
  local scratch result=0 test_started=${SECONDS}
  scratch=$(mktemp -d "${gate_root}/test.XXXXXX") || exit 70
  export TMPDIR=${scratch}
  printf '=== %s\n' "${test}"
  if "$@" >"${scratch}/stdout" 2>"${scratch}/stderr"; then :; else result=$?; fi
  cat "${scratch}/stdout"
  cat "${scratch}/stderr" >&2
  if [[ ${result} == 0 ]]; then
    printf '=== %s: pass (%ss)\n' "${test}" "$((SECONDS - test_started))"
  else
    printf '=== %s: FAIL (exit %s, %ss)\n' "${test}" "${result}" "$((SECONDS - test_started))" >&2
  fi
  exit "${result}"
)
# Only composition overlaps the serial lane. It copies source controls into
# its own mkdtemp tree and uses an ephemeral fixture authority port; all other
# suites retain their existing order. Do not broaden this list implicitly.
composition=ci/release_web_composition_test.mjs
run_test "${composition}" node "${composition}" &
composition_pid=$!
run_test release/monorepo/build-release.test.mjs node --test release/monorepo/build-release.test.mjs || failed=1
# release_npm_namespace_supplement_test.sh verifies the historical rc.3 npm
# namespace supplement against one exact ait-node Snapshot (SNP-22993C1FEF52).
# It is retained as evidence of that publication and is not a release gate.
for test in ci/release_*_test.sh; do
  if [[ ${test} == ci/release_npm_namespace_supplement_test.sh ]]; then
    printf '=== %s: skipped (historical rc.3 supplement pinned to one ait-node Snapshot)\n' "${test}"
    continue
  fi
  run_test "${test}" bash "${test}" || failed=1
done
for test in ci/release_*_test.mjs; do
  if [[ ${test} == "${composition}" ]]; then continue; fi
  run_test "${test}" node "${test}" || failed=1
done
if wait "${composition_pid}"; then :; else failed=1; fi
composition_pid=
printf '=== complete Native CI: %ss, exit %s\n' "$((SECONDS - started))" "${failed}"
exit "${failed}"
