#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
tool=${repo_root}/ci/release_clean_host.mjs
phase_runner=${repo_root}/ci/release_clean_host_phase.mjs
workflow=${repo_root}/.github/workflows/ait-release-prepublish-clean-host.yml
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-clean-host-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-clean-host-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected clean-host test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected clean-host failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

node --check "${tool}"
node --check "${phase_runner}"
test "$(grep -c 'CHECKSUM_ASSET_NAME.test(match\[2\])' "${phase_runner}")" = 2
node --input-type=module - "${phase_runner}" <<'NODE'
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(process.argv[2], "utf8");
const declaration = source.match(/^const CHECKSUM_ASSET_NAME = \/(.*)\/;$/m);
assert.ok(declaration, "clean-host checksum asset-name rule is missing");
const rule = new RegExp(declaration[1]);
for (const name of [
  "ait-native_1.0.0~rc.11_amd64.deb",
  "ait-native-1.0.0-rc.11-x86_64.tar.gz",
  "wa120@ait+native_1.0.0.tgz",
]) {
  assert.equal(rule.test(name), true, `expected safe checksum asset name: ${name}`);
}
for (const name of [
  "",
  "/ait-native.deb",
  "../ait-native.deb",
  "assets/ait-native.deb",
  "assets\\ait-native.deb",
  "ait native.deb",
]) {
  assert.equal(rule.test(name), false, `expected unsafe checksum asset name: ${name}`);
}
NODE
test "$(grep -c 'name: Activate preinstalled Linux Homebrew' "${workflow}")" = 3
test "$(grep -c 'name: Register inbox Windows Package Manager' "${workflow}")" = 3
grep -F 'test -x /home/linuxbrew/.linuxbrew/bin/brew' "${workflow}" >/dev/null
grep -F 'Add-AppxPackage -RegisterByFamilyName -MainPackage Microsoft.DesktopAppInstaller_8wekyb3d8bbwe' \
  "${workflow}" >/dev/null
test "$(grep -Fc "\$gitBash = 'C:\Program Files\Git\bin\bash.exe'" \
  "${workflow}")" = 3
test "$(grep -Fc 'Split-Path -Parent $gitBash | Out-File -FilePath $env:GITHUB_PATH' \
  "${workflow}")" = 3
test "$(grep -c '^        id: python$' "${workflow}")" = 2
test "$(grep -Fc 'AIT_CLEAN_HOST_PYTHON: ${{ steps.python.outputs.python-path }}' \
  "${workflow}")" = 2
python_command_source=$(sed -n '/^function pythonCommand() {$/,/^}$/p' "${phase_runner}")
grep -F 'process.env.AIT_CLEAN_HOST_PYTHON' <<<"${python_command_source}" >/dev/null
grep -F 'requireExecutableFile(configured, "configured clean-host Python")' \
  <<<"${python_command_source}" >/dev/null
grep -F 'process.platform === "win32"' <<<"${python_command_source}" >/dev/null
grep -F 'explicit setup-python output' <<<"${python_command_source}" >/dev/null
grep -F 'package_manager_commands = { python: pythonCommand() }' \
  "${phase_runner}" >/dev/null
grep -F 'ait-prepublish-candidate-${{ inputs.release_id }}' "${workflow}" >/dev/null
grep -F 'ait-prepublish-clean-host-${{ inputs.release_id }}' "${workflow}" >/dev/null
grep -F 'release_prepublish_verify.mjs qualify' "${workflow}" >/dev/null
test "$(grep -c 'mark(checks, "immutable_image_digest")' "${phase_runner}")" = 2
grep -F 'platformPackageName = `@wa120/ait-native-${npmTargetSuffix(row)}`' \
  "${phase_runner}" >/dev/null
grep -F 'fail("npm uninstall retained the target platform package")' \
  "${phase_runner}" >/dev/null

matrix=${temporary_root}/matrix.json
node "${tool}" matrix \
  --family "${repo_root}/ait-release-family.json" \
  --platforms "${repo_root}/ci/native_bootstrap_matrix.json" \
  --output "${matrix}" >/dev/null
jq -e '
  .contract == "ait.release.clean-host.matrix/v1" and
  .matrix_revision == "distribution-target-32-2026-08-17.2" and
  .row_count == 32 and (.rows | length) == 32 and
  ([.rows[].id] | unique | length) == 32 and
  .counts == {
    "apt:product": 2,
    "apt:standalone": 2,
    "github:product": 6,
    "homebrew:product": 4,
    "npm:product": 6,
    "oci:standalone": 4,
    "pypi:product": 6,
    "winget:product": 2
  } and
  ([.rows[].runner] | unique | sort) == ([
    "macos-15", "macos-15-intel", "ubuntu-22.04",
    "ubuntu-22.04-arm", "windows-11-arm", "windows-2025"
  ] | sort)
' "${matrix}" >/dev/null

config=${temporary_root}/config.json
status=${temporary_root}/status.json
version=$(jq -er '.family.version' "${repo_root}/ait-release-family.json")
python_version=$(jq -er '.components[] | select(.id == "ait-python") | .version' \
  "${repo_root}/ait-release-family.json")
tag=v${version}
jq -n \
  --arg version "${version}" \
  --arg python_version "${python_version}" \
  --arg tag "${tag}" '
  {
    contract: "ait.release.family.endpoints/v1",
    release: {
      id: "REL-FAM-0123456789ABCDEF",
      version: $version,
      python_version: $python_version,
      channel: "rc",
      tag: $tag,
      source_commit: "1111111111111111111111111111111111111111"
    }
  }
' >"${config}"
jq -n \
  --arg version "${version}" \
  --arg tag "${tag}" '
  {
    contract: "ait.release.prepublish.candidate/v1",
    status: "frozen_candidate_pending_clean_host",
    release: {
      id: "REL-FAM-0123456789ABCDEF",
      version: $version,
      tag: $tag,
      source_commit: "1111111111111111111111111111111111111111"
    },
    candidate: {
      stage_receipt_sha256: "3333333333333333333333333333333333333333333333333333333333333333"
    },
    public_endpoint_writes: false
  }
' >"${status}"

export AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST=sha256:4444444444444444444444444444444444444444444444444444444444444444

config_sha=$(sha256_file "${config}")
status_sha=$(sha256_file "${status}")
release_binding=${temporary_root}/release-binding.json
jq -n \
  --arg version "${version}" \
  --arg python_version "${python_version}" \
  --arg tag "${tag}" \
  --arg config_sha "${config_sha}" \
  --arg status_sha "${status_sha}" '
  {
    id: "REL-FAM-0123456789ABCDEF",
    version: $version,
    python_version: $python_version,
    channel: "rc",
    tag: $tag,
    source_commit: "1111111111111111111111111111111111111111",
    endpoint_config_sha256: $config_sha,
    operator_status_sha256: $status_sha,
    verification_stage: "prepublication",
    candidate_stage_receipt_sha256: "3333333333333333333333333333333333333333333333333333333333333333",
    candidate_artifact_digest: "sha256:4444444444444444444444444444444444444444444444444444444444444444"
  }
' >"${release_binding}"

evidence_root=${temporary_root}/rows
mkdir "${evidence_root}"
while IFS= read -r row; do
  row_id=$(jq -er .id <<<"${row}")
  row_file=${temporary_root}/${row_id}.row.json
  printf '%s\n' "${row}" >"${row_file}"
  install=${temporary_root}/${row_id}.install.json
  upgrade=${temporary_root}/${row_id}.upgrade.json
  jq -n \
    --slurpfile release "${release_binding}" \
    --slurpfile row "${row_file}" '
    ($row[0]) as $r |
    {
      contract: "ait.release.clean-host.phase/v1",
      status: "pass",
      phase: "install",
      release: $release[0],
      row: $r,
      runner: {
        label: $r.runner,
        target_verified: true,
        github_hosted: true,
        run_id: "7001",
        run_attempt: "1",
        job: ("install-" + $r.id)
      },
      checks: (reduce $r.required_checks.install[] as $check
        ({}; .[$check] = true))
    }
  ' >"${install}"
  jq -n \
    --slurpfile release "${release_binding}" \
    --slurpfile row "${row_file}" '
    ($row[0]) as $r |
    {
      contract: "ait.release.clean-host.phase/v1",
      status: "pass",
      phase: "upgrade",
      release: $release[0],
      row: $r,
      runner: {
        label: $r.runner,
        target_verified: true,
        github_hosted: true,
        run_id: "7001",
        run_attempt: "1",
        job: ("upgrade-" + $r.id)
      },
      checks: (reduce $r.required_checks.upgrade[] as $check
        ({}; .[$check] = true))
    }
  ' >"${upgrade}"
  node "${tool}" combine \
    --matrix "${matrix}" \
    --config "${config}" \
    --status "${status}" \
    --install "${install}" \
    --upgrade "${upgrade}" \
    --output "${evidence_root}/${row_id}.json" >/dev/null
done < <(jq -c '.rows[]' "${matrix}")

aggregate=${temporary_root}/aggregate
node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${evidence_root}" \
  --output-root "${aggregate}" >/dev/null
jq -e '
  .contract == "ait.release.clean-host.aggregate/v1" and
  .status == "qualified" and
  .matrix == {admitted_rows: 32, evidence_files: 32, expected_rows: 32,
    revision: "distribution-target-32-2026-08-17.2"} and
  .failures == [] and
  .promotion == {allowed: true, retry_same_candidate: false, terminal_for_release: false}
' "${aggregate}/ait-release.clean-host-status.json" >/dev/null
test "$(wc -l <"${aggregate}/SHA256SUMS" | tr -d '[:space:]')" = 33
test "$(find "${aggregate}/rows" -type f -name '*.json' | wc -l | tr -d '[:space:]')" = 32

missing_root=${temporary_root}/missing-rows
cp -R "${evidence_root}" "${missing_root}"
missing_id=$(jq -er '.rows[0].id' "${matrix}")
rm -- "${missing_root}/${missing_id}.json"
missing_aggregate=${temporary_root}/missing-aggregate
expect_failure missing-row node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${missing_root}" \
  --output-root "${missing_aggregate}"
jq -e --arg row_id "${missing_id}" '
  .status == "blocked" and
  .promotion == {allowed: false, retry_same_candidate: true, terminal_for_release: false} and
  any(.failures[]; .row_id == $row_id and .reason == "missing_row")
' "${missing_aggregate}/ait-release.clean-host-status.json" >/dev/null

duplicate_root=${temporary_root}/duplicate-rows
cp -R "${evidence_root}" "${duplicate_root}"
cp "${duplicate_root}/${missing_id}.json" "${duplicate_root}/duplicate.json"
duplicate_aggregate=${temporary_root}/duplicate-aggregate
expect_failure duplicate-row node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${duplicate_root}" \
  --output-root "${duplicate_aggregate}"
jq -e --arg row_id "${missing_id}" '
  .status == "blocked" and
  any(.failures[]; .row_id == $row_id and .reason == "filename_mismatch") and
  any(.failures[]; .row_id == $row_id and .reason == "duplicate_row") and
  (any(.failures[]; .row_id == $row_id and .reason == "missing_row") | not)
' "${duplicate_aggregate}/ait-release.clean-host-status.json" >/dev/null

invalid_root=${temporary_root}/invalid-rows
cp -R "${evidence_root}" "${invalid_root}"
jq '.status = "fail"' "${invalid_root}/${missing_id}.json" \
  >"${invalid_root}/${missing_id}.json.new"
mv "${invalid_root}/${missing_id}.json.new" "${invalid_root}/${missing_id}.json"
invalid_aggregate=${temporary_root}/invalid-aggregate
expect_failure invalid-row node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${invalid_root}" \
  --output-root "${invalid_aggregate}"
jq -e --arg row_id "${missing_id}" '
  .status == "blocked" and
  any(.failures[]; .row_id == $row_id and .reason == "invalid_or_failed_row") and
  (any(.failures[]; .row_id == $row_id and .reason == "missing_row") | not)
' "${invalid_aggregate}/ait-release.clean-host-status.json" >/dev/null

first_install=${temporary_root}/first.install.json
first_upgrade=${temporary_root}/first.upgrade.json
first_row=$(jq -c '.rows[0]' "${matrix}")
first_id=$(jq -r .id <<<"${first_row}")
jq '.phases.install' "${evidence_root}/${first_id}.json" >"${first_install}"
jq --arg job "$(jq -er '.runner.job' "${first_install}")" \
  '.phases.upgrade | .runner.job = $job' \
  "${evidence_root}/${first_id}.json" >"${first_upgrade}"
expect_failure reused-job node "${tool}" combine \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --install "${first_install}" \
  --upgrade "${first_upgrade}" \
  --output "${temporary_root}/reused-job.json"

printf 'release clean-host contract: pass\n'
