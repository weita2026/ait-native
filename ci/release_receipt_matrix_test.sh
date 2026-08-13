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

family=${AIT_RELEASE_FAMILY_MANIFEST:-${repo_root}/ait-release-family.json}
if [[ ${family} != /* || ! -f ${family} || -L ${family} ]]; then
  printf 'release family manifest must be an absolute regular file\n' >&2
  exit 66
fi
platforms="${repo_root}/ci/native_bootstrap_matrix.json"
authorities="${repo_root}/ci/release_repository_authorities.json"
workflow="${repo_root}/.github/workflows/ait-release-component-receipts.yml"
protected_verifier="${repo_root}/ci/release_protected_promotion.sh"
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
  'source_commit:' \
  'ref: ${{ inputs.source_commit }}' \
  'AIT_CONTROL_GIT_COMMIT: ${{ github.sha }}' \
  'workflow_control_commit: $control_commit' \
  'AIT_RELEASE_FAMILY_MANIFEST: ${{ github.workspace }}/source/ait-release-family.json' \
  'AIT_CONTROL_ROOT: ${{ github.workspace }}/control' \
  'working-directory: control' \
  'path: control' \
  'path: source' \
  'monorepo-source:' \
  '--component-receipt' \
  'public_git_commit' \
  'persist-credentials: false' \
  'name: ait-release-monorepo-source' \
  "if: matrix.repo_name == 'ait-python' && runner.os == 'Linux'" \
  'ziglang==0.15.2' \
  'python -m ziglang version' \
  'CARGO_BUILD_BUILD_DIR="${RUNNER_TEMP}/ait-family-admission-build"' \
  'input_sha256=ad5212e194db9a52b049d3334a157959102f115aeeb64f43ff0974328af2e4b3' \
  'output_sha256=6eaa298d8dacef5302d2b01bc3e204b73578f66ea69ec892901f9e2d3aa2ed72' \
  'cp -R "${AIT_PUBLIC_SOURCE_ROOT}/ait-core" "${admission_root}"' \
  'admission_rust="${admission_root}/rust"' \
  'patch --batch --forward --strip=0' \
  '<"${AIT_CONTROL_ROOT}/ci/release_family_scoped_npm.patch"' \
  '--manifest-path "${admission_rust}/Cargo.toml"' \
  'admission_root="${RUNNER_TEMP}/ait-family-admission-repository"' \
  'test ! -e "${admission_root}"' \
  'cd "${admission_root}"' \
  'release show "${release_id}"' \
  'release package "${release_id}"' \
  'release promote "${release_id}"' \
  'cp "${promotion}" "${dossier}/ait-release.promotion.json"' \
  'cp -R "dist/${release_id}/packages" "${dossier}/packages"' \
  '.authorization.granted == false' \
  '.mutation.registry_write == false' \
  'source_cache_count: 0' \
  'public_publish == false'; do
  if ! grep -F -- "${required_workflow_text}" "${workflow}" >/dev/null; then
    printf 'release workflow is missing monorepo source contract: %s\n' \
      "${required_workflow_text}" >&2
    exit 65
  fi
done
public_source_root_count=$(grep -F -c -- \
  '--public-source-root "${AIT_PUBLIC_SOURCE_ROOT}"' "${workflow}")
if test "${public_source_root_count}" -ne 6; then
  printf 'release workflow must retain one explicit public source root across candidate, check, build, show, package, and promote\n' >&2
  exit 65
fi
for forbidden_workflow_text in \
  'AIT_RELEASE_SERVER_URL' \
  'release_source_cache.sh' \
  'ait_remote_snapshot_boundary' \
  'secrets.AIT_RELEASE_SERVER_URL' \
  './ci/release_monorepo_export_test.sh' \
  './ci/release_receipt_bundle_test.sh' \
  'cp -R "${AIT_PUBLIC_SOURCE_ROOT}/ait-core/rust"' \
  '--repair-existing' \
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
  .expected_receipt_count == 31 and
  .expected_component_artifact_count == 37 and
  .source_line == "main" and
  .bootstrap_line == "release-bootstrap" and
  (.bootstrap.include | length) == 6 and
  (.sources.include | length) == 5 and
  (.builds.include | length) == 31 and
  ([.builds.include[].receipt_artifact] | unique | length) == 31 and
  ([.builds.include[] | select(.repo_name == "ait-core") |
    .expected_component_artifact_count] | all(. == 2)) and
  ([.builds.include[] | select(.target == "portable")] | length) == 1 and
  ([.builds.include[] | select(.repo_name == "ait-node")] | length) == 7 and
  ([.builds.include[].expected_component_artifact_count] | add) == 37
' "${projection}" >/dev/null

for required_verifier_text in \
  'control_root=' \
  'AIT_RELEASE_SOURCE_CONTROL_SHA' \
  'release_receipt_matrix.jq' \
  'expected_receipt_count=$(jq -er' \
  'expected_component_artifact_count=$(jq -er' \
  'expected_license_material_count=$((expected_source_count * 2))'; do
  if ! grep -F -- "${required_verifier_text}" "${protected_verifier}" >/dev/null; then
    printf 'protected promotion is not tied to the receipt projection: %s\n' \
      "${required_verifier_text}" >&2
    exit 65
  fi
done
if [[ $(grep -F -c -- '--argjson expected_receipt_count' \
    "${protected_verifier}") -ne 2 ||
  $(grep -F -c -- '--argjson expected_component_artifact_count' \
    "${protected_verifier}") -ne 2 ||
  $(grep -F -c -- '--argjson expected_license_material_count' \
    "${protected_verifier}") -ne 2 ]]; then
  printf 'protected promotion must apply projected counts to check and build records\n' >&2
  exit 65
fi
if grep -E -- 'component-artifact.*length\) == (31|37)' \
  "${protected_verifier}" >/dev/null; then
  printf 'protected promotion retains a numeric component-artifact count\n' >&2
  exit 65
fi

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
