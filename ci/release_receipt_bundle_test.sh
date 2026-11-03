#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-bundle-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-bundle-test.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected temporary path: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

sha256_file() {
  local file=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
  else
    shasum -a 256 "${file}" | awk '{print $1}'
  fi
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected receipt bundle failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

source_root="${temporary_root}/source"
artifact_relative='dist/REL-GEN-TEST/components/ait/.ait/release/ait-cli'
artifact="${source_root}/${artifact_relative}"
mkdir -p "$(dirname -- "${artifact}")"
printf 'exact component bytes\n' >"${artifact}"
artifact_sha256=$(sha256_file "${artifact}")
artifact_size=$(wc -c <"${artifact}" | tr -d '[:space:]')
receipt="${temporary_root}/receipt.json"

jq -n \
  --arg path "${artifact_relative}" \
  --arg sha256 "${artifact_sha256}" \
  --argjson size_bytes "${artifact_size}" '
  {
    contract: "ait.release.adapter.receipt/v1",
    status: "built",
    authority: {
      source: "selected_snapshot",
      local_release_authority: "not_activated",
      remote_publish_supported: false
    },
    check_summary: {decision: "pass"},
    repo_name: "ait-core",
    snapshot_id: "SNP-0123456789AB",
    version: "1.0.0-rc.1",
    target: "aarch64-apple-darwin",
    artifacts: [
      {
        role: "component-artifact",
        path: $path,
        sha256: $sha256,
        size_bytes: $size_bytes,
        target: "aarch64-apple-darwin"
      }
    ]
  }
' >"${receipt}"

bundle="${temporary_root}/bundle"
"${repo_root}/ci/release_receipt_bundle.sh" \
  "${source_root}" \
  "${receipt}" \
  "${bundle}" \
  ait-core \
  SNP-0123456789AB \
  1.0.0-rc.1 \
  aarch64-apple-darwin \
  1 >/dev/null

test -f "${bundle}/${artifact_relative}"
cmp "${artifact}" "${bundle}/${artifact_relative}"
jq -e '
  .contract == "ait.release.component-ci-evidence/v1" and
  .status == "pass" and
  .component_artifact_count == 1 and
  .recorded_artifact_count == 1 and
  .registry_publish == false and
  .public_publish == false
' "${bundle}/ci-run.evidence.json" >/dev/null

jq '.artifacts[0].path = "../outside"' "${receipt}" \
  >"${temporary_root}/traversal.json"
expect_failure traversal "${repo_root}/ci/release_receipt_bundle.sh" \
  "${source_root}" "${temporary_root}/traversal.json" \
  "${temporary_root}/traversal-output" ait-core SNP-0123456789AB \
  1.0.0-rc.1 aarch64-apple-darwin 1

jq '.artifacts += [.artifacts[0]]' "${receipt}" \
  >"${temporary_root}/duplicate.json"
expect_failure duplicate "${repo_root}/ci/release_receipt_bundle.sh" \
  "${source_root}" "${temporary_root}/duplicate.json" \
  "${temporary_root}/duplicate-output" ait-core SNP-0123456789AB \
  1.0.0-rc.1 aarch64-apple-darwin 2

jq '.artifacts[0].sha256 = ("0" * 64)' "${receipt}" \
  >"${temporary_root}/wrong-sha.json"
expect_failure wrong-sha "${repo_root}/ci/release_receipt_bundle.sh" \
  "${source_root}" "${temporary_root}/wrong-sha.json" \
  "${temporary_root}/wrong-sha-output" ait-core SNP-0123456789AB \
  1.0.0-rc.1 aarch64-apple-darwin 1

printf 'release receipt bundle contract: pass\n'
