#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' \
    'usage: release_source_bundles.sh <canonical-ait-core-root> <source-bundles-output> [<qualification-family-manifest>]' >&2
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

[[ $# -eq 2 || $# -eq 3 ]] || usage
canonical_core=$1
output=$2
qualification_family=${3:-}
[[ ${canonical_core} == /* && -d ${canonical_core} && ! -L ${canonical_core} ]] ||
  fail 66 'canonical ait-core root must be an absolute real directory'
canonical_core=$(cd "${canonical_core}" && pwd -P)
[[ ${canonical_core##*/} == ait-core ]] ||
  fail 65 'canonical release authority root must be named exactly ait-core'
[[ ${output} == /* ]] || fail 64 'source bundle output must be absolute'
[[ ! -e ${output} && ! -L ${output} ]] ||
  fail 73 "source bundle output already exists: ${output}"
output_parent=$(dirname -- "${output}")
[[ -d ${output_parent} && ! -L ${output_parent} ]] ||
  fail 66 'source bundle output parent must be a real directory'
output_parent=$(cd "${output_parent}" && pwd -P)
output=${output_parent}/$(basename -- "${output}")

preflight=${canonical_core}/ci/release_authority_preflight.sh
source_cache=${canonical_core}/ci/release_source_cache.sh
canonical_family=${canonical_core}/ait-release-family.json
family=${canonical_family}
authorities=${canonical_core}/ci/release_repository_authorities.json
patch_ci=${canonical_core}/ci/patch_ci.json
ait_bin=${canonical_core}/.ait/cargo-target/release/ait-cli
for input in "${preflight}" "${source_cache}" "${ait_bin}"; do
  [[ -x ${input} && ! -L ${input} ]] ||
    fail 66 "required source-bundle executable is unavailable: ${input}"
done
for input in "${canonical_family}" "${authorities}" "${patch_ci}"; do
  [[ -f ${input} && ! -L ${input} ]] ||
    fail 66 "required source-bundle input is unavailable: ${input}"
done
qualification_family_used=false
if [[ -n ${qualification_family} ]]; then
  [[ ${qualification_family} == /* ]] ||
    fail 64 'qualification family manifest must be absolute'
  [[ -f ${qualification_family} && ! -L ${qualification_family} ]] ||
    fail 66 'qualification family manifest must be a regular non-symlink file'
  family=$(cd "$(dirname -- "${qualification_family}")" &&
    printf '%s/%s\n' "$(pwd -P)" "$(basename -- "${qualification_family}")")
  qualification_family_used=true
fi
for command in jq tar; do
  command -v "${command}" >/dev/null 2>&1 ||
    fail 69 "required source-bundle command is unavailable: ${command}"
done

family_version=$(jq -er '.family.version' "${family}")
python_version=${family_version/-rc./rc}

validate_selected_core_authority() {
  local source_root=$1
  local selected_family=${source_root}/ait-release-family.json
  local selected_adapter=${source_root}/ait-release.json
  local selected_authorities=${source_root}/ci/release_repository_authorities.json
  local selected_platforms=${source_root}/ci/native_bootstrap_matrix.json
  local selected_input
  for selected_input in \
    "${selected_family}" \
    "${selected_adapter}" \
    "${selected_authorities}" \
    "${selected_platforms}"; do
    [[ -f ${selected_input} && ! -L ${selected_input} ]] ||
      fail 65 "selected ait-core source authority is unavailable: ${selected_input#${source_root}/}"
  done
  jq -e --arg version "${family_version}" --arg python "${python_version}" '
    .family.version == $version and .family.tag == ("v" + $version) and
    ([.components[] |
      if .version_scheme == "pep440" then .version == $python
      else .version == $version end] | all)
  ' "${selected_family}" >/dev/null &&
    jq -e --arg version "${family_version}" '
      .schema == "ait.release.adapter/v1" and .package.version == $version
    ' "${selected_adapter}" >/dev/null &&
    jq -e --arg version "${family_version}" '
      .contract == "ait.release.repository-authorities/v1" and
      .family_version == $version and .public_publish == false
    ' "${selected_authorities}" >/dev/null &&
    jq -e --arg version "${family_version}" '
      .contract == "ait-native-bootstrap-matrix/v1" and
      .version == $version and .public_publish == false
    ' "${selected_platforms}" >/dev/null ||
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

preflight_args=(
  "${canonical_core}"
  "${staging}/canonical-authority.evidence.json"
)
if [[ ${qualification_family_used} == true ]]; then
  preflight_args+=("${family}")
fi
"${preflight}" "${preflight_args[@]}" >/dev/null

workspace_root=$(dirname -- "${canonical_core}")
remote_urls=${staging}/remote-urls.txt
: >"${remote_urls}"
while IFS= read -r repo_name; do
  jq -er '.remotes.origin.url' "${workspace_root}/${repo_name}/.ait/config.json" \
    >>"${remote_urls}"
done < <(jq -er '.repositories | sort_by(.repository_index)[] | .repo_name' "${authorities}")
server_url=$(LC_ALL=C sort -u "${remote_urls}")
[[ $(printf '%s\n' "${server_url}" | sed '/^$/d' | wc -l | tr -d '[:space:]') == 1 &&
  ${server_url} =~ ^https?://[^[:space:]]+$ ]] ||
  fail 65 'all canonical component authorities must use one explicit HTTP(S) origin'

rows=${staging}/source-bundle-rows.jsonl
: >"${rows}"
while IFS=$'\t' read -r repo_name repository_index namespace; do
  source_identity=$(jq -cer --arg repo "${repo_name}" '
    [.components[] | select(.source_repository == $repo) |
      {snapshot: .source_snapshot, version: .version, license: .license}] |
    unique |
    if length == 1 then .[0] else error("component source identity is not unique") end
  ' "${family}")
  source_snapshot=$(jq -er '.snapshot' <<<"${source_identity}")
  component_version=$(jq -er '.version' <<<"${source_identity}")
  component_license=$(jq -er '.license' <<<"${source_identity}")
  cache_root=${staging}/${repo_name}
  bundle_root=${staging}/ait-release-source-${repo_name}
  mkdir "${bundle_root}"
  AIT_RELEASE_SERVER_URL=${server_url} \
  AIT_RELEASE_SOURCE_EVIDENCE_PATH=${bundle_root}/source-cache.evidence.json \
    "${source_cache}" \
      "${ait_bin}" "${repo_name}" "${repository_index}" "${namespace}" \
      "${source_snapshot}" "${component_version}" "${component_license}" \
      "${patch_ci}" "${cache_root}" >/dev/null
  [[ -d ${cache_root} && ! -L ${cache_root} ]] ||
    fail 65 "source cache was not produced: ${repo_name}"
  evidence=${bundle_root}/source-cache.evidence.json
  [[ -f ${evidence} && ! -L ${evidence} ]] ||
    fail 65 "source cache evidence was not produced: ${repo_name}"
  jq -e --arg repo "${repo_name}" --arg snapshot "${source_snapshot}" \
    --arg version "${component_version}" --arg license "${component_license}" '
    .contract == "ait.release.source-cache/v1" and .status == "ready" and
    .repo_name == $repo and .source_snapshot == $snapshot and
    .version == $version and .license == $license and
    .workspace_clean == true and .remote_coordinates_embedded == false and
    .public_publish == false
  ' "${evidence}" >/dev/null ||
    fail 65 "source cache evidence differs from the selected family: ${repo_name}"
  if [[ ${repo_name} == ait-core ]]; then
    validate_selected_core_authority "${cache_root}"
  fi
  COPYFILE_DISABLE=1 tar -czf "${bundle_root}/source-cache.tar.gz" -C "${cache_root}" .
  rm -rf -- "${cache_root}"
  jq -cn \
    --arg repo_name "${repo_name}" \
    --argjson repository_index "${repository_index}" \
    --arg snapshot "${source_snapshot}" \
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
done < <(jq -er '.repositories | sort_by(.repository_index)[] |
  [.repo_name, (.repository_index | tostring), .namespace] | @tsv' "${authorities}")

jq -S -n \
  --arg family_version "${family_version}" \
  --arg family_tag "$(jq -er '.family.tag' "${family}")" \
  --arg family_manifest_sha256 "$(sha256_file "${family}")" \
  --arg canonical_family_manifest_sha256 "$(sha256_file "${canonical_family}")" \
  --arg authority_evidence_sha256 \
    "$(sha256_file "${staging}/canonical-authority.evidence.json")" \
  --argjson qualification_family_used "${qualification_family_used}" \
  --argjson bundles "$(jq -s 'sort_by(.repository_index)' "${rows}")" '
  {
    contract: "ait.release.source-bundles/v1",
    status: "ready",
    family_version: $family_version,
    family_tag: $family_tag,
    family_manifest_sha256: $family_manifest_sha256,
    canonical_family_manifest_sha256: $canonical_family_manifest_sha256,
    qualification_family_manifest_sha256:
      (if $qualification_family_used then $family_manifest_sha256 else null end),
    qualification_family_used: $qualification_family_used,
    canonical_authority_evidence_sha256: $authority_evidence_sha256,
    bundles: $bundles,
    source_bundle_count: ($bundles | length),
    selected_core_version_authority_verified: true,
    recovery_authority_used: false,
    artifact_rebuild: false,
    registry_write: false,
    public_publish: false
  }
' >"${staging}/source-bundles.evidence.json"
rm -- "${remote_urls}" "${rows}"
mv "${staging}" "${output}"
trap - EXIT HUP INT TERM
printf '%s\n' "${output}/source-bundles.evidence.json"
