#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' 'usage: release_authority_preflight.sh <ait-native-root> <evidence-output>' >&2
  exit 64
}

fail() {
  local code=$1
  shift
  printf '%s\n' "$*" >&2
  exit "${code}"
}

sha256_file() {
  local input=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${input}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${input}" | awk '{print $1}'
  else
    fail 69 'no SHA-256 utility is available'
  fi
}

canonical_directory() {
  (cd "$1" && pwd -P)
}

canonical_file() {
  local input=$1
  (cd "$(dirname -- "${input}")" && printf '%s/%s\n' "$(pwd -P)" "$(basename -- "${input}")")
}

[[ $# -eq 2 ]] || usage
native_root=$1
evidence_output=$2
[[ ${native_root} == /* && -d ${native_root} && ! -L ${native_root} ]] ||
  fail 66 'ait-native root must be an absolute real directory'
native_root=$(canonical_directory "${native_root}")
[[ ${evidence_output} == /* ]] || fail 64 'authority evidence output must be absolute'
[[ ! -e ${evidence_output} && ! -L ${evidence_output} ]] ||
  fail 73 "authority evidence output already exists: ${evidence_output}"
[[ -d $(dirname -- "${evidence_output}") && ! -L $(dirname -- "${evidence_output}") ]] ||
  fail 66 'authority evidence parent must be a real directory'
evidence_output=$(canonical_file "${evidence_output}")

template=${native_root}/ait-release-family.json
authorities=${native_root}/ci/release_repository_authorities.json
manifest=${native_root}/ait-external.toml
lockfile=${native_root}/ait-external.lock
ait_bin=${AIT_RELEASE_AIT_BIN:-ait}
for input in "${template}" "${authorities}" "${manifest}" "${lockfile}"; do
  [[ -f ${input} && ! -L ${input} ]] ||
    fail 66 "release authority input must be a regular file: ${input}"
done
command -v "${ait_bin}" >/dev/null 2>&1 || fail 69 "AIT CLI is unavailable: ${ait_bin}"
for command in jq; do
  command -v "${command}" >/dev/null 2>&1 || fail 69 "required command is unavailable: ${command}"
done

family_version=$(jq -er '.family.version' "${template}")
family_tag=$(jq -er '.family.tag' "${template}")
[[ ${family_tag} == v${family_version} ]] || fail 65 'family tag and version differ'
python_version=${family_version/-rc./rc}
node "${native_root}/ci/release_external_versions.mjs" "${native_root}" "${family_version}"
jq -e '
  .schema == "ait.release.family/v3" and
  ([.components[].source_snapshot] | unique == ["unbound"])
' "${template}" >/dev/null ||
  fail 65 'canonical family template must keep every source_snapshot unbound'
jq -e --arg version "${family_version}" '
  .contract == "ait.release.repository-authorities/v1" and
  .schema_version == 1 and .family_version == $version and
  .source_line == "main" and .public_publish == false and
  ([.repositories[].repo_name] | sort) ==
    ["ait-core", "ait-node", "ait-python", "ait-runner", "ait-server"] and
  ([.repositories[].repository_index] | sort) == [0, 1, 2, 3, 4]
' "${authorities}" >/dev/null ||
  fail 65 'canonical Repository-authority contract is inconsistent'

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-authority.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-authority.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected authority-preflight path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

status_record=${temporary_root}/external-status.json
(cd "${native_root}" && "${ait_bin}" external status --json >"${status_record}")
jq -e '
  .command == "external status" and
  ([.externals[] | .lock_drift] | all(. == false)) and
  (.lock_drifts | length) == 0
' "${status_record}" >/dev/null ||
  fail 65 'ait-external.lock drifts from ait-external.toml'
# Language bindings share the selected producer. Runner retains the dependency
# declared by its own accepted source, rather than an implicit family repin.
node --input-type=module - "${native_root}" "${status_record}" <<'NODE'
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
const [root, record] = process.argv.slice(2);
const { validateReleaseDependencyGraph } = await import(pathToFileURL(root + '/ci/release_runner_dependency.mjs'));
validateReleaseDependencyGraph(root, JSON.parse(readFileSync(record, 'utf8')));
NODE
jq -e '
  [.externals[] | select(.parent_path == "")] as $direct |
  ($direct | length) == 5 and
  ([$direct[].name] | sort) == ["ait-core", "ait-node", "ait-python", "ait-runner", "ait-server"] and
  all($direct[]; .name == .repo_name and .state == "materialized" and .dirty == false and
    .linked == false and (.snapshot | test("^SNP-[0-9A-F]{12}$"))) and
  ([$direct[].snapshot] | unique | length) == 5
' "${status_record}" >/dev/null ||
  fail 65 'direct component externals must be exactly the five family repositories, materialized, clean, and distinct'

rows=${temporary_root}/rows.jsonl
: >"${rows}"
while IFS=$'\t' read -r repo_name repository_index namespace; do
  external=$(jq -c --arg repo "${repo_name}" '
    [.externals[] | select(.parent_path == "" and .name == $repo)] | .[0]
  ' "${status_record}")
  [[ ${external} != null ]] || fail 65 "component external is undeclared: ${repo_name}"
  jq -e --argjson index "${repository_index}" '.repository_index == $index' <<<"${external}" >/dev/null ||
    fail 65 "component external repository index differs: ${repo_name}"
  snapshot=$(jq -er '.snapshot' <<<"${external}")
  materialize_to=$(jq -er '.materialize_to' <<<"${external}")
  license=$(jq -er '.license' <<<"${external}")
  expected_license=$(jq -er --arg repo "${repo_name}" '
    [.components[] | select(.source_repository == $repo) | .license] | unique |
    if length == 1 then .[0] else error("component license is not unique") end
  ' "${template}")
  [[ ${license} == "${expected_license}" ]] ||
    fail 65 "component external license differs from the family template: ${repo_name}"
  tree=${native_root}/${materialize_to}
  marker=${tree}/.ait-external-marker.json
  [[ -d ${tree} && ! -L ${tree} && -f ${marker} && ! -L ${marker} ]] ||
    fail 66 "component external tree is not materialized: ${repo_name}"
  jq -e --arg repo "${repo_name}" --arg snapshot "${snapshot}" '
    .format == "ait.external.materialized" and .repo_name == $repo and .snapshot == $snapshot
  ' "${marker}" >/dev/null ||
    fail 65 "component external marker differs from the lock pin: ${repo_name}"
  adapter=${tree}/ait-release.json
  [[ -f ${adapter} && ! -L ${adapter} ]] ||
    fail 66 "component release adapter is unavailable: ${repo_name}"
  jq -e --arg version "${family_version}" --arg python "${python_version}" --arg repo "${repo_name}" '
    .schema == "ait.release.adapter/v1" and
    (.package.version == (if $repo == "ait-python" then $python else $version end))
  ' "${adapter}" >/dev/null ||
    fail 65 "component release adapter version differs from the family version: ${repo_name}"
  snapshot_record=${temporary_root}/${repo_name}-snapshot.json
  (cd "${native_root}" && "${ait_bin}" snapshot show "${snapshot}" --json >"${snapshot_record}")
  jq -e --arg snapshot "${snapshot}" '
    .snapshot_id == $snapshot and (.manifest_hash | test("^[0-9a-f]{64}$")) and
    (.created_at | type == "string" and test("^(0|[1-9][0-9]*)$"))
  ' "${snapshot_record}" >/dev/null ||
    fail 65 "component Snapshot metadata is unavailable locally: ${repo_name}"
  remote_url=$(jq -er '.remotes.origin.url' "${native_root}/.ait/config.json" 2>/dev/null ||
    printf 'unconfigured')
  jq -cn \
    --arg repo_name "${repo_name}" \
    --argjson repository_index "${repository_index}" \
    --arg namespace "${namespace}" \
    --arg snapshot "${snapshot}" \
    --arg manifest_hash "$(jq -er '.manifest_hash' "${snapshot_record}")" \
    --arg created_at "$(jq -er '.created_at' "${snapshot_record}")" \
    --arg version "${family_version}" \
    --arg license "${license}" \
    --arg materialize_to "${materialize_to}" \
    --arg remote_url "${remote_url}" '
    {
      repo_name: $repo_name,
      repository_index: $repository_index,
      namespace: $namespace,
      selected_snapshot: $snapshot,
      selected_manifest_hash: $manifest_hash,
      selected_snapshot_created_at: $created_at,
      version: $version,
      license: $license,
      materialize_to: $materialize_to,
      remote: "origin",
      remote_url: $remote_url,
      workspace_clean: true
    }
  ' >>"${rows}"
done < <(jq -er '.repositories[] | [.repo_name, (.repository_index | tostring), .namespace] | @tsv' \
  "${authorities}")

repository_rows=$(jq -s 'sort_by(.repository_index)' "${rows}")
jq -S -n \
  --arg family_version "${family_version}" \
  --arg family_tag "${family_tag}" \
  --arg native_root "${native_root}" \
  --arg template_sha256 "$(sha256_file "${template}")" \
  --arg manifest_sha256 "$(sha256_file "${manifest}")" \
  --arg lock_sha256 "$(sha256_file "${lockfile}")" \
  --arg authorities_sha256 "$(sha256_file "${authorities}")" \
  --argjson repositories "${repository_rows}" '
  {
    contract: "ait.release.canonical-authority-preflight/v2",
    status: "ready",
    family_version: $family_version,
    family_tag: $family_tag,
    ait_native_root: $native_root,
    family_template_sha256: $template_sha256,
    external_manifest_sha256: $manifest_sha256,
    external_lock_sha256: $lock_sha256,
    repository_authorities_sha256: $authorities_sha256,
    binding: {
      contract: "ait.release.family-binding/v1",
      version: $family_version,
      tag: $family_tag,
      components: ($repositories | map({key: .repo_name, value: .selected_snapshot}) | from_entries)
    },
    repositories: $repositories,
    recovery_authority_used: false,
    source_snapshot_rewritten: false,
    artifact_rebuild: false,
    registry_write: false,
    public_publish: false
  }
' >"${temporary_root}/evidence.json"
mv "${temporary_root}/evidence.json" "${evidence_output}"
printf '%s\n' "${evidence_output}"
