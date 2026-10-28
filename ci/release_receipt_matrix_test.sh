#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
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
  'pre_tag_admission_sha256:' \
  'pre_tag_admission_b64:' \
  'actions: read' \
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
  'ait.release.operator.pre-tag-admission/v1' \
  'ci/release_pre_rc_delta.mjs' \
  '.name == ("AIT pre-RC qualification / " + $commit)' \
  'qualified repair commit gained a release tag after qualification' \
  'ait-release.pre-tag-admission.json' \
  'persist-credentials: false' \
  'name: ait-release-monorepo-source' \
  "if: matrix.repo_name == 'ait-python' && runner.os == 'Linux'" \
  'ziglang==0.15.2' \
  'python -m ziglang version' \
  'CARGO_BUILD_BUILD_DIR="${RUNNER_TEMP}/ait-family-admission-build"' \
  'admission_rust="${AIT_PUBLIC_SOURCE_ROOT}/ait-core/rust"' \
  'test -f "${admission_rust}/Cargo.lock"' \
  'test -f "${admission_rust}/crates/ait-cli/src/release_surface/family_packages.rs"' \
  'test -f "${admission_rust}/crates/ait-cli/src/release_surface/family_release.rs"' \
  '--manifest-path "${admission_rust}/Cargo.toml"' \
  'admission_parent="${RUNNER_TEMP}/ait-family-admission-repository"' \
  'admission_root="${admission_parent}/ait-core"' \
  'test ! -e "${admission_parent}"' \
  'mkdir -p "${admission_root}"' \
  'cd "${admission_root}"' \
  '"${AIT_FAMILY_ADMISSION_BIN}" init --json' \
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

# Execute the exact jq filters embedded in the hosted admission step. A text
# search alone cannot prove that GitHub's run and artifact records bind.
extract_workflow_filter() {
  local start=$1
  local end=$2
  local output=$3
  awk -v start="${start}" -v end="${end}" '
    index($0, start) { found = 1; capture = 1; next }
    capture && index($0, end) { closed = 1; exit }
    capture { print }
    END { if (!found || !closed) exit 1 }
  ' "${workflow}" >"${output}"
  test -s "${output}"
}

run_filter=${temporary_root}/qualification-run-filter.jq
artifact_filter=${temporary_root}/qualification-artifact-filter.jq
extract_workflow_filter '--arg control "${qualification_control_commit}"' \
  '"${run_record}" >/dev/null' "${run_filter}"
extract_workflow_filter '--arg digest "${qualification_artifact_digest}"' \
  '"${artifact_record}" >/dev/null' "${artifact_filter}"

qualified_commit=aad49fe7d848ca0ccc82afb4132b78a2757bcf98
other_commit=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
qualification_run_id=40741983291
qualification_artifact_id=12840776329
qualification_artifact_digest=sha256:dbe02e011e364a3105fd6874a6731be262a64f02878b38cd5572d055153e5a43
qualification_run=${temporary_root}/qualification-run.json
qualification_artifact=${temporary_root}/qualification-artifact.json
jq -S -n --arg commit "${qualified_commit}" \
  --arg name "AIT pre-RC qualification / ${qualified_commit}" \
  --argjson id "${qualification_run_id}" '
    {
      id: $id, run_attempt: 1, name: $name, display_title: $name,
      path: ".github/workflows/ait-release-pre-rc-qualification.yml",
      event: "workflow_dispatch", status: "completed", conclusion: "success",
      head_sha: $commit
    }
  ' >"${qualification_run}"
jq -S -n --arg name "ait-pre-rc-qualification-${qualified_commit}" \
  --arg digest "${qualification_artifact_digest}" \
  --argjson id "${qualification_artifact_id}" \
  --argjson run_id "${qualification_run_id}" '
    {
      id: $id, name: $name, digest: $digest, expired: false,
      workflow_run: {id: $run_id}
    }
  ' >"${qualification_artifact}"

matches_qualification_run() {
  local record=$1
  local control=${2:-${qualified_commit}}
  jq -e --argjson id "${qualification_run_id}" --argjson attempt 1 \
    --arg commit "${qualified_commit}" --arg control "${control}" \
    -f "${run_filter}" "${record}" >/dev/null
}

matches_qualification_artifact() {
  local record=$1
  jq -e --argjson id "${qualification_artifact_id}" \
    --argjson run_id "${qualification_run_id}" \
    --arg name "ait-pre-rc-qualification-${qualified_commit}" \
    --arg digest "${qualification_artifact_digest}" \
    -f "${artifact_filter}" "${record}" >/dev/null
}

matches_qualification_run "${qualification_run}"
matches_qualification_artifact "${qualification_artifact}"

expect_run_rejection() {
  local label=$1
  local filter=$2
  local record=${temporary_root}/${label}-run.json
  jq --arg commit "${qualified_commit}" --arg other "${other_commit}" \
    "${filter}" "${qualification_run}" >"${record}"
  if matches_qualification_run "${record}" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'hosted qualification run accepted %s\n' "${label}" >&2
    return 1
  fi
  test ! -s "${temporary_root}/${label}.stderr"
}

expect_run_rejection old-static-name '.name = "ait pre-RC qualification"'
expect_run_rejection retained-source-qa \
  '.name = ("Native retained-source QA / " + $commit)'
expect_run_rejection wrong-name-commit \
  '.name = ("AIT pre-RC qualification / " + $other)'
expect_run_rejection wrong-workflow-path '.path = ".github/workflows/other.yml"'
expect_run_rejection wrong-run-id '.id = 1'
expect_run_rejection wrong-run-attempt '.run_attempt = 2'
expect_run_rejection wrong-run-event '.event = "push"'
expect_run_rejection incomplete-run '.status = "in_progress"'
expect_run_rejection failed-run '.conclusion = "failure"'
expect_run_rejection wrong-source-commit '.head_sha = $other'
if matches_qualification_run "${qualification_run}" "${other_commit}" \
  2>"${temporary_root}/wrong-control-commit.stderr"; then
  printf 'hosted qualification run accepted a wrong control commit\n' >&2
  exit 1
fi
test ! -s "${temporary_root}/wrong-control-commit.stderr"

expect_artifact_rejection() {
  local label=$1
  local filter=$2
  local record=${temporary_root}/${label}-artifact.json
  jq "${filter}" "${qualification_artifact}" >"${record}"
  if matches_qualification_artifact "${record}" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'hosted qualification artifact accepted %s\n' "${label}" >&2
    return 1
  fi
  test ! -s "${temporary_root}/${label}.stderr"
}

expect_artifact_rejection wrong-artifact-id '.id = 1'
expect_artifact_rejection wrong-artifact-run '.workflow_run.id = 1'
expect_artifact_rejection wrong-artifact-name '.name = "other"'
expect_artifact_rejection wrong-artifact-digest '.digest = "sha256:bad"'
expect_artifact_rejection expired-artifact '.expired = true'

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
  'cp -R "${AIT_PUBLIC_SOURCE_ROOT}/ait-core"' \
  'release_family_rc4_admission.patch' \
  'patch --batch --forward' \
  'admission_root="${RUNNER_TEMP}/ait-family-admission-repository"' \
  'init --name' \
  '--default-line' \
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
  .expected_component_artifact_count == 31 and
  .source_line == "main" and
  .bootstrap_line == "release-bootstrap" and
  (.bootstrap.include | length) == 6 and
  (.sources.include | length) == 5 and
  (.builds.include | length) == 31 and
  ([.builds.include[].receipt_artifact] | unique | length) == 31 and
  ([.builds.include[] | select(.repo_name == "ait-core") |
    .expected_component_artifact_count] | all(. == 1)) and
  ([.builds.include[] | select(.repo_name == "ait-core") |
    .expected_components] |
    all(. == ["ait"])) and
  ([.builds.include[] | select(.target == "portable")] | length) == 1 and
  ([.builds.include[] | select(.repo_name == "ait-node")] | length) == 7 and
  ([.builds.include[].expected_component_artifact_count] | add) == 31
' "${projection}" >/dev/null

for required_verifier_text in \
  'control_root=' \
  'AIT_RELEASE_SOURCE_CONTROL_SHA' \
  'release_receipt_matrix.jq' \
  'expected_receipt_count=$(jq -er' \
  'expected_component_artifact_count=$(jq -er' \
  'expected_license_material_count=$((expected_source_count * 2))' \
  'admission_rust=${public_source_root}/ait-core/rust' \
  'cargo_lock_sha256=$(sha256_file "${admission_cargo_lock}")' \
  'family_packages_sha256=$(sha256_file "${family_packages}")' \
  'family_release_sha256=$(sha256_file "${family_release}")' \
  'native admission build mutated the immutable tagged source' \
  'github_prerelease=true' \
  'github_prerelease=false' \
  '.routes.github == {draft: false, prerelease: $github_prerelease, tag: $tag}' \
  'immutable-tag-native-admission/v1'; do
  if ! grep -F -- "${required_verifier_text}" "${protected_verifier}" >/dev/null; then
    printf 'protected promotion is not tied to the receipt projection: %s\n' \
      "${required_verifier_text}" >&2
    exit 65
  fi
done
for retired_verifier_text in \
  release_family_rc4_admission.patch \
  immutable-tag-plus-hash-pinned-control-patch/v1 \
  'patch --batch --forward' \
  family_packages_input_sha256 \
  family_packages_output_sha256; do
  if grep -F -- "${retired_verifier_text}" "${protected_verifier}" >/dev/null; then
    printf 'protected promotion retains RC.4 source rewriting: %s\n' \
      "${retired_verifier_text}" >&2
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

jq '(.components[] | select(.id == "ait") | .source_snapshot) =
  "unbound"' "${family}" >"${temporary_root}/snapshot-drift.json"
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
