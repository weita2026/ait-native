#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
preflight=${repo_root}/ci/release_authority_preflight.sh
bundles=${repo_root}/ci/release_source_bundles.sh
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

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

test -x "${preflight}"
bash -n "${preflight}"
test -x "${bundles}"
bash -n "${bundles}"

native=${temporary_root}/ait-native
mkdir -p "${native}/ci" "${native}/.ait"
cp "${repo_root}/ait-release-family.json" "${native}/ait-release-family.json"
cp "${repo_root}/ci/release_repository_authorities.json" \
  "${native}/ci/release_repository_authorities.json"
cp "${preflight}" "${native}/ci/release_authority_preflight.sh"
cp "${repo_root}/ci/release_external_versions.mjs" "${native}/ci/release_external_versions.mjs"
cp "${repo_root}/ci/release_runner_dependency.mjs" "${native}/ci/release_runner_dependency.mjs"
chmod 0755 "${native}/ci/release_authority_preflight.sh"
printf '{"remotes":{"origin":{"url":"http://127.0.0.1:8088"}}}\n' >"${native}/.ait/config.json"
family_version=$(jq -er '.family.version' "${native}/ait-release-family.json")
python_version=${family_version/-rc./rc}
test "$(jq -r '[.components[].source_snapshot] | unique | join(",")' "${native}/ait-release-family.json")" = unbound

# The fixture state drives a stub `ait` that answers external status and
# snapshot show exactly like the real CLI for these five pins.
state=${temporary_root}/state.json
jq -n '
  {
    "ait-core": {index: 0, snapshot: "SNP-AAAAAAAAAAAA", license: "Apache-2.0"},
    "ait-server": {index: 1, snapshot: "SNP-BBBBBBBBBBBB", license: "AGPL-3.0-only"},
    "ait-python": {index: 2, snapshot: "SNP-CCCCCCCCCCCC", license: "Apache-2.0"},
    "ait-node": {index: 3, snapshot: "SNP-DDDDDDDDDDDD", license: "Apache-2.0"},
    "ait-runner": {index: 4, snapshot: "SNP-EEEEEEEEEEEE", license: "Apache-2.0"},
    lock_drift: false,
    nested_core: "SNP-AAAAAAAAAAAA",
    dirty: false
  }
' >"${state}"
: >"${native}/ait-external.toml"
: >"${native}/ait-external.lock"
for component in ait-core ait-server ait-python ait-node ait-runner; do
  version=${family_version}
  [[ ${component} != ait-python ]] || version=${python_version}
  printf '[[external]]\nname = "%s"\nversion = "%s"\n' "${component}" "${version}" >>"${native}/ait-external.toml"
  printf '[[node]]\nname = "%s"\nversion = "%s"\n' "${component}" "${version}" >>"${native}/ait-external.lock"
done

materialize() {
  local repo=$1
  local tree=${native}/.ait-external/${repo}
  local snapshot version
  snapshot=$(jq -er --arg repo "${repo}" '.[$repo].snapshot' "${state}")
  version=${family_version}
  [[ ${repo} == ait-python ]] && version=${python_version}
  mkdir -p "${tree}/ci"
  jq -n --arg repo "${repo}" --arg snapshot "${snapshot}" \
    --argjson index "$(jq -er --arg repo "${repo}" '.[$repo].index' "${state}")" '
    {format: "ait.external.materialized", name: $repo, repo_name: $repo,
     repository_index: $index, snapshot: $snapshot, line: "main", remote: "origin",
     materialize_to: (".ait-external/" + $repo), parent_path: "", version: 3}
  ' >"${tree}/.ait-external-marker.json"
  jq -n --arg version "${version}" '
    {schema: "ait.release.adapter/v1", package: {name: "fixture", version: $version}}
  ' >"${tree}/ait-release.json"
  printf 'fixture %s\n' "${repo}" >"${tree}/source.txt"
  printf 'license\n' >"${tree}/LICENSE"
  printf 'notice\n' >"${tree}/NOTICE"
  if [[ ${repo} == ait-core ]]; then
    cp "${native}/ait-release-family.json" "${tree}/ait-release-family.json"
    jq --arg version "${family_version}" '.family_version = $version' \
      "${native}/ci/release_repository_authorities.json" \
      >"${tree}/ci/release_repository_authorities.json"
    jq -n --arg version "${family_version}" \
      '{contract: "ait-native-bootstrap-matrix/v1", version: $version, public_publish: false}' \
      >"${tree}/ci/native_bootstrap_matrix.json"
  fi
}
for repo in ait-core ait-server ait-python ait-node ait-runner; do
  materialize "${repo}"
done
for owner in ait-node ait-python ait-runner; do
  parent=".ait-external/${owner}"
  tree="${native}/${parent}/.ait-external/ait-core"
  mkdir -p "${tree}"
  jq -n --arg parent "${parent}" '
    {format:"ait.external.materialized", repo_name:"ait-core", snapshot:"SNP-AAAAAAAAAAAA",
     repository_index:0, parent_path:$parent, materialize_to:($parent + "/.ait-external/ait-core")}
  ' >"${tree}/.ait-external-marker.json"
done
runner=${native}/.ait-external/ait-runner
for spec in 'ait-external.toml external' 'ait-external.lock node'; do
  read -r name table <<<"${spec}"
  printf '[[%s]]\nname = "ait-core"\nrepo_name = "ait-core"\nrepository_index = 0\nsnapshot = "SNP-AAAAAAAAAAAA"\nmaterialize_to = ".ait-external/ait-core"\nlicense = "Apache-2.0"\nversion = "%s"\n' \
    "${table}" "${family_version}" >"${runner}/${name}"
done

cat >"${temporary_root}/ait" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_TEST_AUTHORITY_STATE:?}
case "${1:-} ${2:-}" in
  'external status')
    [[ ${3:-} == --json ]] || exit 64
    jq '
      . as $state |
      def row($repo; $parent): {
        name: $repo, repo_name: $repo, repository_index: $state[$repo].index,
        snapshot: (if $parent == "" then $state[$repo].snapshot elif $parent == ".ait-external/ait-runner" then ($state.runner_core // $state.nested_core) else $state.nested_core end),
        license: $state[$repo].license, parent_path: $parent,
        materialize_to: (if $parent == "" then (".ait-external/" + $repo) else ($parent + "/.ait-external/" + $repo) end),
        state: "materialized",
        lock_drift: $state.lock_drift, dirty: $state.dirty, linked: false
      };
      {
        command: "external status",
        externals: ([("ait-core","ait-server","ait-python","ait-node","ait-runner") as $r | row($r; "")]
          + [(".ait-external/ait-runner", ".ait-external/ait-python", ".ait-external/ait-node") as $p | row("ait-core"; $p)]),
        lock_drifts: (if $state.lock_drift then [{name: "ait-core"}] else [] end),
        duplicates: [{
          name: "ait-core",
          policy: "allow",
          entries: ([{snapshot: $state["ait-core"].snapshot}] +
            [range(3) | {snapshot: $state.nested_core}])
        }]
      }
    ' "${state}"
    ;;
  'snapshot show')
    snapshot=${3:-}
    [[ ${4:-} == --json ]] || exit 64
    jq -e --arg snapshot "${snapshot}" 'any(.[]; (type == "object" and .snapshot == $snapshot) or (type == "string" and . == $snapshot))' "${state}" >/dev/null || exit 2
    jq -n --arg snapshot "${snapshot}" \
      '{snapshot_id: $snapshot, manifest_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", created_at: "1700000000"}'
    ;;
  *) exit 64 ;;
esac
STUB
chmod 0755 "${temporary_root}/ait"
export AIT_TEST_AUTHORITY_STATE=${state}
export AIT_RELEASE_AIT_BIN=${temporary_root}/ait

evidence=${temporary_root}/authority.json
"${preflight}" "${native}" "${evidence}" >/dev/null
jq -e --arg version "${family_version}" '
  .contract == "ait.release.canonical-authority-preflight/v2" and
  .status == "ready" and .family_version == $version and
  .binding.contract == "ait.release.family-binding/v1" and
  .binding.version == $version and
  .binding.components == {
    "ait-core": "SNP-AAAAAAAAAAAA", "ait-server": "SNP-BBBBBBBBBBBB",
    "ait-python": "SNP-CCCCCCCCCCCC", "ait-node": "SNP-DDDDDDDDDDDD",
    "ait-runner": "SNP-EEEEEEEEEEEE"
  } and
  (.repositories | length) == 5 and
  ([.repositories[].repository_index] | sort) == [0, 1, 2, 3, 4] and
  ([.repositories[].selected_manifest_hash] | all(test("^[0-9a-f]{64}$"))) and
  ([.repositories[].workspace_clean] | all(. == true)) and
  .recovery_authority_used == false and .registry_write == false and
  .public_publish == false
' "${evidence}" >/dev/null
expect_failure existing-output "${preflight}" "${native}" "${evidence}"

source_bundles=${temporary_root}/source-bundles
"${bundles}" "${native}" "${source_bundles}" >/dev/null
jq -e --arg version "${family_version}" '
  .contract == "ait.release.source-bundles/v2" and .status == "ready" and
  .family_version == $version and .source_bundle_count == 5 and
  .binding.components["ait-core"] == "SNP-AAAAAAAAAAAA" and
  (.bundles | length) == 5 and .selected_core_version_authority_verified == true and
  .registry_write == false and .public_publish == false
' "${source_bundles}/source-bundles.evidence.json" >/dev/null
for repo in ait-core ait-server ait-python ait-node ait-runner; do
  bundle=${source_bundles}/ait-release-source-${repo}
  test -f "${bundle}/source-cache.tar.gz"
  jq -e --arg repo "${repo}" '
    .contract == "ait.release.source-cache/v1" and .status == "ready" and
    .repo_name == $repo and .workspace_clean == true and
    .local_selection_authority == "ait_external_lock" and .public_publish == false
  ' "${bundle}/source-cache.evidence.json" >/dev/null
  tar -tzf "${bundle}/source-cache.tar.gz" | grep -F './source.txt' >/dev/null
  if tar -tzf "${bundle}/source-cache.tar.gz" | grep -F '.ait-external-marker.json' >/dev/null; then
    printf 'source bundle leaked the materialization marker: %s\n' "${repo}" >&2
    exit 65
  fi
done
expect_failure existing-bundles "${bundles}" "${native}" "${source_bundles}"

jq '.lock_drift = true' "${state}" >"${state}.new" && mv "${state}.new" "${state}"
expect_failure lock-drift "${preflight}" "${native}" "${temporary_root}/lock-drift.json"
grep -F 'drifts from ait-external.toml' "${temporary_root}/lock-drift.stderr" >/dev/null
jq '.lock_drift = false | .nested_core = "SNP-999999999999"' "${state}" >"${state}.new" && mv "${state}.new" "${state}"
expect_failure mixed-core "${preflight}" "${native}" "${temporary_root}/mixed-core.json"
grep -F 'Core dependency differs from its required producer' "${temporary_root}/mixed-core.stderr" >/dev/null
jq '.nested_core = "SNP-AAAAAAAAAAAA" | .dirty = true' "${state}" >"${state}.new" && mv "${state}.new" "${state}"
expect_failure dirty-external "${preflight}" "${native}" "${temporary_root}/dirty.json"
jq '.dirty = false' "${state}" >"${state}.new" && mv "${state}.new" "${state}"

jq '.runner_core = "SNP-999999999999"' "${state}" >"${state}.new" && mv "${state}.new" "${state}"
for name in ait-external.toml ait-external.lock; do
  sed 's/SNP-AAAAAAAAAAAA/SNP-999999999999/g' "${runner}/${name}" >"${runner}/${name}.new"
  mv "${runner}/${name}.new" "${runner}/${name}"
done
runner_marker=${runner}/.ait-external/ait-core/.ait-external-marker.json
jq '.snapshot = "SNP-999999999999"' "${runner_marker}" >"${runner_marker}.new" && mv "${runner_marker}.new" "${runner_marker}"
"${preflight}" "${native}" "${temporary_root}/valid-distinct-runner.json" >/dev/null
for name in ait-release.json source.txt LICENSE NOTICE; do
  cp "${native}/.ait-external/ait-core/${name}" "${runner}/.ait-external/ait-core/${name}"
done
printf 'declared distinct Runner Core\n' >"${runner}/.ait-external/ait-core/source.txt"
"${bundles}" "${native}" "${temporary_root}/distinct-source-bundles" >/dev/null
jq -e '.source_bundle_count == 6 and (.bundles | length) == 6 and any(.bundles[]; .snapshot == "SNP-999999999999")' \
  "${temporary_root}/distinct-source-bundles/source-bundles.evidence.json" >/dev/null
jq -e '.source_snapshot == "SNP-999999999999"' \
  "${temporary_root}/distinct-source-bundles/ait-release-source-ait-runner/.ait-external/ait-core/source-cache.evidence.json" >/dev/null
jq '.snapshot = "SNP-AAAAAAAAAAAA"' "${runner_marker}" >"${runner_marker}.new" && mv "${runner_marker}.new" "${runner_marker}"
expect_failure wrong-runner-materialization "${preflight}" "${native}" "${temporary_root}/wrong-runner-materialization.json"
for name in ait-external.toml ait-external.lock; do
  sed 's/SNP-999999999999/SNP-AAAAAAAAAAAA/g' "${runner}/${name}" >"${runner}/${name}.new"
  mv "${runner}/${name}.new" "${runner}/${name}"
done
jq 'del(.runner_core)' "${state}" >"${state}.new" && mv "${state}.new" "${state}"

jq '.snapshot = "SNP-FFFFFFFFFFFF"' "${native}/.ait-external/ait-node/.ait-external-marker.json" \
  >"${temporary_root}/marker.json"
cp "${native}/.ait-external/ait-node/.ait-external-marker.json" "${temporary_root}/marker.backup"
mv "${temporary_root}/marker.json" "${native}/.ait-external/ait-node/.ait-external-marker.json"
expect_failure marker-drift "${preflight}" "${native}" "${temporary_root}/marker-drift.json"
grep -F 'marker differs from the lock pin: ait-node' "${temporary_root}/marker-drift.stderr" >/dev/null
mv "${temporary_root}/marker.backup" "${native}/.ait-external/ait-node/.ait-external-marker.json"

jq '.package.version = "9.9.9"' "${native}/.ait-external/ait-runner/ait-release.json" \
  >"${temporary_root}/adapter.json"
cp "${native}/.ait-external/ait-runner/ait-release.json" "${temporary_root}/adapter.backup"
mv "${temporary_root}/adapter.json" "${native}/.ait-external/ait-runner/ait-release.json"
expect_failure adapter-version "${preflight}" "${native}" "${temporary_root}/adapter-version.json"
grep -F 'adapter version differs from the family version: ait-runner' \
  "${temporary_root}/adapter-version.stderr" >/dev/null
mv "${temporary_root}/adapter.backup" "${native}/.ait-external/ait-runner/ait-release.json"

jq '(.components[] | select(.id == "ait")).source_snapshot = "SNP-AAAAAAAAAAAA"' \
  "${native}/ait-release-family.json" >"${temporary_root}/bound-template.json"
cp "${native}/ait-release-family.json" "${temporary_root}/template.backup"
mv "${temporary_root}/bound-template.json" "${native}/ait-release-family.json"
expect_failure bound-template "${preflight}" "${native}" "${temporary_root}/bound-template.evidence.json"
grep -F 'must keep every source_snapshot unbound' "${temporary_root}/bound-template.stderr" >/dev/null
mv "${temporary_root}/template.backup" "${native}/ait-release-family.json"

cp "${native}/ait-external.toml" "${temporary_root}/manifest.saved"
sed 's/^version = ".*"/version = "9.9.9"/' "${temporary_root}/manifest.saved" >"${native}/ait-external.toml"
expect_failure metadata-version node "${repo_root}/ci/release_external_versions.mjs" "${native}" "${family_version}"
grep -F 'metadata version 9.9.9 differs from family' "${temporary_root}/metadata-version.stderr" >/dev/null
cp "${temporary_root}/manifest.saved" "${native}/ait-external.toml"
printf 'release canonical authority preflight contract: pass\n'
