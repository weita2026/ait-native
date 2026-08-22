#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
workflow=${repo_root}/.github/workflows/ait-release-pre-rc-qualification.yml
delta=${repo_root}/ci/release_pre_rc_delta.mjs
test_tmp_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd -P)
temporary_root=$(mktemp -d "${test_tmp_parent}/ait-pre-rc-qualification-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${test_tmp_parent}"/ait-pre-rc-qualification-test.*)
      rm -rf -- "${temporary_root}"
      ;;
    *) printf 'refusing to remove unexpected pre-RC test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected pre-RC qualification failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

test -f "${workflow}"
test -x "${delta}"
node --check "${delta}"

for required in \
  'name: ait pre-RC qualification' \
  'Require an exact untagged repair commit' \
  "git tag --points-at HEAD | grep -Eq '^v[0-9]'" \
  'runner: windows-11-arm' \
  'target: aarch64-pc-windows-msvc' \
  'runner: windows-2025' \
  'target: x86_64-pc-windows-msvc' \
  'init_cli_creates_then_reinitializes_the_agent_contract' \
  'init_is_idempotent_and_creates_only_an_empty_runtime_root' \
  'installed_run_initializes_then_serves_from_an_explicit_root' \
  'bash ait-core/ci/release_monorepo_export_test.sh --public-layout-selftest' \
  'root_command_inventory_is_frozen' \
  'root_ait_agent_namespace_is_absent' \
  'ait.release.pre-rc-qualification/v1' \
  'immutable_release_tag_created: false' \
  'release_receipts_created: false' \
  'public_endpoint_writes: false' \
  'ait-pre-rc-qualification-${{ inputs.source_commit }}'; do
  grep -F -- "${required}" "${workflow}" >/dev/null
done

for forbidden in \
  'contents: write' \
  'packages: write' \
  'id-token: write' \
  'npm publish' \
  'twine upload' \
  'gh release create' \
  'docker push' \
  'bash ci/release_monorepo_export_test.sh' \
  'git tag -a'; do
  if grep -F -- "${forbidden}" "${workflow}" >/dev/null; then
    printf 'pre-RC qualification gained release authority: %s\n' "${forbidden}" >&2
    exit 65
  fi
done

fixture=${temporary_root}/fixture
mkdir -p "${fixture}/ci" "${fixture}/component"
git -C "${fixture}" init -q
git -C "${fixture}" config user.name 'AIT pre-RC test'
git -C "${fixture}" config user.email 'pre-rc@localhost'

write_family() {
  local version=$1
  local python_version=$2
  local core_snapshot=$3
  local authority_snapshot=${4:-${core_snapshot}}
  local escaped_version=${version//./\\.}
  jq -S -n --arg version "${version}" --arg python "${python_version}" '
    {
      schema: "ait.release.family/v3",
      family: {
        name: "ait-native",
        version: $version,
        channel: "rc",
        tag: ("v" + $version)
      },
      components: [{id: "ait-python", version: $python}]
    }
  ' >"${fixture}/ait-release-family.json"
  jq -S -n --arg version "${version}" --arg core "${core_snapshot}" '
    {
      schema: "ait.release.monorepo-source/v1",
      family_version: $version,
      family_tag: ("v" + $version),
      subtrees: [
        {source_repository: "ait-core", source_snapshot: $core},
        {source_repository: "ait-node", source_snapshot: "SNP-222222222222"},
        {source_repository: "ait-python", source_snapshot: "SNP-333333333333"},
        {source_repository: "ait-runner", source_snapshot: "SNP-444444444444"},
        {source_repository: "ait-server", source_snapshot: "SNP-555555555555"}
      ]
    }
  ' \
    >"${fixture}/ait-monorepo-source.json"
  jq -S -n --arg version "${version}" \
    '{version: $version, public_publish: false}' \
    >"${fixture}/ci/native_bootstrap_matrix.json"
  jq -S -n --arg version "${version}" \
    '{family_version: $version, public_publish: false}' \
    >"${fixture}/ci/release_repository_authorities.json"
  printf '%s\n' "${version}" >"${fixture}/component/version.txt"
  printf 'version=%s\nregex=%s\nsnapshot=%s\n' \
    "${version}" "${escaped_version}" "${authority_snapshot}" \
    >"${fixture}/component/authority.txt"
}

write_family 1.2.3-rc.4 1.2.3rc4 SNP-111111111111
printf '\0qualified\n' >"${fixture}/component/blob.bin"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'qualified repair'
qualified_commit=$(git -C "${fixture}" rev-parse HEAD)

write_family 1.2.3-rc.5 1.2.3rc5 SNP-AAAAAAAAAAAA
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'version-only release'
release_commit=$(git -C "${fixture}" rev-parse HEAD)

node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${release_commit}" >"${temporary_root}/delta.json"
jq -e \
  --arg qualified "${qualified_commit}" \
  --arg release "${release_commit}" '
    .contract == "ait.release.pre-rc-delta/v1" and
    .decision == "pass" and
    .qualified_commit == $qualified and
    .release_commit == $release and
    .qualified_version == "1.2.3-rc.4" and
    .release_version == "1.2.3-rc.5" and
    .authority_snapshot_transitions == [{
      source_repository: "ait-core",
      qualified_snapshot: "SNP-111111111111",
      release_snapshot: "SNP-AAAAAAAAAAAA"
    }] and
    .normalized_version_paths == [
      "component/authority.txt",
      "component/version.txt"
    ]
  ' "${temporary_root}/delta.json" >/dev/null

git -C "${fixture}" checkout -q --detach "${qualified_commit}"
write_family 1.2.3-rc.5 1.2.3rc5 SNP-AAAAAAAAAAAA
printf 'behavior=changed\n' >>"${fixture}/component/authority.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'forbidden release behavior'
non_version_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure non-version node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${non_version_commit}"
grep -F 'release delta contains non-version changes' \
  "${temporary_root}/non-version.stderr" >/dev/null

printf 'second generation\n' >>"${fixture}/component/authority.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'non-direct forbidden release behavior'
non_direct_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure non-direct node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${non_direct_commit}"
grep -F 'release commit must be the single direct child' \
  "${temporary_root}/non-direct.stderr" >/dev/null

git -C "${fixture}" checkout -q --detach "${qualified_commit}"
write_family 1.2.3-rc.5 1.2.3rc5 \
  SNP-AAAAAAAAAAAA SNP-FFFFFFFFFFFF
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'unbound Snapshot release behavior'
unbound_snapshot_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure unbound-snapshot node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${unbound_snapshot_commit}"
grep -F 'release delta contains non-version changes' \
  "${temporary_root}/unbound-snapshot.stderr" >/dev/null

git -C "${fixture}" checkout -q --detach "${qualified_commit}"
write_family 1.2.3-rc.5 1.2.3rc5 SNP-AAAAAAAAAAAA
printf 'added path\n' >"${fixture}/component/added.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'added non-authority release path'
added_path_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure added-path node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${added_path_commit}"

git -C "${fixture}" checkout -q --detach "${qualified_commit}"
write_family 1.2.3-rc.5 1.2.3rc5 SNP-AAAAAAAAAAAA
printf '\0release\n' >"${fixture}/component/blob.bin"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'changed binary release path'
binary_path_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure binary-path node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${binary_path_commit}"
grep -F 'release delta changes a binary non-authority path' \
  "${temporary_root}/binary-path.stderr" >/dev/null

git -C "${fixture}" checkout -q --detach "${qualified_commit}"
write_family 1.2.3 1.2.3 SNP-111111111111
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'qualified stable base'
stable_base_commit=$(git -C "${fixture}" rev-parse HEAD)

write_family 1.2.4 1.2.4 SNP-AAAAAAAAAAAA
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'stable patch release'
stable_patch_commit=$(git -C "${fixture}" rev-parse HEAD)
node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${stable_base_commit}" \
  --release-commit "${stable_patch_commit}" >"${temporary_root}/stable-patch.json"
jq -e '
  .decision == "pass" and
  .qualified_version == "1.2.3" and
  .release_version == "1.2.4"
' "${temporary_root}/stable-patch.json" >/dev/null

git -C "${fixture}" checkout -q --detach "${stable_base_commit}"
write_family 1.2.5 1.2.5 SNP-AAAAAAAAAAAA
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'stable patch skip'
stable_skip_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure stable-skip node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${stable_base_commit}" \
  --release-commit "${stable_skip_commit}"
grep -F 'advance a qualified stable base by exactly one patch version' \
  "${temporary_root}/stable-skip.stderr" >/dev/null

git -C "${fixture}" checkout -q --detach "${stable_base_commit}"
write_family 1.3.0 1.3.0 SNP-AAAAAAAAAAAA
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'stable minor jump'
stable_minor_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure stable-minor node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${stable_base_commit}" \
  --release-commit "${stable_minor_commit}"

git -C "${fixture}" checkout -q --detach "${stable_base_commit}"
printf 'own 1.2.3 dependency 1.2.3 kept\n' >"${fixture}/component/mixed.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'qualified stable base with mixed occurrences'
mixed_base_commit=$(git -C "${fixture}" rev-parse HEAD)
git -C "${fixture}" checkout -q --detach "${mixed_base_commit}"
write_family 1.2.4 1.2.4 SNP-AAAAAAAAAAAA
printf 'own 1.2.4 dependency 1.2.3 kept\n' >"${fixture}/component/mixed.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'stable patch advancing only the family occurrence'
mixed_patch_commit=$(git -C "${fixture}" rev-parse HEAD)
node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${mixed_base_commit}" \
  --release-commit "${mixed_patch_commit}" >"${temporary_root}/mixed-patch.json"
jq -e '.decision == "pass"' "${temporary_root}/mixed-patch.json" >/dev/null

git -C "${fixture}" checkout -q --detach "${mixed_base_commit}"
write_family 1.2.4 1.2.4 SNP-AAAAAAAAAAAA
printf 'own 1.2.4 dependency 9.9.9 kept\n' >"${fixture}/component/mixed.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'stable patch with a non-token edit'
mixed_bad_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure mixed-non-token node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${mixed_base_commit}" \
  --release-commit "${mixed_bad_commit}"
grep -F 'release delta contains non-version changes' \
  "${temporary_root}/mixed-non-token.stderr" >/dev/null

printf 'pre-RC qualification contract: pass\n'
