#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' \
    'usage: release_authority_preflight.sh <canonical-ait-core-root> <evidence-output> [<qualification-family-manifest>]' >&2
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

[[ $# -eq 2 || $# -eq 3 ]] || usage
canonical_core=$1
evidence_output=$2
qualification_family=${3:-}
[[ ${canonical_core} == /* && -d ${canonical_core} && ! -L ${canonical_core} ]] ||
  fail 66 'canonical ait-core root must be an absolute real directory'
canonical_core=$(canonical_directory "${canonical_core}")
[[ ${canonical_core##*/} == ait-core ]] ||
  fail 65 'canonical release authority root must be named exactly ait-core'
workspace_root=$(dirname -- "${canonical_core}")
[[ ${evidence_output} == /* ]] || fail 64 'authority evidence output must be absolute'
[[ ! -e ${evidence_output} && ! -L ${evidence_output} ]] ||
  fail 73 "authority evidence output already exists: ${evidence_output}"
[[ -d $(dirname -- "${evidence_output}") && ! -L $(dirname -- "${evidence_output}") ]] ||
  fail 66 'authority evidence parent must be a real directory'
evidence_output=$(canonical_file "${evidence_output}")

canonical_family=${canonical_core}/ait-release-family.json
family=${canonical_family}
authorities=${canonical_core}/ci/release_repository_authorities.json
ait_bin=${canonical_core}/.ait/cargo-target/release/ait-cli
for input in "${canonical_family}" "${authorities}"; do
  [[ -f ${input} && ! -L ${input} ]] ||
    fail 66 "canonical release input must be a regular file: ${input}"
done
[[ -x ${ait_bin} && ! -L ${ait_bin} ]] ||
  fail 66 'canonical ait-core native CLI is unavailable or symlinked'

qualification_family_used=false
if [[ -n ${qualification_family} ]]; then
  [[ ${qualification_family} == /* ]] ||
    fail 64 'qualification family manifest must be absolute'
  [[ -f ${qualification_family} && ! -L ${qualification_family} ]] ||
    fail 66 'qualification family manifest must be a regular non-symlink file'
  qualification_family=$(canonical_file "${qualification_family}")
  [[ ${qualification_family} != "${canonical_family}" ]] ||
    fail 65 'qualification family manifest must be independent from canonical authority'
  case "${qualification_family}" in
    "${canonical_core}"/*)
      fail 65 'qualification family manifest must remain outside canonical ait-core'
      ;;
  esac
  jq -e --slurpfile canonical "${canonical_family}" '
    . as $qualification |
    $canonical[0] as $published |
    $qualification != $published and
    (($qualification | del(.components)) == ($published | del(.components))) and
    (($qualification.components | map(del(.source_snapshot))) ==
      ($published.components | map(del(.source_snapshot)))) and
    ([ $qualification.components[].source_snapshot ] |
      all(type == "string" and test("^SNP-[0-9A-F]{12}$")))
  ' "${qualification_family}" >/dev/null ||
    fail 65 'qualification family may differ only in valid component source_snapshot values'
  family=${qualification_family}
  qualification_family_used=true
fi

family_version=$(jq -er '.family.version' "${family}")
family_tag=$(jq -er '.family.tag' "${family}")
[[ ${family_tag} == v${family_version} ]] || fail 65 'family tag and version differ'
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
rows=${temporary_root}/rows.jsonl
: >"${rows}"

while IFS=$'\t' read -r repo_name repository_index namespace; do
  repo_root=${workspace_root}/${repo_name}
  [[ -d ${repo_root} && ! -L ${repo_root} ]] ||
    fail 66 "canonical component root is missing or symlinked: ${repo_root}"
  repo_root=$(canonical_directory "${repo_root}")
  [[ ${repo_root} == "${workspace_root}/${repo_name}" ]] ||
    fail 65 "canonical component root resolves outside the workspace: ${repo_name}"
  [[ -d ${repo_root}/.ait && ! -L ${repo_root}/.ait ]] ||
    fail 66 "canonical component .ait authority is missing or symlinked: ${repo_name}"
  config=${repo_root}/.ait/config.json
  [[ -f ${config} && ! -L ${config} ]] ||
    fail 66 "canonical component config is unavailable: ${repo_name}"
  jq -e --arg repo "${repo_name}" --argjson index "${repository_index}" \
    --arg namespace "${namespace}" '
    .repo_name == $repo and .repository_index == $index and
    .id_namespace_prefix == $namespace and .default_remote == "origin" and
    (.remotes.origin.url | type == "string" and length > 0) and
    .remotes.origin.repo_name == $repo
  ' "${config}" >/dev/null ||
    fail 65 "canonical component config identity differs: ${repo_name}"

  source_identity=$(jq -cer --arg repo "${repo_name}" '
    [.components[] | select(.source_repository == $repo) |
      {snapshot: .source_snapshot, version: .version, license: .license}] |
    unique |
    if length == 1 then .[0] else error("component source identity is not unique") end
  ' "${family}")
  source_snapshot=$(jq -er '.snapshot | select(test("^SNP-[0-9A-F]{12}$"))' \
    <<<"${source_identity}")
  component_version=$(jq -er '.version' <<<"${source_identity}")
  component_license=$(jq -er '.license' <<<"${source_identity}")

  status_record=${temporary_root}/${repo_name}-status.json
  snapshot_record=${temporary_root}/${repo_name}-snapshot.json
  line_record=${temporary_root}/${repo_name}-line.json
  ancestry_record=${temporary_root}/${repo_name}-ancestry.json
  (
    cd "${repo_root}"
    "${ait_bin}" status --json >"${status_record}"
    "${ait_bin}" snapshot show "${source_snapshot}" --json >"${snapshot_record}"
    "${ait_bin}" line show main --json >"${line_record}"
    "${ait_bin}" snapshot is-ancestor "${source_snapshot}" \
      "$(jq -er '.head_snapshot_id' "${line_record}")" --json >"${ancestry_record}"
  )
  jq -e --arg repo "${repo_name}" '
    .repo_name == $repo and .workspace.status == "clean" and
    .workspace.changed_count == 0
  ' "${status_record}" >/dev/null ||
    fail 65 "canonical component workspace is not clean: ${repo_name}"
  jq -e --arg snapshot "${source_snapshot}" \
    '.snapshot_id == $snapshot and (.manifest_hash | test("^[0-9a-f]{64}$"))' \
    "${snapshot_record}" >/dev/null ||
    fail 65 "selected release Snapshot is absent from canonical Binary DB: ${repo_name}"
  jq -e --arg older "${source_snapshot}" '
    .contract == "snapshot-is-ancestor/v1" and
    .older_snapshot_id == $older and .is_ancestor == true
  ' "${ancestry_record}" >/dev/null ||
    fail 65 "selected release Snapshot is not retained by canonical main: ${repo_name}"

  jq -cn \
    --arg repo_name "${repo_name}" \
    --argjson repository_index "${repository_index}" \
    --arg namespace "${namespace}" \
    --arg snapshot "${source_snapshot}" \
    --arg manifest_hash "$(jq -er '.manifest_hash' "${snapshot_record}")" \
    --arg main_head "$(jq -er '.head_snapshot_id' "${line_record}")" \
    --arg version "${component_version}" \
    --arg license "${component_license}" \
    --arg remote_url "$(jq -er '.remotes.origin.url' "${config}")" '
    {
      repo_name: $repo_name,
      repository_index: $repository_index,
      namespace: $namespace,
      selected_snapshot: $snapshot,
      selected_manifest_hash: $manifest_hash,
      canonical_main_head: $main_head,
      selected_snapshot_retained_by_main: true,
      version: $version,
      license: $license,
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
  --arg canonical_core "${canonical_core}" \
  --arg family_sha256 "$(sha256_file "${family}")" \
  --arg canonical_family_sha256 "$(sha256_file "${canonical_family}")" \
  --arg authorities_sha256 "$(sha256_file "${authorities}")" \
  --argjson qualification_family_used "${qualification_family_used}" \
  --argjson repositories "${repository_rows}" '
  {
    contract: "ait.release.canonical-authority-preflight/v1",
    status: "ready",
    family_version: $family_version,
    family_tag: $family_tag,
    canonical_ait_core_root: $canonical_core,
    family_manifest_sha256: $family_sha256,
    canonical_family_manifest_sha256: $canonical_family_sha256,
    qualification_family_manifest_sha256:
      (if $qualification_family_used then $family_sha256 else null end),
    qualification_family_used: $qualification_family_used,
    repository_authorities_sha256: $authorities_sha256,
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
