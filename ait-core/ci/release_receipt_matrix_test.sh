#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-matrix-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-matrix-test.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected temporary path: %s\n' "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

project() {
  local family_path=$1
  local platform_path=$2
  local authority_path=$3
  jq -n \
    --slurpfile family "${family_path}" \
    --slurpfile platforms "${platform_path}" \
    --slurpfile authorities "${authority_path}" \
    -f "${repo_root}/ci/release_receipt_matrix.jq"
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" 2>"${temporary_root}/${label}.stderr"; then
    printf 'expected matrix projection failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

family="${repo_root}/ait-release-family.json"
platforms="${repo_root}/ci/native_bootstrap_matrix.json"
authorities="${repo_root}/ci/release_repository_authorities.json"
workflow="${repo_root}/.github/workflows/ait-release-component-receipts.yml"
projection="${temporary_root}/projection.json"

if awk '
  function leading_spaces(line) {
    match(line, /[^ ]/)
    return RSTART - 1
  }
  /^[ ]*run:[ ]*\|[ ]*$/ {
    in_run = 1
    run_indent = leading_spaces($0)
    next
  }
  in_run && $0 !~ /^[ ]*$/ {
    current_indent = leading_spaces($0)
    if (current_indent <= run_indent) {
      in_run = 0
    } else if (index($0, "${{") != 0) {
      found = 1
    }
  }
  END { exit(found ? 0 : 1) }
' "${workflow}"; then
  printf 'workflow run blocks must receive GitHub expressions through env\n' >&2
  exit 65
fi

for required_workflow_text in \
  './ci/release_monorepo_export_test.sh' \
  'monorepo-source:' \
  '--component-receipt' \
  'public_git_commit' \
  'persist-credentials: false' \
  'name: ait-release-monorepo-source' \
  "if: matrix.repo_name == 'ait-python' && runner.os == 'Linux'" \
  'ziglang==0.15.2' \
  'python -m ziglang version' \
  'source_cache_count: 0' \
  'public_publish == false'; do
  if ! grep -F -- "${required_workflow_text}" "${workflow}" >/dev/null; then
    printf 'release workflow is missing monorepo source contract: %s\n' \
      "${required_workflow_text}" >&2
    exit 65
  fi
done
for forbidden_workflow_text in \
  'AIT_RELEASE_SERVER_URL' \
  'release_source_cache.sh' \
  'ait_remote_snapshot_boundary' \
  'secrets.AIT_RELEASE_SERVER_URL' \
  'pattern: ait-release-source-ait-*'; do
  if grep -F -- "${forbidden_workflow_text}" "${workflow}" >/dev/null; then
    printf 'release workflow retains live-server source hydration: %s\n' \
      "${forbidden_workflow_text}" >&2
    exit 65
  fi
done
if grep -F 'contents: write' "${workflow}" >/dev/null; then
  printf 'internal release workflow must not gain GitHub contents write authority\n' >&2
  exit 65
fi

project "${family}" "${platforms}" "${authorities}" >"${projection}"
jq -e '
  .contract == "ait.release.receipt-matrix/v1" and
  .public_publish == false and
  .expected_source_count == 5 and
  .expected_receipt_count == 25 and
  .expected_component_artifact_count == 31 and
  .source_line == "main" and
  .bootstrap_line == "release-bootstrap" and
  (.bootstrap.include | length) == 6 and
  (.sources.include | length) == 5 and
  (.builds.include | length) == 25 and
  ([.builds.include[].receipt_artifact] | unique | length) == 25 and
  ([.builds.include[] | select(.repo_name == "ait-core") |
    .expected_component_artifact_count] | all(. == 2)) and
  ([.builds.include[] | select(.target == "portable")] | length) == 1 and
  ([.builds.include[].expected_component_artifact_count] | add) == 31
' "${projection}" >/dev/null

jq '.schema = "ait.release.family/v2"' "${family}" \
  >"${temporary_root}/legacy-family.json"
expect_failure legacy-family project \
  "${temporary_root}/legacy-family.json" "${platforms}" "${authorities}"

jq '.public_source.subtrees |= map(select(.source_repository != "ait-node"))' \
  "${family}" >"${temporary_root}/missing-subtree.json"
expect_failure missing-subtree project \
  "${temporary_root}/missing-subtree.json" "${platforms}" "${authorities}"

jq '.distributions += [.distributions[] | select(.channel == "github")]' \
  "${family}" >"${temporary_root}/duplicate-github.json"
expect_failure duplicate-github project \
  "${temporary_root}/duplicate-github.json" "${platforms}" "${authorities}"

jq '.public_source.transforms[0].to = "../../ait-core"' "${family}" \
  >"${temporary_root}/undeclared-transform.json"
expect_failure undeclared-transform project \
  "${temporary_root}/undeclared-transform.json" "${platforms}" "${authorities}"

jq '.repositories[1].repository_index = 0' "${authorities}" \
  >"${temporary_root}/duplicate-index.json"
expect_failure duplicate-index project \
  "${family}" "${platforms}" "${temporary_root}/duplicate-index.json"

jq '.public_publish = true' "${authorities}" \
  >"${temporary_root}/public-publish.json"
expect_failure public-publish project \
  "${family}" "${platforms}" "${temporary_root}/public-publish.json"

jq '(.components[] | select(.id == "ait-agent") | .source_snapshot) =
  "SNP-000000000000"' "${family}" >"${temporary_root}/snapshot-drift.json"
expect_failure snapshot-drift project \
  "${temporary_root}/snapshot-drift.json" "${platforms}" "${authorities}"

jq '(.components[] | select(.id == "ait-node") | .source_repository) =
  "ait-napi"' "${family}" >"${temporary_root}/unknown-repository.json"
expect_failure unknown-repository project \
  "${temporary_root}/unknown-repository.json" "${platforms}" "${authorities}"

jq '.targets = .targets[0:5]' "${platforms}" \
  >"${temporary_root}/platform-drift.json"
expect_failure platform-drift project \
  "${family}" "${temporary_root}/platform-drift.json" "${authorities}"

jq '.portable_runner.os = "plan9"' "${authorities}" \
  >"${temporary_root}/portable-runner-drift.json"
expect_failure portable-runner-drift project \
  "${family}" "${platforms}" "${temporary_root}/portable-runner-drift.json"

printf 'release receipt matrix contract: pass\n'
