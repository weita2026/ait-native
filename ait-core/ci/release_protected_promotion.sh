#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf '%s\n' \
    'usage: release_protected_promotion.sh <dossier-root> <public-source-root> <evidence-output>' >&2
  exit 64
fi

dossier_root=$1
public_source_root=$2
evidence_output=$3
control_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

required_environment=(
  AIT_RELEASE_AUTHORIZATION_REF
  AIT_RELEASE_AUTHORIZATION_RUN_ATTEMPT
  AIT_RELEASE_AUTHORIZATION_RUN_ID
  AIT_RELEASE_AUTHORIZATION_SHA
  AIT_RELEASE_CHECKSUM_SHA256
  AIT_RELEASE_CHANNEL
  AIT_RELEASE_COORDINATOR_SNAPSHOT
  AIT_RELEASE_DOSSIER_ARTIFACT_DIGEST
  AIT_RELEASE_DOSSIER_ARTIFACT_ID
  AIT_RELEASE_FROZEN_MANIFEST_SHA256
  AIT_RELEASE_GIT_COMMIT
  AIT_RELEASE_ID
  AIT_RELEASE_PROTECTED_ENVIRONMENT
  AIT_RELEASE_REPOSITORY
  AIT_RELEASE_SOURCE_CONTROL_SHA
  AIT_RELEASE_SOURCE_RUN_ATTEMPT
  AIT_RELEASE_SOURCE_RUN_ID
  AIT_RELEASE_TAG
)
for variable in "${required_environment[@]}"; do
  if [[ -z ${!variable:-} ]]; then
    printf 'required protected-promotion environment is missing: %s\n' "${variable}" >&2
    exit 64
  fi
done

if [[ ${AIT_RELEASE_CHANNEL} != rc && ${AIT_RELEASE_CHANNEL} != stable ]]; then
  printf 'protected-promotion channel must be rc or stable\n' >&2
  exit 64
fi
expected_environment=${AIT_RELEASE_CHANNEL}-promotion
if [[ ${AIT_RELEASE_REPOSITORY} != weita2026/ait-native ||
  ${AIT_RELEASE_PROTECTED_ENVIRONMENT} != "${expected_environment}" ||
  ! ${AIT_RELEASE_ID} =~ ^REL-FAM-[0-9A-F]{16}$ ||
  ! ${AIT_RELEASE_GIT_COMMIT} =~ ^[0-9a-f]{40}$ ||
  ! ${AIT_RELEASE_COORDINATOR_SNAPSHOT} =~ ^SNP-[0-9A-F]{12}$ ||
  ! ${AIT_RELEASE_FROZEN_MANIFEST_SHA256} =~ ^[0-9a-f]{64}$ ||
  ! ${AIT_RELEASE_CHECKSUM_SHA256} =~ ^[0-9a-f]{64}$ ||
  ! ${AIT_RELEASE_DOSSIER_ARTIFACT_DIGEST} =~ ^sha256:[0-9a-f]{64}$ ||
  ! ${AIT_RELEASE_AUTHORIZATION_SHA} =~ ^[0-9a-f]{40}$ ||
  ! ${AIT_RELEASE_SOURCE_CONTROL_SHA} =~ ^[0-9a-f]{40}$ ||
  ! ${AIT_RELEASE_SOURCE_RUN_ID} =~ ^[1-9][0-9]*$ ||
  ! ${AIT_RELEASE_SOURCE_RUN_ATTEMPT} =~ ^[1-9][0-9]*$ ||
  ! ${AIT_RELEASE_DOSSIER_ARTIFACT_ID} =~ ^[1-9][0-9]*$ ||
  ! ${AIT_RELEASE_AUTHORIZATION_RUN_ID} =~ ^[1-9][0-9]*$ ||
  ! ${AIT_RELEASE_AUTHORIZATION_RUN_ATTEMPT} =~ ^[1-9][0-9]*$ ]]; then
  printf 'protected-promotion identity input is invalid\n' >&2
  exit 64
fi
case "${AIT_RELEASE_CHANNEL}" in
  rc)
    [[ ${AIT_RELEASE_TAG} =~ ^v[0-9]+\.[0-9]+\.[0-9]+-rc\.[1-9][0-9]*$ ]] || {
      printf 'protected-promotion RC tag is invalid\n' >&2
      exit 64
    }
    ;;
  stable)
    [[ ${AIT_RELEASE_TAG} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
      printf 'protected-promotion stable tag is invalid\n' >&2
      exit 64
    }
    ;;
esac

for command in cargo diff find git jq node rustup tar; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf 'required protected-promotion command is unavailable: %s\n' "${command}" >&2
    exit 69
  fi
done

sha256_file() {
  local path=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${path}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${path}" | awk '{print $1}'
  else
    printf 'no SHA-256 utility is available\n' >&2
    return 69
  fi
}

require_real_directory() {
  local path=$1
  local label=$2
  if [[ ${path} != /* || ! -d ${path} || -L ${path} ]]; then
    printf '%s must be an absolute real directory: %s\n' "${label}" "${path}" >&2
    return 66
  fi
}

require_regular_file() {
  local path=$1
  local label=$2
  if [[ ! -f ${path} || -L ${path} ]]; then
    printf '%s must be a regular non-symlink file: %s\n' "${label}" "${path}" >&2
    return 66
  fi
}

require_real_directory "${dossier_root}" 'family dossier root'
require_real_directory "${public_source_root}" 'public source root'
if [[ ${evidence_output} != /* || -e ${evidence_output} || -L ${evidence_output} ]]; then
  printf 'protected-promotion evidence output must be a new absolute path\n' >&2
  exit 73
fi
evidence_parent=$(dirname -- "${evidence_output}")
require_real_directory "${evidence_parent}" 'protected-promotion evidence parent'

dossier_root=$(cd "${dossier_root}" && pwd -P)
public_source_root=$(cd "${public_source_root}" && pwd -P)
evidence_parent=$(cd "${evidence_parent}" && pwd -P)
evidence_output=${evidence_parent}/$(basename -- "${evidence_output}")

if find "${dossier_root}" \
  \( -type l -o \( ! -type f -a ! -type d \) \) -print -quit | grep -q .; then
  printf 'family dossier contains a symlink or special file\n' >&2
  exit 65
fi

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-protected-promotion.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-protected-promotion.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected protected-promotion path: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

expected_top=${temporary_root}/expected-top
actual_top=${temporary_root}/actual-top
printf '%s\n' \
  ait-monorepo-source.json \
  ait-native-source-tree.tar.gz \
  ait-public-git-source.evidence.json \
  ait-release.build.json \
  ait-release.candidate.json \
  ait-release.check.json \
  ait-release.promotion.json \
  frozen \
  packages | LC_ALL=C sort >"${expected_top}"
find "${dossier_root}" -mindepth 1 -maxdepth 1 -exec basename {} \; |
  LC_ALL=C sort >"${actual_top}"
if ! diff -u "${expected_top}" "${actual_top}"; then
  printf 'family dossier top-level inventory is not exact\n' >&2
  exit 65
fi

candidate=${dossier_root}/ait-release.candidate.json
check=${dossier_root}/ait-release.check.json
build=${dossier_root}/ait-release.build.json
promotion=${dossier_root}/ait-release.promotion.json
source_mapping=${dossier_root}/ait-monorepo-source.json
source_evidence=${dossier_root}/ait-public-git-source.evidence.json
source_archive=${dossier_root}/ait-native-source-tree.tar.gz
frozen_root=${dossier_root}/frozen
frozen_manifest=${frozen_root}/ait-release-family.manifest.json
frozen_checksums=${frozen_root}/SHA256SUMS
packages_root=${dossier_root}/packages
for required in \
  "${candidate}" "${check}" "${build}" "${promotion}" \
  "${source_mapping}" "${source_evidence}" "${source_archive}" \
  "${frozen_manifest}" "${frozen_checksums}" \
  "${frozen_root}/ait-release.build.json"; do
  require_regular_file "${required}" 'protected-promotion input'
done
require_real_directory "${frozen_root}" 'frozen family root'
require_real_directory "${packages_root}" 'assembled package root'

actual_frozen_manifest_sha256=$(sha256_file "${frozen_manifest}")
actual_checksum_sha256=$(sha256_file "${frozen_checksums}")
if [[ ${actual_frozen_manifest_sha256} != "${AIT_RELEASE_FROZEN_MANIFEST_SHA256}" ||
  ${actual_checksum_sha256} != "${AIT_RELEASE_CHECKSUM_SHA256}" ]]; then
  printf 'approved frozen family digest does not match the downloaded dossier\n' >&2
  exit 65
fi
if ! cmp "${build}" "${frozen_root}/ait-release.build.json"; then
  printf 'top-level and frozen family build receipts differ\n' >&2
  exit 65
fi

version=${AIT_RELEASE_TAG#v}
if [[ ${AIT_RELEASE_CHANNEL} == rc ]]; then
  github_prerelease=true
  npm_dist_tag=rc
  pypi_prerelease=true
  oci_moving_tag=rc
  homebrew_channel=rc
  stable_formula_mutation=false
  apt_suite=testing
  winget_route=validation
  winget_community_submission=false
else
  github_prerelease=false
  npm_dist_tag=latest
  pypi_prerelease=false
  oci_moving_tag=latest
  homebrew_channel=stable
  stable_formula_mutation=true
  apt_suite=stable
  winget_route=community
  winget_community_submission=true
fi
family_manifest_sha256=$(sha256_file "${public_source_root}/ait-release-family.json")
mapping_sha256=$(sha256_file "${source_mapping}")
promotion_sha256=$(sha256_file "${promotion}")
source_archive_sha256=$(sha256_file "${source_archive}")

receipt_matrix_filter=${control_root}/ci/release_receipt_matrix.jq
platform_contract=${control_root}/ci/native_bootstrap_matrix.json
repository_contract=${control_root}/ci/release_repository_authorities.json
for matrix_input in \
  "${public_source_root}/ait-release-family.json" \
  "${receipt_matrix_filter}" \
  "${platform_contract}" \
  "${repository_contract}"; do
  require_regular_file "${matrix_input}" 'tagged receipt-matrix input'
done
receipt_matrix=${temporary_root}/receipt-matrix.json
jq -n \
  --slurpfile family "${public_source_root}/ait-release-family.json" \
  --slurpfile platforms "${platform_contract}" \
  --slurpfile authorities "${repository_contract}" \
  -f "${receipt_matrix_filter}" >"${receipt_matrix}"
expected_source_count=$(jq -er '
  .expected_source_count |
  select(type == "number" and . > 0 and . == floor)
' "${receipt_matrix}")
expected_receipt_count=$(jq -er '
  .expected_receipt_count |
  select(type == "number" and . > 0 and . == floor)
' "${receipt_matrix}")
expected_component_artifact_count=$(jq -er '
  .expected_component_artifact_count |
  select(type == "number" and . > 0 and . == floor)
' "${receipt_matrix}")
expected_license_material_count=$((expected_source_count * 2))

if ! jq -e \
  --arg release_id "${AIT_RELEASE_ID}" \
  --arg version "${version}" \
  --arg channel "${AIT_RELEASE_CHANNEL}" \
  --arg tag "${AIT_RELEASE_TAG}" \
  --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
  --arg family_sha "${family_manifest_sha256}" \
  --arg repository "${AIT_RELEASE_REPOSITORY}" '
    .contract == "ait.release.family.candidate/v1" and
    .release_id == $release_id and .version == $version and
    .channel == $channel and .tag == $tag and .snapshot_id == $snapshot and
    .profile == "family" and .family_manifest_sha256 == $family_sha and
    .family.public_source.identity == $repository and
    .authority.local_release_authority == "not_activated" and
    .authority.remote_release_authority == "not_activated"
  ' "${candidate}" >/dev/null; then
  printf 'family candidate identity or authority is invalid\n' >&2
  exit 65
fi

for record in "${check}" "${build}" "${promotion}"; do
  if ! jq -e \
    --arg release_id "${AIT_RELEASE_ID}" \
    --arg version "${version}" \
    --arg channel "${AIT_RELEASE_CHANNEL}" \
    --arg tag "${AIT_RELEASE_TAG}" \
    --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
    --arg family_sha "${family_manifest_sha256}" '
      .release_id == $release_id and .version == $version and
      .channel == $channel and .tag == $tag and .snapshot_id == $snapshot and
      .profile == "family" and .family_manifest_sha256 == $family_sha
    ' "${record}" >/dev/null; then
    printf 'family record identity differs from the approved candidate: %s\n' \
      "$(basename -- "${record}")" >&2
    exit 65
  fi
done
if ! jq -e \
  --arg source_commit "${AIT_RELEASE_GIT_COMMIT}" \
  --argjson expected_receipt_count "${expected_receipt_count}" \
  --argjson expected_component_artifact_count \
    "${expected_component_artifact_count}" \
  --argjson expected_license_material_count \
    "${expected_license_material_count}" '
  .contract == "ait.release.family.check/v1" and
  .status == "checked" and .check_summary.decision == "pass" and
  .check_summary.failed == 0 and .check_summary.blocking == 0 and
  (.component_receipts | length) == $expected_receipt_count and
  ([.component_receipts[].git_commit] | unique) == [$source_commit] and
  ([.artifacts[] | select(.role == "component-artifact")] | length) ==
    $expected_component_artifact_count and
  (.license_material | length) == $expected_license_material_count
' "${check}" >/dev/null; then
  printf 'family check receipt is not a complete passing receipt matrix\n' >&2
  exit 65
fi
if ! jq -e \
  --arg source_commit "${AIT_RELEASE_GIT_COMMIT}" \
  --argjson expected_receipt_count "${expected_receipt_count}" \
  --argjson expected_component_artifact_count \
    "${expected_component_artifact_count}" \
  --argjson expected_license_material_count \
    "${expected_license_material_count}" '
  .contract == "ait.release.family.build/v1" and
  .status == "built" and .check_summary.decision == "pass" and
  .promotion.authorized == false and .promotion.performed == false and
  .promotion.registry_write == false and
  (.component_receipts | length) == $expected_receipt_count and
  ([.component_receipts[].git_commit] | unique) == [$source_commit] and
  ([.artifacts[] | select(.role == "component-artifact")] | length) ==
    $expected_component_artifact_count and
  ([.artifacts[] | select(.role == "license-material")] | length) ==
    $expected_license_material_count
' "${build}" >/dev/null; then
  printf 'family build receipt is not the exact unpromoted frozen build\n' >&2
  exit 65
fi
if ! jq -e \
  --arg tag "${AIT_RELEASE_TAG}" \
  --arg version "${version}" \
  --arg npm_dist_tag "${npm_dist_tag}" \
  --arg oci_moving_tag "${oci_moving_tag}" \
  --arg homebrew_channel "${homebrew_channel}" \
  --arg apt_suite "${apt_suite}" \
  --arg winget_route "${winget_route}" \
  --argjson github_prerelease "${github_prerelease}" \
  --argjson pypi_prerelease "${pypi_prerelease}" \
  --argjson stable_formula_mutation "${stable_formula_mutation}" \
  --argjson winget_community_submission "${winget_community_submission}" '
  .contract == "ait.release.family.promotion/v1" and
  .status == "ready_for_protected_ci" and
  .authorization.required == true and .authorization.granted == false and
  .authorization.protected_environment_required == true and
  .authorization.public_source_readback_required == true and
  .authorization.snapshot_to_git_tree_equality_required == true and
  .source_publication.status == "required_unverified" and
  .source_publication.binary_publication_allowed == false and
  .mutation.credentials_loaded == false and .mutation.performed == false and
  .mutation.rebuild_allowed == false and .mutation.registry_write == false and
  .routes.github == {draft: false, prerelease: $github_prerelease, tag: $tag} and
  .routes.npm == {dist_tag: $npm_dist_tag, version: $version} and
  .routes.pypi == {prerelease: $pypi_prerelease, repository: "pypi"} and
  .routes.oci == {moving_tag: $oci_moving_tag, version_tag: $version} and
  .routes.homebrew == {
    channel: $homebrew_channel,
    stable_formula_mutation: $stable_formula_mutation
  } and
  .routes.apt == {suite: $apt_suite} and
  .routes.winget == {
    community_manifest_submission: $winget_community_submission,
    route: $winget_route
  } and
  .next_action.code == "approve_exact_frozen_digest"
' "${promotion}" >/dev/null; then
  printf 'family promotion handoff is not awaiting protected exact-digest approval\n' >&2
  exit 65
fi
if ! jq -e \
  --arg release_id "${AIT_RELEASE_ID}" \
  --arg version "${version}" \
  --arg channel "${AIT_RELEASE_CHANNEL}" \
  --arg tag "${AIT_RELEASE_TAG}" \
  --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
  --arg family_sha "${family_manifest_sha256}" \
  --arg npm_dist_tag "${npm_dist_tag}" \
  --arg oci_moving_tag "${oci_moving_tag}" \
  --arg homebrew_channel "${homebrew_channel}" \
  --arg apt_suite "${apt_suite}" \
  --arg winget_route "${winget_route}" \
  --argjson github_prerelease "${github_prerelease}" \
  --argjson pypi_prerelease "${pypi_prerelease}" \
  --argjson stable_formula_mutation "${stable_formula_mutation}" \
  --argjson winget_community_submission "${winget_community_submission}" '
    .contract == "ait.release.family.frozen/v1" and
    .release_id == $release_id and .version == $version and
    .channel == $channel and .tag == $tag and .snapshot_id == $snapshot and
    .family_manifest_sha256 == $family_sha and
    .promotion.authorized == false and .promotion.performed == false and
    .promotion.registry_write == false and
    .promotion.routes.github == {
      draft: false,
      prerelease: $github_prerelease,
      tag: $tag
    } and
    .promotion.routes.npm == {dist_tag: $npm_dist_tag, version: $version} and
    .promotion.routes.pypi == {
      prerelease: $pypi_prerelease,
      repository: "pypi"
    } and
    .promotion.routes.oci == {
      moving_tag: $oci_moving_tag,
      version_tag: $version
    } and
    .promotion.routes.apt == {suite: $apt_suite} and
    .promotion.routes.homebrew == {
      channel: $homebrew_channel,
      stable_formula_mutation: $stable_formula_mutation
    } and
    .promotion.routes.winget == {
      community_manifest_submission: $winget_community_submission,
      route: $winget_route
    }
  ' "${frozen_manifest}" >/dev/null; then
  printf 'frozen family manifest identity or mutation state is invalid\n' >&2
  exit 65
fi
if ! jq -e \
  --arg repository "${AIT_RELEASE_REPOSITORY}" \
  --arg commit "${AIT_RELEASE_GIT_COMMIT}" \
  --arg control_commit "${AIT_RELEASE_SOURCE_CONTROL_SHA}" \
  --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
  --arg mapping_sha "${mapping_sha256}" '
    .contract == "ait.release.public-git-source/v1" and .status == "ready" and
    .public_source_identity == $repository and .git_commit == $commit and
    .workflow_control_commit == $control_commit and
    .coordinator_snapshot == $snapshot and .mapping_sha256 == $mapping_sha and
    .source_cache_count == 0 and .registry_write == false and
    .public_publish == false
  ' "${source_evidence}" >/dev/null; then
  printf 'public Git source evidence differs from the approved source authority\n' >&2
  exit 65
fi
if ! jq -e \
  --arg repository "${AIT_RELEASE_REPOSITORY}" \
  --arg tag "${AIT_RELEASE_TAG}" \
  --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
  --arg family_sha "${family_manifest_sha256}" '
    .schema == "ait.release.monorepo-source/v1" and
    .public_source_identity == $repository and .family_tag == $tag and
    .coordinator_snapshot == $snapshot and
    .family_manifest_sha256 == $family_sha and
    (.subtrees | length) == 5 and .git_commit_created == false and
    .public_publish == false
  ' "${source_mapping}" >/dev/null; then
  printf 'public monorepo source mapping differs from the approved family\n' >&2
  exit 65
fi

verify_checksum_manifest() {
  local root=$1
  local manifest=$2
  local label=$3
  local paths=${temporary_root}/${label}.paths
  local digest relative line actual count=0
  : >"${paths}"
  while IFS= read -r line || [[ -n ${line} ]]; do
    if [[ ! ${line} =~ ^([0-9a-f]{64})[[:space:]][[:space:]](.+)$ ]]; then
      printf '%s checksum line is malformed\n' "${label}" >&2
      return 65
    fi
    digest=${BASH_REMATCH[1]}
    relative=${BASH_REMATCH[2]}
    case "/${relative}/" in
      *'//'*)
        printf '%s checksum path is empty or repeated: %s\n' "${label}" "${relative}" >&2
        return 65
        ;;
      *'/../'*|*'/./'*)
        printf '%s checksum path is not normalized: %s\n' "${label}" "${relative}" >&2
        return 65
        ;;
    esac
    if [[ ${relative} == /* || ${relative} == *\\* || ${relative} == *$'\t'* ||
      ${relative} == *$'\r'* ]]; then
      printf '%s checksum path is unsafe: %s\n' "${label}" "${relative}" >&2
      return 65
    fi
    require_regular_file "${root}/${relative}" "${label} checksum member"
    actual=$(sha256_file "${root}/${relative}")
    if [[ ${actual} != "${digest}" ]]; then
      printf '%s checksum member differs: %s\n' "${label}" "${relative}" >&2
      return 65
    fi
    printf '%s\n' "${relative}" >>"${paths}"
    count=$((count + 1))
  done <"${manifest}"
  if [[ ${count} -eq 0 || -n $(LC_ALL=C sort "${paths}" | uniq -d) ]]; then
    printf '%s checksum inventory is empty or duplicated\n' "${label}" >&2
    return 65
  fi
  printf '%s\n' "${count}"
}

frozen_checksum_count=$(verify_checksum_manifest \
  "${frozen_root}" "${frozen_checksums}" frozen)
build_artifact_count=$(jq -er '.artifacts | length' "${build}")
frozen_file_count=$(find "${frozen_root}" -type f | wc -l | tr -d '[:space:]')
if [[ ${frozen_checksum_count} -ne $((build_artifact_count - 1)) ]]; then
  printf 'frozen checksum coverage differs from the build artifact inventory\n' >&2
  exit 65
fi
if [[ ${frozen_file_count} -ne $((frozen_checksum_count + 2)) ]]; then
  printf 'frozen family root contains an unrecorded file\n' >&2
  exit 65
fi

expected_channels=${temporary_root}/expected-channels
actual_channels=${temporary_root}/actual-channels
printf '%s\n' apt homebrew npm pypi winget | LC_ALL=C sort >"${expected_channels}"
find "${packages_root}" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; |
  LC_ALL=C sort >"${actual_channels}"
if find "${packages_root}" -mindepth 1 -maxdepth 1 ! -type d -print -quit |
  grep -q .; then
  printf 'assembled package root contains a non-channel entry\n' >&2
  exit 65
fi
if ! diff -u "${expected_channels}" "${actual_channels}"; then
  printf 'assembled package channel inventory is not exact\n' >&2
  exit 65
fi

package_rows=${temporary_root}/package-rows.jsonl
: >"${package_rows}"
for channel in apt homebrew npm pypi winget; do
  channel_root=${packages_root}/${channel}
  package_receipt=${channel_root}/ait-release.package.json
  package_checksums=${channel_root}/SHA256SUMS
  require_regular_file "${package_receipt}" "${channel} package receipt"
  require_regular_file "${package_checksums}" "${channel} package checksum manifest"
  if ! jq -e \
    --arg release_id "${AIT_RELEASE_ID}" \
    --arg version "${version}" \
    --arg release_channel "${AIT_RELEASE_CHANNEL}" \
    --arg tag "${AIT_RELEASE_TAG}" \
    --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
    --arg channel "${channel}" \
    --arg frozen_manifest_sha "${AIT_RELEASE_FROZEN_MANIFEST_SHA256}" \
    --arg frozen_checksum_sha "${AIT_RELEASE_CHECKSUM_SHA256}" '
      .contract == "ait.release.family.package/v1" and
      .release_id == $release_id and .version == $version and
      .release_channel == $release_channel and .tag == $tag and
      .snapshot_id == $snapshot and
      .channel == $channel and .status == "assembled" and
      .frozen_manifest_sha256 == $frozen_manifest_sha and
      .frozen_checksum_sha256 == $frozen_checksum_sha and
      .check_summary.decision == "pass" and .check_summary.failed == 0 and
      .check_summary.blocking == 0 and .artifact_count == (.artifacts | length) and
      ([.mutation[]] | all(. == false))
    ' "${package_receipt}" >/dev/null; then
    printf '%s package receipt identity, checksum binding, or mutation state is invalid\n' \
      "${channel}" >&2
    exit 65
  fi
  package_checksum_count=$(verify_checksum_manifest \
    "${channel_root}" "${package_checksums}" "package-${channel}")
  package_artifact_count=$(jq -er '.artifact_count' "${package_receipt}")
  package_file_count=$(find "${channel_root}" -type f | wc -l | tr -d '[:space:]')
  if [[ ${package_checksum_count} -ne $((package_artifact_count + 1)) ]]; then
    printf '%s package checksum coverage is incomplete\n' "${channel}" >&2
    exit 65
  fi
  if [[ ${package_file_count} -ne $((package_checksum_count + 1)) ]]; then
    printf '%s package root contains an unrecorded file\n' "${channel}" >&2
    exit 65
  fi
  jq -cn \
    --arg channel "${channel}" \
    --arg receipt_sha256 "$(sha256_file "${package_receipt}")" \
    --arg checksum_sha256 "$(sha256_file "${package_checksums}")" \
    --argjson artifact_count "${package_artifact_count}" '
      {
        channel: $channel,
        artifact_count: $artifact_count,
        receipt_sha256: $receipt_sha256,
        checksum_sha256: $checksum_sha256
      }
    ' >>"${package_rows}"
done

archive_members=${temporary_root}/source-archive.members
archive_verbose=${temporary_root}/source-archive.verbose
tar -tzf "${source_archive}" >"${archive_members}"
while IFS= read -r member; do
  [[ ${member} == . || ${member} == ./ ]] && continue
  normalized=${member#./}
  if [[ -z ${normalized} || ${normalized} == /* || ${normalized} == .. ||
    ${normalized} == ../* || ${normalized} == */../* || ${normalized} == */.. ||
    ${normalized} == *\\* || ${normalized} == *$'\r'* || ${normalized} == *$'\t'* ]]; then
    printf 'corresponding-source archive contains an unsafe member: %s\n' "${member}" >&2
    exit 65
  fi
done <"${archive_members}"
tar -tvzf "${source_archive}" >"${archive_verbose}"
while IFS= read -r line; do
  entry_type=${line:0:1}
  if [[ ${entry_type} != - && ${entry_type} != d ]]; then
    printf 'corresponding-source archive contains a link or special member\n' >&2
    exit 65
  fi
done <"${archive_verbose}"
archive_root=${temporary_root}/source-archive
mkdir "${archive_root}"
tar -xzf "${source_archive}" -C "${archive_root}"
if find "${archive_root}" \
  \( -type l -o \( ! -type f -a ! -type d \) \) -print -quit | grep -q .; then
  printf 'extracted corresponding source contains a link or special file\n' >&2
  exit 65
fi
if ! diff -qr -x .git "${archive_root}" "${public_source_root}"; then
  printf 'tagged public source bytes differ from archived corresponding source\n' >&2
  exit 65
fi

expected_executables=${temporary_root}/expected-executables
archive_executables=${temporary_root}/archive-executables
git -C "${public_source_root}" ls-files --stage |
  awk '$1 == "100755" {sub(/^[^\t]+\t/, ""); print}' |
  LC_ALL=C sort >"${expected_executables}"
find "${archive_root}" -type f -perm -111 -print |
  sed "s#^${archive_root}/##" | LC_ALL=C sort >"${archive_executables}"
if ! diff -u "${expected_executables}" "${archive_executables}"; then
  printf 'tagged public executable modes differ from archived corresponding source\n' >&2
  exit 65
fi

public_head=$(git -C "${public_source_root}" rev-parse HEAD)
public_tag_head=$(git -C "${public_source_root}" \
  rev-list -n 1 "refs/tags/${AIT_RELEASE_TAG}")
public_status=$(git -C "${public_source_root}" status --porcelain --untracked-files=all)
if [[ ${public_head} != "${AIT_RELEASE_GIT_COMMIT}" ||
  ${public_tag_head} != "${AIT_RELEASE_GIT_COMMIT}" || -n ${public_status} ]]; then
  printf 'public tag checkout does not match the approved Git commit\n' >&2
  exit 65
fi
for required_build_input in \
  build-release.sh build-release.ps1 build-release.mjs \
  CONTRIBUTING.md SECURITY.md \
  ait-core/rust/Cargo.lock ait-server/rust/Cargo.lock ait-runner/Cargo.lock \
  ait-python/pyproject.toml ait-node/package.json \
  docs/distribution.md ait-release-family.json ait-monorepo-source.json; do
  require_regular_file "${public_source_root}/${required_build_input}" \
    'tagged public build input'
done
node "${public_source_root}/build-release.mjs" \
  --validate-only --git-commit "${AIT_RELEASE_GIT_COMMIT}" >/dev/null

case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    native_target=aarch64-apple-darwin
    ;;
  Darwin:x86_64)
    native_target=x86_64-apple-darwin
    ;;
  Linux:aarch64|Linux:arm64)
    native_target=aarch64-unknown-linux-gnu
    ;;
  Linux:x86_64|Linux:amd64)
    native_target=x86_64-unknown-linux-gnu
    ;;
  *)
    printf 'protected-promotion verifier does not support this native host\n' >&2
    exit 69
    ;;
esac
frozen_ait=${frozen_root}/artifacts/ait/native-executable/${native_target}/ait-cli
require_regular_file "${frozen_ait}" 'frozen native AIT verifier'
frozen_ait_bin=${temporary_root}/frozen-ait
cp "${frozen_ait}" "${frozen_ait_bin}"
chmod 0755 "${frozen_ait_bin}"
if [[ $("${frozen_ait_bin}" --version) != "ait ${version}" ]]; then
  printf 'frozen native AIT version differs from the approved family\n' >&2
  exit 65
fi

admission_rust=${public_source_root}/ait-core/rust
family_packages=${admission_rust}/crates/ait-cli/src/release_surface/family_packages.rs
family_release=${admission_rust}/crates/ait-cli/src/release_surface/family_release.rs
admission_cargo_lock=${admission_rust}/Cargo.lock
require_regular_file "${family_packages}" 'tagged family package admission source'
require_regular_file "${family_release}" 'tagged family release admission source'
require_regular_file "${admission_cargo_lock}" 'tagged family admission Cargo lock'
family_packages_sha256=$(sha256_file "${family_packages}")
family_release_sha256=$(sha256_file "${family_release}")
cargo_lock_sha256=$(sha256_file "${admission_cargo_lock}")
rust_toolchain=$(jq -er '
  .rust_toolchain | select(type == "string" and test("^[0-9]+\\.[0-9]+\\.[0-9]+$"))
' "${platform_contract}")
rustup toolchain install "${rust_toolchain}" --profile minimal >/dev/null
admission_target=${temporary_root}/family-admission-target
CARGO_BUILD_BUILD_DIR=${temporary_root}/family-admission-build \
  rustup run "${rust_toolchain}" cargo build \
    --locked \
    --release \
    --target-dir "${admission_target}" \
    --manifest-path "${admission_rust}/Cargo.toml" \
    -p ait-cli \
    --bin ait-cli
ait_bin=${admission_target}/release/ait-cli
if [[ ! -x ${ait_bin} || $("${ait_bin}" --version) != "ait ${version}" ]]; then
  printf 'protected family admission CLI is unavailable or has the wrong version\n' >&2
  exit 65
fi
if [[ -n $(git -C "${public_source_root}" status --porcelain --untracked-files=all) ||
  $(sha256_file "${family_packages}") != "${family_packages_sha256}" ||
  $(sha256_file "${family_release}") != "${family_release_sha256}" ||
  $(sha256_file "${admission_cargo_lock}") != "${cargo_lock_sha256}" ]]; then
  printf 'native admission build mutated the immutable tagged source\n' >&2
  exit 65
fi
admission_root=${temporary_root}/ait-core
mkdir "${admission_root}"
(
  cd "${admission_root}"
  "${ait_bin}" init --json >/dev/null
  "${ait_bin}" line rename main release-bootstrap --json >/dev/null
  release_root=${admission_root}/dist/${AIT_RELEASE_ID}
  mkdir -p "${release_root}"
  cp "${candidate}" "${release_root}/ait-release.candidate.json"
  cp "${check}" "${release_root}/ait-release.check.json"
  cp "${build}" "${release_root}/ait-release.build.json"
  cp "${promotion}" "${release_root}/ait-release.promotion.json"
  cp -R "${frozen_root}" "${release_root}/frozen"
  "${ait_bin}" release promote "${AIT_RELEASE_ID}" \
    --channel "${AIT_RELEASE_CHANNEL}" \
    --public-source-root "${public_source_root}" --json \
    >"${temporary_root}/promotion-readback.json"
)
jq -S . "${promotion}" >"${temporary_root}/promotion-expected.sorted.json"
jq -S . "${temporary_root}/promotion-readback.json" \
  >"${temporary_root}/promotion-readback.sorted.json"
if ! diff -u "${temporary_root}/promotion-expected.sorted.json" \
  "${temporary_root}/promotion-readback.sorted.json"; then
  printf 'frozen native AIT did not reproduce the immutable promotion handoff\n' >&2
  exit 65
fi

packages=$(jq -s 'sort_by(.channel)' "${package_rows}")
source_content_sha256=$(jq -er '.content_sha256' "${source_mapping}")
source_evidence_sha256=$(sha256_file "${source_evidence}")
candidate_sha256=$(sha256_file "${candidate}")
check_sha256=$(sha256_file "${check}")
build_sha256=$(sha256_file "${build}")
executable_mode_count=$(wc -l <"${expected_executables}" | tr -d '[:space:]')
source_file_count=$(find "${archive_root}" -type f | wc -l | tr -d '[:space:]')

jq -n \
  --arg release_id "${AIT_RELEASE_ID}" \
  --arg version "${version}" \
  --arg channel "${AIT_RELEASE_CHANNEL}" \
  --arg tag "${AIT_RELEASE_TAG}" \
  --arg repository "${AIT_RELEASE_REPOSITORY}" \
  --arg commit "${AIT_RELEASE_GIT_COMMIT}" \
  --arg snapshot "${AIT_RELEASE_COORDINATOR_SNAPSHOT}" \
  --arg source_run_id "${AIT_RELEASE_SOURCE_RUN_ID}" \
  --arg source_run_attempt "${AIT_RELEASE_SOURCE_RUN_ATTEMPT}" \
  --arg source_control_sha "${AIT_RELEASE_SOURCE_CONTROL_SHA}" \
  --arg dossier_artifact_id "${AIT_RELEASE_DOSSIER_ARTIFACT_ID}" \
  --arg dossier_artifact_digest "${AIT_RELEASE_DOSSIER_ARTIFACT_DIGEST}" \
  --arg environment "${AIT_RELEASE_PROTECTED_ENVIRONMENT}" \
  --arg authorization_run_id "${AIT_RELEASE_AUTHORIZATION_RUN_ID}" \
  --arg authorization_run_attempt "${AIT_RELEASE_AUTHORIZATION_RUN_ATTEMPT}" \
  --arg authorization_ref "${AIT_RELEASE_AUTHORIZATION_REF}" \
  --arg authorization_sha "${AIT_RELEASE_AUTHORIZATION_SHA}" \
  --arg frozen_manifest_sha256 "${AIT_RELEASE_FROZEN_MANIFEST_SHA256}" \
  --arg checksum_sha256 "${AIT_RELEASE_CHECKSUM_SHA256}" \
  --arg candidate_sha256 "${candidate_sha256}" \
  --arg check_sha256 "${check_sha256}" \
  --arg build_sha256 "${build_sha256}" \
  --arg promotion_sha256 "${promotion_sha256}" \
  --arg source_mapping_sha256 "${mapping_sha256}" \
  --arg source_evidence_sha256 "${source_evidence_sha256}" \
  --arg source_archive_sha256 "${source_archive_sha256}" \
  --arg source_content_sha256 "${source_content_sha256}" \
  --arg admission_rust_toolchain "${rust_toolchain}" \
  --arg cargo_lock_sha256 "${cargo_lock_sha256}" \
  --arg family_packages_sha256 "${family_packages_sha256}" \
  --arg family_release_sha256 "${family_release_sha256}" \
  --argjson source_file_count "${source_file_count}" \
  --argjson executable_mode_count "${executable_mode_count}" \
  --argjson frozen_checksum_count "${frozen_checksum_count}" \
  --argjson packages "${packages}" '
    {
      contract: "ait.release.family.protected-promotion/v1",
      status: "authorized_for_explicit_endpoint_promotion",
      release_id: $release_id,
      version: $version,
      channel: $channel,
      tag: $tag,
      snapshot_id: $snapshot,
      public_source: {
        repository: $repository,
        git_commit: $commit,
        mapping_sha256: $source_mapping_sha256,
        evidence_sha256: $source_evidence_sha256,
        archive_sha256: $source_archive_sha256,
        content_sha256: $source_content_sha256,
        file_count: $source_file_count,
        executable_mode_count: $executable_mode_count,
        anonymous_tag_readback: true,
        commit_tree_equal: true,
        archived_source_equal: true,
        locked_build_inputs_present: true,
        build_contract_valid: true,
        status: "verified"
      },
      dossier: {
        source_run_id: $source_run_id,
        source_run_attempt: $source_run_attempt,
        source_workflow_sha: $source_control_sha,
        artifact_id: $dossier_artifact_id,
        artifact_digest: $dossier_artifact_digest,
        candidate_sha256: $candidate_sha256,
        check_sha256: $check_sha256,
        build_sha256: $build_sha256,
        promotion_sha256: $promotion_sha256,
        frozen_manifest_sha256: $frozen_manifest_sha256,
        checksum_sha256: $checksum_sha256,
        frozen_checksum_count: $frozen_checksum_count,
        native_promotion_readback_equal: true,
        admission_replay: {
          model: "immutable-tag-native-admission/v1",
          rust_toolchain: $admission_rust_toolchain,
          cargo_lock_sha256: $cargo_lock_sha256,
          family_packages_sha256: $family_packages_sha256,
          family_release_sha256: $family_release_sha256
        },
        packages: $packages
      },
      authorization: {
        required: true,
        granted: true,
        exact_digest_approval: true,
        boundary: "github_protected_environment",
        protected_environment: $environment,
        workflow_run_id: $authorization_run_id,
        workflow_run_attempt: $authorization_run_attempt,
        workflow_ref: $authorization_ref,
        workflow_sha: $authorization_sha
      },
      mutation: {
        artifact_rebuild: false,
        component_rebuild: false,
        registry_credentials_loaded: false,
        registry_write: false,
        github_release_write: false,
        tag_write: false,
        ait_remote_release_activation: false,
        service_mutation: false
      },
      next_action: {
        code: "request_explicit_registry_authorization",
        detail: "Authorize each exact endpoint separately; publish and read back only these frozen bytes without rebuilding."
      }
    }
  ' >"${evidence_output}"

if ! jq -e \
  --arg release_id "${AIT_RELEASE_ID}" \
  --arg channel "${AIT_RELEASE_CHANNEL}" \
  --arg source_control_sha "${AIT_RELEASE_SOURCE_CONTROL_SHA}" \
  --arg frozen_manifest_sha256 "${AIT_RELEASE_FROZEN_MANIFEST_SHA256}" \
  --arg checksum_sha256 "${AIT_RELEASE_CHECKSUM_SHA256}" '
    .contract == "ait.release.family.protected-promotion/v1" and
    .status == "authorized_for_explicit_endpoint_promotion" and
    .release_id == $release_id and .channel == $channel and
    .authorization.granted == true and
    .dossier.source_workflow_sha == $source_control_sha and
    .dossier.frozen_manifest_sha256 == $frozen_manifest_sha256 and
    .dossier.checksum_sha256 == $checksum_sha256 and
    .dossier.native_promotion_readback_equal == true and
    .dossier.admission_replay.model ==
      "immutable-tag-native-admission/v1" and
    .public_source.status == "verified" and
    ([.mutation[]] | all(. == false)) and
    .next_action.code == "request_explicit_registry_authorization"
  ' "${evidence_output}" >/dev/null; then
  printf 'generated protected-promotion evidence failed its contract check\n' >&2
  exit 65
fi

printf '%s\n' "${evidence_output}"
