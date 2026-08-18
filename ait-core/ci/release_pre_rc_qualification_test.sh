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
  jq -S -n --arg version "${version}" \
    '{schema: "ait.release.monorepo-source/v1", family_version: $version}' \
    >"${fixture}/ait-monorepo-source.json"
  jq -S -n --arg version "${version}" \
    '{version: $version, public_publish: false}' \
    >"${fixture}/ci/native_bootstrap_matrix.json"
  jq -S -n --arg version "${version}" \
    '{family_version: $version, public_publish: false}' \
    >"${fixture}/ci/release_repository_authorities.json"
  printf '%s\n' "${version}" >"${fixture}/component/version.txt"
}

write_family 1.2.3-rc.4 1.2.3rc4
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'qualified repair'
qualified_commit=$(git -C "${fixture}" rev-parse HEAD)

write_family 1.2.3-rc.5 1.2.3rc5
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
    .normalized_version_paths == ["component/version.txt"]
  ' "${temporary_root}/delta.json" >/dev/null

printf 'behavior changed\n' >"${fixture}/component/behavior.txt"
git -C "${fixture}" add -A
git -C "${fixture}" commit -qm 'forbidden release behavior'
non_version_commit=$(git -C "${fixture}" rev-parse HEAD)
expect_failure non-version node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${release_commit}" \
  --release-commit "${non_version_commit}"

expect_failure non-direct node "${delta}" \
  --repository "${fixture}" \
  --qualified-commit "${qualified_commit}" \
  --release-commit "${non_version_commit}"

printf 'pre-RC qualification contract: pass\n'
