#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
preflight=${repo_root}/ci/release_authority_preflight.sh
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-authority-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-authority-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected authority test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected authority-preflight failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

test -x "${preflight}"
bash -n "${preflight}"

workspace=${temporary_root}/workspace
canonical_core=${workspace}/ait-core
mkdir -p "${canonical_core}/ci" "${canonical_core}/.ait/cargo-target/release"
cp "${repo_root}/ait-release-family.json" "${canonical_core}/ait-release-family.json"
cp "${repo_root}/ci/release_repository_authorities.json" \
  "${canonical_core}/ci/release_repository_authorities.json"

state=${temporary_root}/state.json
jq -n --slurpfile family "${canonical_core}/ait-release-family.json" '
  reduce ($family[0].components[] |
    {key: .source_repository, value: .source_snapshot}) as $row ({};
      .[$row.key] = {snapshot: $row.value, dirty: false})
' >"${state}"

while IFS=$'\t' read -r repo_name repository_index namespace; do
  component=${workspace}/${repo_name}
  mkdir -p "${component}/.ait"
  jq -n --arg repo "${repo_name}" --argjson index "${repository_index}" \
    --arg namespace "${namespace}" '
    {
      repo_name: $repo,
      repository_index: $index,
      id_namespace_prefix: $namespace,
      default_remote: "origin",
      remotes: {
        origin: {
          repo_name: $repo,
          url: "http://127.0.0.1:8088"
        }
      }
    }
  ' >"${component}/.ait/config.json"
done < <(jq -er '.repositories[] | [.repo_name, (.repository_index | tostring), .namespace] | @tsv' \
  "${canonical_core}/ci/release_repository_authorities.json")

cat >"${canonical_core}/.ait/cargo-target/release/ait-cli" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_TEST_AUTHORITY_STATE:?}
repo=${PWD##*/}
snapshot=$(jq -er --arg repo "${repo}" '.[$repo].snapshot' "${state}")
case "${1:-} ${2:-}" in
  'status --json')
    dirty=$(jq -r --arg repo "${repo}" '.[$repo].dirty' "${state}")
    if [[ ${dirty} == true ]]; then
      jq -n --arg repo "${repo}" \
        '{repo_name: $repo, workspace_status: "dirty", workspace_changed_count: 1}'
    else
      jq -n --arg repo "${repo}" \
        '{repo_name: $repo, workspace_status: "clean", workspace_changed_count: 0}'
    fi
    ;;
  'snapshot show')
    [[ ${3:-} == "${snapshot}" && ${4:-} == --json ]] || exit 2
    jq -n --arg snapshot "${snapshot}" \
      '{snapshot_id: $snapshot, manifest_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
    ;;
  'line show')
    [[ ${3:-} == main && ${4:-} == --json ]] || exit 64
    jq -n --arg snapshot "${snapshot}" \
      '{line_name: "main", head_snapshot_id: $snapshot}'
    ;;
  'snapshot is-ancestor')
    [[ ${3:-} == "${snapshot}" && ${4:-} == "${snapshot}" && ${5:-} == --json ]] || exit 2
    jq -n --arg snapshot "${snapshot}" '
      {
        contract: "snapshot-is-ancestor/v1",
        older_snapshot_id: $snapshot,
        newer_snapshot_id: $snapshot,
        is_ancestor: true,
        distance: 0
      }
    '
    ;;
  *) exit 64 ;;
esac
STUB
chmod 0755 "${canonical_core}/.ait/cargo-target/release/ait-cli"

export AIT_TEST_AUTHORITY_STATE=${state}
evidence=${temporary_root}/authority.json
"${preflight}" "${canonical_core}" "${evidence}" >/dev/null
jq -e '
  .contract == "ait.release.canonical-authority-preflight/v1" and
  .status == "ready" and .family_version == "1.0.0-rc.6" and
  (.repositories | length) == 5 and
  ([.repositories[].repository_index] | sort) == [0, 1, 2, 3, 4] and
  ([.repositories[].selected_snapshot_retained_by_main] | all(. == true)) and
  ([.repositories[].workspace_clean] | all(. == true)) and
  .recovery_authority_used == false and .registry_write == false and
  .public_publish == false
' "${evidence}" >/dev/null

cp "${preflight}" "${canonical_core}/ci/release_authority_preflight.sh"
chmod 0755 "${canonical_core}/ci/release_authority_preflight.sh"
cp "${repo_root}/ci/patch_ci.json" "${canonical_core}/ci/patch_ci.json"
cat >"${canonical_core}/ci/release_source_cache.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 9 ]]
repo_name=$2
repository_index=$3
source_snapshot=$5
version=$6
license=$7
destination=$9
evidence=${AIT_RELEASE_SOURCE_EVIDENCE_PATH:?}
mkdir -p "${destination}/source"
printf '%s %s\n' "${repo_name}" "${source_snapshot}" \
  >"${destination}/source/fixture.txt"
jq -n --arg repo "${repo_name}" --argjson index "${repository_index}" \
  --arg snapshot "${source_snapshot}" --arg version "${version}" \
  --arg license "${license}" '
  {
    contract: "ait.release.source-cache/v1",
    status: "ready",
    repo_name: $repo,
    repository_index: $index,
    source_snapshot: $snapshot,
    version: $version,
    license: $license,
    workspace_clean: true,
    remote_coordinates_embedded: false,
    public_publish: false
  }
' >"${evidence}"
STUB
chmod 0755 "${canonical_core}/ci/release_source_cache.sh"

source_bundles=${temporary_root}/source-bundles
"${repo_root}/ci/release_source_bundles.sh" "${canonical_core}" \
  "${source_bundles}" >/dev/null
jq -e '
  .contract == "ait.release.source-bundles/v1" and .status == "ready" and
  .family_version == "1.0.0-rc.6" and .source_bundle_count == 5 and
  (.bundles | length) == 5 and .recovery_authority_used == false and
  .registry_write == false and .public_publish == false
' "${source_bundles}/source-bundles.evidence.json" >/dev/null
test "$(find "${source_bundles}" -mindepth 1 -maxdepth 1 -type d \
  -name 'ait-release-source-*' | wc -l | tr -d '[:space:]')" = 5
while IFS= read -r repo_name; do
  bundle=${source_bundles}/ait-release-source-${repo_name}
  test -f "${bundle}/source-cache.tar.gz"
  test -f "${bundle}/source-cache.evidence.json"
  tar -tzf "${bundle}/source-cache.tar.gz" | grep -F './source/fixture.txt' >/dev/null
done < <(jq -er '.repositories[].repo_name' \
  "${canonical_core}/ci/release_repository_authorities.json")
test -z "$(find "${source_bundles}" -maxdepth 1 -name '.source-cache-*' -print -quit)"

expect_failure existing-output "${preflight}" "${canonical_core}" "${evidence}"
mkdir "${workspace}/.ait-core-recovery"
expect_failure recovery-name "${preflight}" "${workspace}/.ait-core-recovery" \
  "${temporary_root}/recovery.json"

jq '."ait-node".dirty = true' "${state}" >"${state}.new"
mv "${state}.new" "${state}"
expect_failure dirty-component "${preflight}" "${canonical_core}" \
  "${temporary_root}/dirty.json"
grep -F 'canonical component workspace is not clean: ait-node' \
  "${temporary_root}/dirty-component.stderr" >/dev/null

printf 'release canonical authority preflight contract: pass\n'
