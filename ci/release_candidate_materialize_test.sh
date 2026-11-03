#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-candidate-materialize.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-candidate-materialize.*) rm -rf -- "${temporary_root}" ;;
    *) return 1 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) triple=aarch64-apple-darwin ;;
  Darwin:x86_64) triple=x86_64-apple-darwin ;;
  Linux:aarch64 | Linux:arm64) triple=aarch64-unknown-linux-gnu ;;
  Linux:x86_64 | Linux:amd64) triple=x86_64-unknown-linux-gnu ;;
  *) printf 'release candidate materialization test skipped on unsupported host\n'; exit 0 ;;
esac
mkdir -p "${temporary_root}/cache/payload/assets"
printf 'ait fixture\n' >"${temporary_root}/cache/payload/assets/ait-1.2.0-${triple}"
printf 'server fixture\n' >"${temporary_root}/cache/payload/assets/ait-server-1.2.0-${triple}"

"${repo_root}/ci/release_candidate_materialize.sh" materialize 1.2.0 \
  "${temporary_root}/cache" "${temporary_root}/candidate" >/dev/null
"${repo_root}/ci/release_candidate_materialize.sh" probe 1.2.0 \
  "${temporary_root}/cache" "${temporary_root}/candidate"
cmp "${temporary_root}/cache/payload/assets/ait-1.2.0-${triple}" "${temporary_root}/candidate/ait"
cmp "${temporary_root}/cache/payload/assets/ait-server-1.2.0-${triple}" "${temporary_root}/candidate/ait-server"
jq -e '.contract == "ait.release.candidate-materialization/v1" and .status == "complete"' \
  "${temporary_root}/candidate/receipt.json" >/dev/null

printf 'release candidate materialization tests passed\n'
