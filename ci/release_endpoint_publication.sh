#!/usr/bin/env bash
set -euo pipefail

authority_mode=protected
if [[ ${1:-} == --pre-tag ]]; then
  authority_mode=pre_tag
  shift
fi
if [[ (${authority_mode} == protected && $# -ne 4) ||
  (${authority_mode} == pre_tag && $# -ne 3) ]]; then
  printf '%s\n' \
    'usage: release_endpoint_publication.sh [--pre-tag] <authority-config> <dossier-root> [<protected-evidence>] <output-root>' >&2
  exit 64
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
endpoint_config=$1
dossier_root=$2
if [[ ${authority_mode} == protected ]]; then
  protected_evidence=$3
  output_root=$4
else
  protected_evidence=
  output_root=$3
fi

for command in awk bash basename cmp cp diff find jq mv node sed sort tar; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf 'required endpoint-publication command is unavailable: %s\n' "${command}" >&2
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

require_regular_file() {
  local path=$1
  local label=$2
  if [[ ! -f ${path} || -L ${path} ]]; then
    printf '%s must be a regular non-symlink file: %s\n' "${label}" "${path}" >&2
    return 66
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

require_regular_file "${endpoint_config}" 'endpoint configuration'
require_real_directory "${dossier_root}" 'family dossier root'
if [[ ${authority_mode} == protected ]]; then
  require_regular_file "${protected_evidence}" 'protected authorization evidence'
fi
if [[ ${output_root} != /* || -e ${output_root} || -L ${output_root} ]]; then
  printf 'endpoint-publication output must be a new absolute path: %s\n' "${output_root}" >&2
  exit 73
fi
output_parent=$(dirname -- "${output_root}")
require_real_directory "${output_parent}" 'endpoint-publication output parent'

endpoint_config=$(cd "$(dirname -- "${endpoint_config}")" && pwd -P)/$(basename -- "${endpoint_config}")
dossier_root=$(cd "${dossier_root}" && pwd -P)
if [[ ${authority_mode} == protected ]]; then
  protected_evidence=$(cd "$(dirname -- "${protected_evidence}")" && pwd -P)/$(basename -- "${protected_evidence}")
fi
output_parent=$(cd "${output_parent}" && pwd -P)
output_root=${output_parent}/$(basename -- "${output_root}")

if find "${dossier_root}" \
  \( -type l -o \( ! -type f -a ! -type d \) \) -print -quit | grep -q .; then
  printf 'family dossier contains a symlink or special file\n' >&2
  exit 65
fi

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-endpoint-publication.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-endpoint-publication.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected endpoint-publication path: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

staging=${temporary_root}/staging
assets=${staging}/assets
mkdir -p "${assets}" "${staging}/oci/ait-server" "${staging}/oci/ait-runner"

if [[ ${authority_mode} == protected ]]; then
  bash "${repo_root}/ci/release_operator.sh" validate-config \
    --config "${endpoint_config}" >/dev/null
else
  bash "${repo_root}/ci/release_operator.sh" validate-candidate-config \
    --config "${endpoint_config}" >/dev/null
fi
release_channel=$(jq -er '.release.channel' "${endpoint_config}")
release_version=$(jq -er '.release.version' "${endpoint_config}")
release_tag=$(jq -er '.release.tag' "${endpoint_config}")
release_id=$(jq -er '.release.id' "${endpoint_config}")

if [[ ${authority_mode} == protected ]]; then
  expected_protected_sha=$(jq -er '.protected_authorization.evidence_sha256' "${endpoint_config}")
  if [[ $(sha256_file "${protected_evidence}") != "${expected_protected_sha}" ]]; then
    printf 'protected authorization evidence digest is not exact\n' >&2
    exit 65
  fi
  if ! jq -e --slurpfile config "${endpoint_config}" '
  .contract == "ait.release.family.protected-promotion/v1" and
  .status == "authorized_for_explicit_endpoint_promotion" and
  .release_id == $config[0].release.id and
  .version == $config[0].release.version and
  .channel == $config[0].release.channel and
  .tag == $config[0].release.tag and
  .snapshot_id == $config[0].release.coordinator_snapshot and
  .public_source.repository == $config[0].publisher.repository and
  .public_source.git_commit == $config[0].release.source_commit and
  .public_source.status == "verified" and
  .public_source.anonymous_tag_readback == true and
  .public_source.commit_tree_equal == true and
  .public_source.archived_source_equal == true and
  .dossier.source_run_id == ($config[0].source_dossier.workflow_run_id | tostring) and
  .dossier.source_run_attempt == ($config[0].source_dossier.workflow_run_attempt | tostring) and
  .dossier.source_workflow_sha == $config[0].source_dossier.workflow_control_commit and
  .dossier.artifact_id == ($config[0].source_dossier.artifact_id | tostring) and
  .dossier.artifact_digest == $config[0].source_dossier.artifact_digest and
  .dossier.frozen_manifest_sha256 == $config[0].release.frozen_manifest_sha256 and
  .dossier.checksum_sha256 == $config[0].release.frozen_checksums_sha256 and
  (.dossier.frozen_checksum_count | type == "number" and . > 0 and floor == .) and
  .dossier.native_promotion_readback_equal == true and
  .dossier.pre_tag_admission_verified == true and
  (.dossier.pre_tag_admission_sha256 | test("^[0-9a-f]{64}$")) and
  .dossier.admission_replay == {
    model: "immutable-tag-native-admission/v1",
    rust_toolchain: .dossier.admission_replay.rust_toolchain,
    cargo_lock_sha256: .dossier.admission_replay.cargo_lock_sha256,
    family_packages_sha256: .dossier.admission_replay.family_packages_sha256,
    family_release_sha256: .dossier.admission_replay.family_release_sha256
  } and
  (.dossier.admission_replay.rust_toolchain | test("^[0-9]+\\.[0-9]+\\.[0-9]+$")) and
  (.dossier.admission_replay.cargo_lock_sha256 | test("^[0-9a-f]{64}$")) and
  (.dossier.admission_replay.family_packages_sha256 | test("^[0-9a-f]{64}$")) and
  (.dossier.admission_replay.family_release_sha256 | test("^[0-9a-f]{64}$")) and
  .authorization.required == true and
  .authorization.granted == true and
  .authorization.exact_digest_approval == true and
  .authorization.boundary == "github_protected_environment" and
  .authorization.protected_environment == ($config[0].release.channel + "-promotion") and
  .authorization.workflow_run_id == ($config[0].protected_authorization.workflow_run_id | tostring) and
  .authorization.workflow_run_attempt == ($config[0].protected_authorization.workflow_run_attempt | tostring) and
  .authorization.workflow_sha == $config[0].protected_authorization.workflow_control_commit and
  .mutation == {
    artifact_rebuild: false,
    component_rebuild: false,
    registry_credentials_loaded: false,
    registry_write: false,
    github_release_write: false,
    tag_write: false,
    ait_remote_release_activation: false,
    service_mutation: false
  }
' "${protected_evidence}" >/dev/null; then
    printf 'protected authorization evidence does not authorize this exact release\n' >&2
    exit 65
  fi
fi

expected_top=${temporary_root}/expected-top
actual_top=${temporary_root}/actual-top
printf '%s\n' \
  ait-monorepo-source.json \
  ait-native-source-tree.tar.gz \
  ait-public-git-source.evidence.json \
  ait-release.build.json \
  ait-release.candidate.json \
  ait-release.check.json \
  ait-release.pre-tag-admission.json \
  ait-release.promotion.json \
  frozen \
  packages | LC_ALL=C sort >"${expected_top}"
find "${dossier_root}" -mindepth 1 -maxdepth 1 -exec basename {} \; |
  LC_ALL=C sort >"${actual_top}"
if ! diff -u "${expected_top}" "${actual_top}"; then
  printf 'family dossier top-level inventory is not exact\n' >&2
  exit 65
fi

pre_tag_admission=${dossier_root}/ait-release.pre-tag-admission.json
require_regular_file "${pre_tag_admission}" 'family dossier pre-tag admission'
if [[ ${authority_mode} == protected ]]; then
  expected_pre_tag_admission_sha=$(jq -er \
    '.dossier.pre_tag_admission_sha256' "${protected_evidence}")
else
  expected_pre_tag_admission_sha=$(jq -er \
    '.pre_rc_admission.admission_sha256' "${endpoint_config}")
fi
if [[ $(sha256_file "${pre_tag_admission}") != \
  "${expected_pre_tag_admission_sha}" ]]; then
  printf 'family dossier pre-tag admission differs from the selected authority\n' >&2
  exit 65
fi

frozen_root=${dossier_root}/frozen
frozen_manifest=${frozen_root}/ait-release-family.manifest.json
frozen_checksums=${frozen_root}/SHA256SUMS
require_regular_file "${frozen_manifest}" 'frozen family manifest'
require_regular_file "${frozen_checksums}" 'frozen checksum inventory'
if [[ $(sha256_file "${frozen_manifest}") != \
    $(jq -er '.release.frozen_manifest_sha256' "${endpoint_config}") ||
  $(sha256_file "${frozen_checksums}") != \
    $(jq -er '.release.frozen_checksums_sha256' "${endpoint_config}") ]]; then
  printf 'frozen family manifest or checksum digest drifted\n' >&2
  exit 65
fi

verify_checksum_inventory() {
  local checksum_file=$1
  local content_root=$2
  local label=$3
  local count=0 line digest relative actual
  while IFS= read -r line || [[ -n ${line} ]]; do
    if [[ ! ${line} =~ ^([0-9a-f]{64})\ \ (.+)$ ]]; then
      printf '%s contains a malformed checksum row\n' "${label}" >&2
      return 65
    fi
    digest=${BASH_REMATCH[1]}
    relative=${BASH_REMATCH[2]}
    case "${relative}" in
      '' | /* | *'..'* | *'//'*)
        printf '%s contains an unsafe checksum path: %s\n' "${label}" "${relative}" >&2
        return 65
        ;;
    esac
    require_regular_file "${content_root}/${relative}" "${label} member"
    actual=$(sha256_file "${content_root}/${relative}")
    if [[ ${actual} != "${digest}" ]]; then
      printf '%s member digest drifted: %s\n' "${label}" "${relative}" >&2
      return 65
    fi
    count=$((count + 1))
  done <"${checksum_file}"
  printf '%s\n' "${count}"
}

if [[ ${authority_mode} == protected ]]; then
  expected_frozen_count=$(jq -er '.dossier.frozen_checksum_count' "${protected_evidence}")
else
  expected_frozen_count=$(jq -er '.frozen_checksum_count' "${endpoint_config}")
fi
if [[ $(verify_checksum_inventory "${frozen_checksums}" "${frozen_root}" 'frozen inventory') != \
  "${expected_frozen_count}" ]]; then
  printf 'frozen checksum inventory count differs from protected evidence\n' >&2
  exit 65
fi

expected_channels=${temporary_root}/expected-channels
actual_channels=${temporary_root}/actual-channels
printf '%s\n' apt homebrew npm pypi winget | LC_ALL=C sort >"${expected_channels}"
find "${dossier_root}/packages" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; |
  LC_ALL=C sort >"${actual_channels}"
if ! diff -u "${expected_channels}" "${actual_channels}"; then
  printf 'assembled package channel inventory is not exact\n' >&2
  exit 65
fi

copy_asset() {
  local source=$1
  local destination_name=$2
  require_regular_file "${source}" 'release asset source'
  if [[ -e ${assets}/${destination_name} || -L ${assets}/${destination_name} ]]; then
    printf 'release asset basename collides: %s\n' "${destination_name}" >&2
    return 65
  fi
  cp -p "${source}" "${assets}/${destination_name}"
}

npm_addon_platform() {
  case "$1" in
    @wa120/ait-native-darwin-arm64)
      printf '%s\t%s\t%s\t%s\n' aarch64-apple-darwin darwin arm64 none
      ;;
    @wa120/ait-native-darwin-x64)
      printf '%s\t%s\t%s\t%s\n' x86_64-apple-darwin darwin x64 none
      ;;
    @wa120/ait-native-linux-arm64)
      printf '%s\t%s\t%s\t%s\n' aarch64-unknown-linux-gnu linux arm64 glibc
      ;;
    @wa120/ait-native-linux-x64)
      printf '%s\t%s\t%s\t%s\n' x86_64-unknown-linux-gnu linux x64 glibc
      ;;
    @wa120/ait-native-win32-arm64)
      printf '%s\t%s\t%s\t%s\n' aarch64-pc-windows-msvc win32 arm64 none
      ;;
    @wa120/ait-native-win32-x64)
      printf '%s\t%s\t%s\t%s\n' x86_64-pc-windows-msvc win32 x64 none
      ;;
    *)
      return 1
      ;;
  esac
}

validate_npm_payload_contract() {
  local contract=$1
  jq -e --arg version "${release_version}" '
    def expected_platforms: [
      {target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", libc: null},
      {target: "x86_64-apple-darwin", os: "darwin", cpu: "x64", libc: null},
      {target: "aarch64-unknown-linux-gnu", os: "linux", cpu: "arm64", libc: "glibc"},
      {target: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64", libc: "glibc"},
      {target: "aarch64-pc-windows-msvc", os: "win32", cpu: "arm64", libc: null},
      {target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", libc: null}
    ];
    .schema == "ait.node.napi-platform-packages/v2" and
    .family_version == $version and
    .top_level_package == "@wa120/ait-native" and
    (.payloads | type) == "array" and
    (.payloads | length) == 6 and
    ([.payloads[] | {target, os, cpu, libc}] | sort_by(.target)) ==
      (expected_platforms | sort_by(.target)) and
    all(.payloads[];
      (keys | sort) == ([
        "addon", "binding_repository", "binding_snapshot", "component",
        "cpu", "libc", "license", "os", "package", "target", "version"
      ] | sort) and
      .component == "ait-node" and
      .package == ("@wa120/ait-native-" + .os + "-" + .cpu) and
      .version == $version and
      .binding_repository == "ait-core" and
      (.binding_snapshot | test("^SNP-[0-9A-F]{12}$")) and
      .license == "Apache-2.0" and
      .addon == "native/ait_napi.node")
  ' "${contract}" >/dev/null
}

validate_npm_package_archive() {
  local archive=$1
  local package_manifest=$2
  local package_name target os cpu libc provenance contract
  package_name=$(jq -er '.name' "${package_manifest}")
  if ! jq -e --arg version "${release_version}" '
    .version == $version and
    .repository == {
      type: "git",
      url: "git+https://github.com/weita2026/ait-native.git",
      directory: "ait-node"
    }
  ' "${package_manifest}" >/dev/null; then
    return 1
  fi
  if [[ ${package_name} == @wa120/ait-native ]]; then
    if ! jq -e --arg version "${release_version}" \
      --slurpfile config "${endpoint_config}" '
        ((has("os") or has("cpu") or has("libc") or has("aitNativeAddon")) | not) and
        (.optionalDependencies | keys | sort) ==
          ([$config[0].endpoints.npm.packages[] |
            select(. != "@wa120/ait-native")] | sort) and
        all(.optionalDependencies[]; . == $version)
      ' "${package_manifest}" >/dev/null; then
      return 1
    fi
    contract=${temporary_root}/npm-payload-contract.json
    tar -xOf "${archive}" package/lib/npm-payload-contract.json >"${contract}"
    validate_npm_payload_contract "${contract}"
    return
  fi

  if ! IFS=$'\t' read -r target os cpu libc < <(npm_addon_platform "${package_name}"); then
    return 1
  fi
  if ! jq -e \
    --arg version "${release_version}" \
    --arg target "${target}" \
    --arg os "${os}" \
    --arg cpu "${cpu}" \
    --arg libc "${libc}" '
      .version == $version and
      .os == [$os] and
      .cpu == [$cpu] and
      (if $libc == "none" then has("libc") | not
       else .libc == [$libc] end) and
      .aitNativeAddon.schema == "ait.node.napi-platform-addon/v2" and
      .aitNativeAddon.component == "ait-node" and
      .aitNativeAddon.target == $target and
      .aitNativeAddon.libc == (if $libc == "none" then null else $libc end) and
      .aitNativeAddon.addon == "native/ait_napi.node" and
      .aitNativeAddon.binding_repository == "ait-core" and
      (.aitNativeAddon.binding_snapshot | test("^SNP-[0-9A-F]{12}$"))
    ' "${package_manifest}" >/dev/null; then
    return 1
  fi
  provenance=${temporary_root}/npm-provenance-${package_name//\//_}.json
  tar -xOf "${archive}" package/provenance.json >"${provenance}"
  jq -e \
    --arg version "${release_version}" \
    --arg package "${package_name}" \
    --arg target "${target}" \
    --arg os "${os}" \
    --arg cpu "${cpu}" \
    --arg libc "${libc}" '
      .schema == "ait.node.napi-platform-addon-provenance/v2" and
      .family_version == $version and
      .package == $package and
      .target == $target and
      .os == $os and
      .cpu == $cpu and
      .libc == (if $libc == "none" then null else $libc end) and
      .component == "ait-node" and
      .package_source_repository == "ait-node" and
      .binding_repository == "ait-core" and
      (.binding_snapshot | test("^SNP-[0-9A-F]{12}$")) and
      .installed_path == "native/ait_napi.node"
    ' "${provenance}" >/dev/null
}

npm_package_names=${temporary_root}/npm-package-names
: >"${npm_package_names}"
package_authority=${temporary_root}/package-authority.json
if [[ ${authority_mode} == protected ]]; then
  jq -S '.dossier.packages' "${protected_evidence}" >"${package_authority}"
else
  jq -S '.packages' "${endpoint_config}" >"${package_authority}"
fi
for channel in apt homebrew npm pypi winget; do
  channel_root=${dossier_root}/packages/${channel}
  receipt=${channel_root}/ait-release.package.json
  checksums=${channel_root}/SHA256SUMS
  require_regular_file "${receipt}" "${channel} package receipt"
  require_regular_file "${checksums}" "${channel} package checksums"
  expected_receipt_sha=$(jq -er --arg channel "${channel}" \
    '.[] | select(.channel == $channel) | .receipt_sha256' \
    "${package_authority}")
  expected_checksum_sha=$(jq -er --arg channel "${channel}" \
    '.[] | select(.channel == $channel) | .checksum_sha256' \
    "${package_authority}")
  expected_artifact_count=$(jq -er --arg channel "${channel}" \
    '.[] | select(.channel == $channel) | .artifact_count' \
    "${package_authority}")
  if [[ $(sha256_file "${receipt}") != "${expected_receipt_sha}" ||
    $(sha256_file "${checksums}") != "${expected_checksum_sha}" ||
    $(verify_checksum_inventory "${checksums}" "${channel_root}" "${channel} package inventory") != \
      "$((expected_artifact_count + 1))" ]]; then
    printf '%s package receipt or checksum evidence drifted\n' "${channel}" >&2
    exit 65
  fi
  if ! jq -e \
    --arg channel "${channel}" \
    --arg release_id "${release_id}" \
    --arg version "${release_version}" \
    --arg release_channel "${release_channel}" \
    --arg tag "${release_tag}" \
    --slurpfile config "${endpoint_config}" '
    .contract == "ait.release.family.package/v1" and
    .status == "assembled" and
    .release_id == $release_id and
    .version == $version and
    .release_channel == $release_channel and
    .tag == $tag and
    .channel == $channel and
    .check_summary.decision == "pass" and
    .check_summary.blocking == 0 and
    .mutation == {
      component_rebuild: false,
      credentials_loaded: false,
      public_publish: false,
      registry_write: false,
      server_authority_initialization: false,
      service_enable: false,
      service_registration: false,
      service_start: false,
      signing: false,
      tag_write: false
    } and
    if $channel == "winget" then
      .route == {
        community_manifest_submission:
          $config[0].endpoints.winget.community_manifest_submission,
        route: $config[0].endpoints.winget.route
      } and
      all(.artifacts[];
        if .kind == "winget-portable-zip" then true
        else .metadata.community_manifest_submission ==
          $config[0].endpoints.winget.community_manifest_submission end)
    else true end
  ' "${receipt}" >/dev/null; then
    printf '%s package receipt is not an unmodified staging receipt\n' "${channel}" >&2
    exit 65
  fi
  while IFS=$'\t' read -r artifact_path expected_sha expected_size; do
    prefix=dist/${release_id}/
    if [[ ${artifact_path} != "${prefix}"* ]]; then
      printf '%s receipt artifact path is outside the family root: %s\n' \
        "${channel}" "${artifact_path}" >&2
      exit 65
    fi
    source=${dossier_root}/${artifact_path#"${prefix}"}
    require_regular_file "${source}" "${channel} package artifact"
    if [[ $(sha256_file "${source}") != "${expected_sha}" ||
      $(wc -c <"${source}" | tr -d '[:space:]') != "${expected_size}" ]]; then
      printf '%s package artifact drifted: %s\n' "${channel}" "${artifact_path}" >&2
      exit 65
    fi
    if [[ ${channel} == npm ]]; then
      npm_package_manifest=${temporary_root}/npm-package-${expected_sha}.json
      if ! tar -xOf "${source}" package/package.json >"${npm_package_manifest}" ||
        ! jq -e --slurpfile config "${endpoint_config}" '
          .name as $name |
          any($config[0].endpoints.npm.packages[]; . == $name)
        ' "${npm_package_manifest}" >/dev/null ||
        ! validate_npm_package_archive "${source}" "${npm_package_manifest}"; then
        printf 'frozen npm package platform or repository metadata is not exact: %s\n' \
          "${artifact_path}" >&2
        exit 65
      fi
      jq -er '.name' "${npm_package_manifest}" >>"${npm_package_names}"
    fi
    copy_asset "${source}" "$(basename -- "${source}")"
  done < <(jq -r '.artifacts[] | [.path, .sha256, (.size_bytes | tostring)] | @tsv' "${receipt}")
  if [[ ${channel} == npm ]]; then
    expected_npm_package_names=${temporary_root}/expected-npm-package-names
    jq -r '.endpoints.npm.packages[]' "${endpoint_config}" |
      LC_ALL=C sort >"${expected_npm_package_names}"
    LC_ALL=C sort "${npm_package_names}" -o "${npm_package_names}"
    if ! diff -u "${expected_npm_package_names}" "${npm_package_names}"; then
      printf 'frozen npm package identity inventory is not exact\n' >&2
      exit 65
    fi
  fi
  copy_asset "${receipt}" "ait-release-${channel}.package.json"
  copy_asset "${checksums}" "ait-release-${channel}.SHA256SUMS"
done

while IFS=$'\t' read -r component target artifact_path expected_sha expected_size; do
  source=${frozen_root}/${artifact_path}
  require_regular_file "${source}" 'native GitHub asset'
  if [[ $(sha256_file "${source}") != "${expected_sha}" ||
    $(wc -c <"${source}" | tr -d '[:space:]') != "${expected_size}" ]]; then
    printf 'native GitHub asset drifted: %s\n' "${artifact_path}" >&2
    exit 65
  fi
  extension=
  if [[ ${target} == *-windows-msvc ]]; then
    extension=.exe
  fi
  copy_asset "${source}" "${component}-${release_version}-${target}${extension}"
done < <(jq -r '
  .artifacts[] |
  select(.kind == "native-executable") |
  [.component, .target, .path, .sha256, (.size_bytes | tostring)] | @tsv
' "${frozen_manifest}")

while IFS=$'\t' read -r repository _role declared_path material_path expected_sha expected_size; do
  source=${frozen_root}/${material_path}
  require_regular_file "${source}" 'GitHub legal asset'
  if [[ $(sha256_file "${source}") != "${expected_sha}" ||
    $(wc -c <"${source}" | tr -d '[:space:]') != "${expected_size}" ]]; then
    printf 'GitHub legal asset drifted: %s\n' "${material_path}" >&2
    exit 65
  fi
  copy_asset "${source}" "${repository}-${declared_path}"
done < <(jq -r '
  .license_material[] |
  [.source_repository, .material_role, .declared_path, .path, .sha256, (.size_bytes | tostring)] | @tsv
' "${frozen_manifest}")

if [[ ${authority_mode} == protected ]]; then
  copy_asset "${endpoint_config}" 'ait-release.endpoints.json'
  copy_asset "${protected_evidence}" 'ait-release.protected-promotion.json'
else
  copy_asset "${endpoint_config}" 'ait-release.pre-tag-candidate-authority.json'
fi
copy_asset "${dossier_root}/ait-monorepo-source.json" 'ait-monorepo-source.json'
copy_asset "${dossier_root}/ait-native-source-tree.tar.gz" 'ait-native-source-tree.tar.gz'
copy_asset "${dossier_root}/ait-public-git-source.evidence.json" 'ait-public-git-source.evidence.json'
copy_asset "${dossier_root}/ait-release.build.json" 'ait-release.build.json'
copy_asset "${dossier_root}/ait-release.candidate.json" 'ait-release.candidate.json'
copy_asset "${dossier_root}/ait-release.check.json" 'ait-release.check.json'
copy_asset "${dossier_root}/ait-release.promotion.json" 'ait-release.promotion.json'
copy_asset "${frozen_manifest}" 'ait-release-family.manifest.json'
copy_asset "${frozen_checksums}" 'ait-release-frozen.SHA256SUMS'

if [[ ${release_version} == 1.1.0 ]]; then
  benchmark_campaign=game-v1-g56s-max-complete200-fx27-20260826
  benchmark_relative=release/benchmarks/${benchmark_campaign}
  if [[ -d ${repo_root}/${benchmark_relative} &&
    ! -L ${repo_root}/${benchmark_relative} ]]; then
    benchmark_root=${repo_root}/${benchmark_relative}
  elif [[ -d ${repo_root}/ait-core/${benchmark_relative} &&
    ! -L ${repo_root}/ait-core/${benchmark_relative} ]]; then
    benchmark_root=${repo_root}/ait-core/${benchmark_relative}
  else
    printf '1.1.0 public benchmark dossier is unavailable\n' >&2
    exit 66
  fi
  for benchmark_file in summary.txt result.json runs.json SHA256SUMS; do
    require_regular_file "${benchmark_root}/${benchmark_file}" \
      '1.1.0 public benchmark artifact'
  done
  if [[ $(verify_checksum_inventory \
      "${benchmark_root}/SHA256SUMS" "${benchmark_root}" \
      '1.1.0 public benchmark dossier') != 3 ]]; then
    printf '1.1.0 public benchmark checksum inventory is not exact\n' >&2
    exit 65
  fi
  if ! jq -e '
    .contract == "ait-agent-token-benchmark-publication/v1" and
    .release_version == "1.1.0" and
    .campaign_id == "game-v1-g56s-max-complete200-fx27-20260826" and
    .scheduled_run_count == 200 and .observed_run_count == 200 and
    .executed_evidence_run_count == 201 and
    .statistically_excluded_run_count == 1 and
    .valid_run_count == 200 and .invalid_run_count == 0 and
    .accepted_run_count == 200 and
    .accepted_by_mode == {
      ait_linear_single_session: 100,
      git_linear_single_session: 100
    } and
    .source_protocol_claim_eligible == false and
    .claim_eligible == true and
    .claim_blockers == [] and
    (.retained_failures | length) == 0 and
    (.statistically_excluded_failures | length) == 1 and
    .statistically_excluded_failures[0].workload_id == "GD-05" and
    .statistically_excluded_failures[0].mode == "ait_linear_single_session" and
    .statistically_excluded_failures[0].evaluator_score == 50 and
    .replacement_policy_revision == "game-development-2026-08-27.29" and
    (.statistical_replacements | length) == 1 and
    .statistical_replacements[0].source_run_id == "game-v1-g56s-max-complete200-fx27-20260826-b006-gd-05-ait" and
    .statistical_replacements[0].replacement_run_id == "game-v1-g56s-max-complete200-fx27-20260826-b006-gd-05-ait-replacement-01" and
    .statistical_replacements[0].replacement_runner_sha256 == "sha256:89046039ffa7554b8791e5b2c2c75eaa42ac8c719bd93e831d1fc6ba2814d71d" and
    .source_sha256["replacement-runner"] == "89046039ffa7554b8791e5b2c2c75eaa42ac8c719bd93e831d1fc6ba2814d71d"
  ' "${benchmark_root}/result.json" >/dev/null; then
    printf '1.1.0 public benchmark result contract is not exact\n' >&2
    exit 65
  fi
  if ! jq -e '
    .contract == "ait-agent-token-benchmark-public-runs/v1" and
    .campaign_id == "game-v1-g56s-max-complete200-fx27-20260826" and
    .scheduled_run_count == 200 and .observed_run_count == 200 and
    .executed_evidence_run_count == 201 and
    .statistically_excluded_run_count == 1 and
    (.runs | length) == 200 and
    ([.runs[] | select(.valid_attempt == false)] | length) == 0 and
    ([.runs[] | select(.accepted_equivalent == false)] | length) == 0 and
    (.excluded_runs | length) == 1 and
    .excluded_runs[0].valid_attempt == true and
    .excluded_runs[0].accepted_equivalent == false and
    .excluded_runs[0].evaluator_score == 50
  ' "${benchmark_root}/runs.json" >/dev/null; then
    printf '1.1.0 public benchmark run index is not exact\n' >&2
    exit 65
  fi
  copy_asset "${benchmark_root}/summary.txt" \
    "ait-agent-token-benchmark-${benchmark_campaign}.txt"
  copy_asset "${benchmark_root}/result.json" \
    "ait-agent-token-benchmark-${benchmark_campaign}.result.json"
  copy_asset "${benchmark_root}/runs.json" \
    "ait-agent-token-benchmark-${benchmark_campaign}.runs.json"
  copy_asset "${benchmark_root}/SHA256SUMS" \
    "ait-agent-token-benchmark-${benchmark_campaign}.SHA256SUMS"
fi

for component in ait-server ait-runner; do
  context=${staging}/oci/${component}
  cp "${repo_root}/release/oci/${component}.Dockerfile" "${context}/Dockerfile"
  mkdir -p \
    "${context}/bin/amd64" \
    "${context}/bin/arm64" \
    "${context}/licenses" \
    "${context}/runtime"
  : >"${context}/runtime/.keep"
  case "${component}" in
    ait-server)
      repository=ait-server
      ;;
    ait-runner)
      repository=ait-runner
      ;;
  esac
  for target_arch in \
    'x86_64-unknown-linux-gnu amd64' \
    'aarch64-unknown-linux-gnu arm64'; do
    target=${target_arch%% *}
    architecture=${target_arch##* }
    artifact_path=$(jq -er --arg component "${component}" --arg target "${target}" '
      .artifacts[] |
      select(.component == $component and .kind == "native-executable" and .target == $target) |
      .path
    ' "${frozen_manifest}")
    cp -p "${frozen_root}/${artifact_path}" "${context}/bin/${architecture}/${component}"
  done
  while IFS=$'\t' read -r declared_path material_path; do
    cp -p "${frozen_root}/${material_path}" "${context}/licenses/${declared_path}"
  done < <(jq -r --arg repository "${repository}" '
    .license_material[] |
    select(.source_repository == $repository) |
    [.declared_path, .path] | @tsv
  ' "${frozen_manifest}")
  jq -n \
    --arg contract 'ait.release.oci-context/v1' \
    --arg component "${component}" \
    --arg release_id "${release_id}" \
    --arg version "${release_version}" \
    --arg source_commit "$(jq -er '.release.source_commit' "${endpoint_config}")" \
    --arg dockerfile_frontend "$(jq -er '.endpoints.oci.dockerfile_frontend' "${endpoint_config}")" \
    --arg base_image "$(jq -er '.endpoints.oci.base_image' "${endpoint_config}")" \
    --arg amd64_sha256 "$(sha256_file "${context}/bin/amd64/${component}")" \
    --arg arm64_sha256 "$(sha256_file "${context}/bin/arm64/${component}")" '
      {
        contract: $contract,
        component: $component,
        release_id: $release_id,
        version: $version,
        source_commit: $source_commit,
        dockerfile_frontend: $dockerfile_frontend,
        base_image: $base_image,
        component_rebuild: false,
        binaries: {
          amd64: {sha256: $amd64_sha256},
          arm64: {sha256: $arm64_sha256}
        }
      }
    ' >"${context}/provenance.json"
  chmod 0644 "${context}/Dockerfile" "${context}/provenance.json" "${context}/licenses/"*
  chmod 0644 "${context}/runtime/.keep"
  chmod 0755 "${context}/bin/amd64/${component}" "${context}/bin/arm64/${component}"
done

release_checksums=${assets}/SHA256SUMS
find "${assets}" -mindepth 1 -maxdepth 1 -type f ! -name SHA256SUMS -print |
  LC_ALL=C sort |
  while IFS= read -r asset; do
    printf '%s  %s\n' "$(sha256_file "${asset}")" "$(basename -- "${asset}")"
  done >"${release_checksums}"
asset_count=$(wc -l <"${release_checksums}" | tr -d '[:space:]')

endpoint_receipt=${staging}/ait-release.endpoint-publication.json
if [[ ${release_channel} == rc ]]; then
  winget_stage_status=forbidden_for_rc
else
  winget_stage_status=requires_external_community_submission
fi
if [[ ${authority_mode} == pre_tag ]]; then
  jq -n \
    --arg contract 'ait.release.family.pre-tag-candidate-materialization/v1' \
    --arg status 'frozen_for_pre_tag_clean_host' \
    --arg release_id "${release_id}" \
    --arg version "${release_version}" \
    --arg tag "$(jq -er '.release.tag' "${endpoint_config}")" \
    --arg candidate_authority_sha256 "$(sha256_file "${endpoint_config}")" \
    --arg release_checksums_sha256 "$(sha256_file "${release_checksums}")" \
    --arg winget_stage_status "${winget_stage_status}" \
    --argjson asset_count "${asset_count}" \
    --slurpfile config "${endpoint_config}" '
      {
        contract: $contract,
        status: $status,
        release_id: $release_id,
        version: $version,
        tag: $tag,
        candidate_authority_sha256: $candidate_authority_sha256,
        release_checksums_sha256: $release_checksums_sha256,
        release_asset_count: $asset_count,
        endpoints: $config[0].endpoints,
        checks: {
          pre_tag_authority: "pass",
          protected_authorization: "not_requested",
          frozen_checksums: "pass",
          package_receipts: "pass",
          package_checksums: "pass",
          github_asset_staging: "pass",
          oci_context_staging: "pass",
          winget_community_submission: $winget_stage_status
        },
        mutation: {
          artifact_rebuild: false,
          component_rebuild: false,
          credentials_loaded: false,
          registry_write: false,
          github_release_write: false,
          endpoint_repository_write: false,
          tag_write: false,
          ait_remote_release_activation: false,
          service_mutation: false
        },
        next_action: {
          code: "complete_pre_tag_clean_host_matrix",
          detail: "Qualify every frozen installable byte before creating the immutable tag."
        }
      }
    ' >"${endpoint_receipt}"
else
  jq -n \
  --arg contract 'ait.release.family.endpoint-publication/v1' \
  --arg status 'ready_for_authenticated_endpoint_preflight' \
  --arg release_id "${release_id}" \
  --arg version "${release_version}" \
  --arg tag "$(jq -er '.release.tag' "${endpoint_config}")" \
  --arg endpoint_config_sha256 "$(sha256_file "${endpoint_config}")" \
  --arg protected_evidence_sha256 "$(sha256_file "${protected_evidence}")" \
  --arg release_checksums_sha256 "$(sha256_file "${release_checksums}")" \
  --arg winget_stage_status "${winget_stage_status}" \
  --argjson asset_count "${asset_count}" \
  --slurpfile config "${endpoint_config}" '
    {
      contract: $contract,
      status: $status,
      release_id: $release_id,
      version: $version,
      tag: $tag,
      endpoint_config_sha256: $endpoint_config_sha256,
      protected_evidence_sha256: $protected_evidence_sha256,
      release_checksums_sha256: $release_checksums_sha256,
      release_asset_count: $asset_count,
      endpoints: $config[0].endpoints,
      checks: {
        protected_authorization: "pass",
        frozen_checksums: "pass",
        package_receipts: "pass",
        package_checksums: "pass",
        github_asset_staging: "pass",
        oci_context_staging: "pass",
        winget_community_submission: $winget_stage_status
      },
      mutation: {
        artifact_rebuild: false,
        component_rebuild: false,
        credentials_loaded: false,
        registry_write: false,
        github_release_write: false,
        endpoint_repository_write: false,
        tag_write: false,
        ait_remote_release_activation: false,
        service_mutation: false
      },
      next_action: {
        code: "authenticated_all_endpoint_preflight",
        detail: "Prove every configured credential and remote identity before the first package write."
      }
    }
  ' >"${endpoint_receipt}"
fi

chmod 0755 "${staging}" "${assets}" "${staging}/oci" \
  "${staging}/oci/ait-server" "${staging}/oci/ait-runner"
find "${assets}" -type f -exec chmod 0644 {} +
chmod 0644 "${endpoint_receipt}"
mv "${staging}" "${output_root}"
printf '%s\n' "${output_root}"
