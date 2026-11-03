#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' 'usage: release_source_bundles.sh <ait-native-root> <source-bundles-output>' >&2
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

[[ $# -eq 2 ]] || usage
native_root=$1
output=$2
[[ ${native_root} == /* && -d ${native_root} && ! -L ${native_root} ]] ||
  fail 66 'ait-native root must be an absolute real directory'
native_root=$(cd "${native_root}" && pwd -P)
[[ ${output} == /* ]] || fail 64 'source bundle output must be absolute'
[[ ! -e ${output} && ! -L ${output} ]] ||
  fail 73 "source bundle output already exists: ${output}"
output_parent=$(dirname -- "${output}")
[[ -d ${output_parent} && ! -L ${output_parent} ]] ||
  fail 66 'source bundle output parent must be a real directory'
output_parent=$(cd "${output_parent}" && pwd -P)
output=${output_parent}/$(basename -- "${output}")

preflight=${native_root}/ci/release_authority_preflight.sh
template=${native_root}/ait-release-family.json
[[ -x ${preflight} && ! -L ${preflight} ]] ||
  fail 66 "required source-bundle executable is unavailable: ${preflight}"
for command in jq tar; do
  command -v "${command}" >/dev/null 2>&1 ||
    fail 69 "required source-bundle command is unavailable: ${command}"
done

family_version=$(jq -er '.family.version' "${template}")
python_version=${family_version/-rc./rc}

validate_selected_core_authority() {
  local source_root=$1
  local selected_adapter=${source_root}/ait-release.json
  [[ -f ${selected_adapter} && ! -L ${selected_adapter} ]] ||
    fail 65 'selected ait-core source authority is unavailable: ait-release.json'
  jq -e --arg version "${family_version}" '
    .schema == "ait.release.adapter/v1" and .package.version == $version
  ' "${selected_adapter}" >/dev/null ||
    fail 65 'selected ait-core source authority differs from coordinator family'
}

staging=$(mktemp -d "${output_parent}/.ait-release-source-bundles.XXXXXX")
cleanup() {
  case "${staging}" in
    "${output_parent}"/.ait-release-source-bundles.*) rm -rf -- "${staging}" ;;
    *) printf 'refusing to remove unexpected source-bundle staging path: %s\n' \
      "${staging}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

authority=${staging}/canonical-authority.evidence.json
"${preflight}" "${native_root}" "${authority}" >/dev/null

rows=${staging}/source-bundle-rows.jsonl
: >"${rows}"
write_source_cache() {
  mkdir -p "${bundle_root}"
  adapter_sha256=$(sha256_file "${tree}/ait-release.json")
  family_manifest_sha256=null
  if [[ -f ${tree}/ait-release-family.json ]]; then
    family_manifest_sha256=$(sha256_file "${tree}/ait-release-family.json")
  fi
  # The materialized tree is the exact Snapshot content plus the AIT
  # materialization marker and nested external placeholders; only those are
  # excluded so the bundle equals the Snapshot tree.
  COPYFILE_DISABLE=1 tar -czf "${bundle_root}/source-cache.tar.gz" -C "${tree}" \
    --exclude='./.ait-external-marker.json' \
    --exclude='./.ait-external' --exclude='./.ait-external/*' \
    --exclude='./.ait' --exclude='./.ait/*' \
    --exclude='./.ait-runtime' --exclude='./.ait-runtime/*' \
    --exclude='./.ait-worktree-links' --exclude='./.ait-worktree-links/*' \
    --exclude='./.git' --exclude='./.git/*' .
  evidence=${bundle_root}/source-cache.evidence.json
  jq -n \
    --arg repo_name "${repo_name}" \
    --argjson repository_index "${repository_index}" \
    --arg namespace "${namespace}" \
    --arg snapshot "${snapshot}" \
    --arg manifest_hash "${manifest_hash}" \
    --arg created_at "${created_at}" \
    --arg version "${cache_version}" \
    --arg license "${license}" \
    --arg materialize_to "${materialize_to}" \
    --arg adapter_sha256 "${adapter_sha256}" \
    --arg family_manifest_sha256 "${family_manifest_sha256}" '
    {
      contract: "ait.release.source-cache/v1",
      status: "ready",
      repo_name: $repo_name,
      repository_index: $repository_index,
      namespace: $namespace,
      source_snapshot: $snapshot,
      source_manifest_hash: $manifest_hash,
      source_snapshot_created_at: $created_at,
      source_line: "main",
      version: $version,
      license: $license,
      source_authority: "ait_remote_snapshot_boundary",
      local_selection_authority: "ait_external_lock",
      materialize_to: $materialize_to,
      shallow_history_boundary: true,
      workspace_clean: true,
      workspace_external_materialized: false,
      remote_coordinates_embedded: false,
      adapter_manifest_sha256: $adapter_sha256,
      family_manifest_sha256: (if $family_manifest_sha256 == "null" then null else $family_manifest_sha256 end),
      public_publish: false
    }
  ' >"${evidence}"
  jq -cn \
    --arg repo_name "${repo_name}" \
    --argjson repository_index "${repository_index}" \
    --arg snapshot "${snapshot}" \
    --arg archive_sha256 "$(sha256_file "${bundle_root}/source-cache.tar.gz")" \
    --arg evidence_sha256 "$(sha256_file "${evidence}")" '
    {
      repo_name: $repo_name,
      repository_index: $repository_index,
      snapshot: $snapshot,
      archive_sha256: $archive_sha256,
      evidence_sha256: $evidence_sha256
    }
  ' >>"${rows}"
}

while IFS=$'\t' read -r repo_name repository_index namespace snapshot manifest_hash created_at license materialize_to; do
  tree=${native_root}/${materialize_to}
  bundle_root=${staging}/ait-release-source-${repo_name}
  cache_version=${family_version}
  if [[ ${repo_name} == ait-python ]]; then cache_version=${python_version}; fi
  if [[ ${repo_name} == ait-core ]]; then validate_selected_core_authority "${tree}"; fi
  write_source_cache
done < <(jq -er '.repositories[] |
  [.repo_name, (.repository_index | tostring), .namespace, .selected_snapshot,
   .selected_manifest_hash, .selected_snapshot_created_at, .license, .materialize_to] | @tsv' \
  "${authority}")


# A distinct Runner Core remains an additional ordinary source-cache/v1,
# addressed by the existing immutable declaration and path, not a new family
# component or a rewritten Snapshot.
IFS=$'\t' read -r runner_core runner_core_version < <(
  node --input-type=module - "${native_root}" <<'NODE'
import { pathToFileURL } from 'node:url';
const root = process.argv[2];
const { runnerCoreDeclaration } = await import(pathToFileURL(root + '/ci/release_runner_dependency.mjs'));
const row = runnerCoreDeclaration(root + '/.ait-external/ait-runner');
process.stdout.write(row.snapshot + '\t' + row.version + '\n');
NODE
)
if [[ ${runner_core} != $(jq -er '.binding.components["ait-core"]' "${authority}") ]]; then
  repo_name=ait-core
  repository_index=0
  namespace=$(jq -er '.repositories[] | select(.repo_name == "ait-core") | .namespace' "${authority}")
  snapshot=${runner_core}
  materialize_to=.ait-external/ait-runner/.ait-external/ait-core
  tree=${native_root}/${materialize_to}
  license=Apache-2.0
  cache_version=${runner_core_version}
  snapshot_record=${staging}/runner-core-snapshot.json
  (cd "${native_root}" && "${AIT_RELEASE_AIT_BIN:-ait}" snapshot show "${snapshot}" --json >"${snapshot_record}")
  [[ $(jq -er '.snapshot_id' "${snapshot_record}") == "${snapshot}" ]] || fail 65 'Runner Core Snapshot metadata differs'
  manifest_hash=$(jq -er '.manifest_hash | select(test("^[0-9a-f]{64}$"))' "${snapshot_record}")
  created_at=$(jq -er '.created_at | select(test("^[0-9]+$"))' "${snapshot_record}")
  bundle_root=${staging}/ait-release-source-ait-runner/.ait-external/ait-core
  write_source_cache
  rm -- "${snapshot_record}"
fi

jq -S -n \
  --arg family_version "${family_version}" \
  --arg family_tag "$(jq -er '.family.tag' "${template}")" \
  --arg template_sha256 "$(sha256_file "${template}")" \
  --arg authority_evidence_sha256 "$(sha256_file "${authority}")" \
  --argjson binding "$(jq -c '.binding' "${authority}")" \
  --argjson bundles "$(jq -s 'sort_by(.repository_index)' "${rows}")" '
  {
    contract: "ait.release.source-bundles/v2",
    status: "ready",
    family_version: $family_version,
    family_tag: $family_tag,
    family_template_sha256: $template_sha256,
    canonical_authority_evidence_sha256: $authority_evidence_sha256,
    binding: $binding,
    bundles: $bundles,
    source_bundle_count: ($bundles | length),
    selected_core_version_authority_verified: true,
    recovery_authority_used: false,
    artifact_rebuild: false,
    registry_write: false,
    public_publish: false
  }
' >"${staging}/source-bundles.evidence.json"
rm -- "${rows}"
mv "${staging}" "${output}"
trap - EXIT HUP INT TERM
printf '%s\n' "${output}/source-bundles.evidence.json"
