#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  printf '%s\n' \
    'usage: release_endpoint_publication.sh <endpoint-config> <dossier-root> <protected-evidence> <output-root>' >&2
  exit 64
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
endpoint_config=$1
dossier_root=$2
protected_evidence=$3
output_root=$4

for command in awk basename cmp cp diff find jq mv node sed sort tar; do
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
require_regular_file "${protected_evidence}" 'protected authorization evidence'
if [[ ${output_root} != /* || -e ${output_root} || -L ${output_root} ]]; then
  printf 'endpoint-publication output must be a new absolute path: %s\n' "${output_root}" >&2
  exit 73
fi
output_parent=$(dirname -- "${output_root}")
require_real_directory "${output_parent}" 'endpoint-publication output parent'

endpoint_config=$(cd "$(dirname -- "${endpoint_config}")" && pwd -P)/$(basename -- "${endpoint_config}")
dossier_root=$(cd "${dossier_root}" && pwd -P)
protected_evidence=$(cd "$(dirname -- "${protected_evidence}")" && pwd -P)/$(basename -- "${protected_evidence}")
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

if ! jq -e '
  .contract == "ait.release.family.endpoints/v1" and
  .release == {
    id: "REL-FAM-701E789EBD8B6848",
    version: "1.0.0-rc.4",
    python_version: "1.0.0rc4",
    tag: "v1.0.0-rc.4",
    source_commit: "ea2d347010d3ead2cdfb304e6df448cbf9fe0c4e",
    coordinator_snapshot: "SNP-2989EE2B6137",
    frozen_manifest_sha256: "6b299059c078445b24cbe4a6db00d23a74d100816f4dfe107b204d4e3c8aceb0",
    frozen_checksums_sha256: "6b67be7893ff536e04c03c4514f92eb7cddd6503953b3c4da7356058e615b9ad"
  } and
  .source_dossier == {
    workflow_run_id: 31716406486,
    workflow_run_attempt: 1,
    workflow_control_commit: "e8925452d487665ea6fd67503278f48baa3eca68",
    artifact_id: 9188270344,
    artifact_digest: "sha256:35c3fc115bda047ca9b9998f4e23941edb3b95c94e08ef98896fd559b1b146b9"
  } and
  .protected_authorization == {
    workflow_run_id: 31721469565,
    workflow_run_attempt: 1,
    workflow_control_commit: "99191fe336d76e5ebba06333ac8a9338f4381763",
    artifact_id: 9189654794,
    artifact_digest: "sha256:aec6ebb7269dcb2333fba915acd3bdc64d379df798a585b422ba0177719752ab",
    evidence_sha256: "4a5c025cdcb8626fe93c71cb4f2780eda1057f979eaa114e4e3e0e3a5dd17e09"
  } and
  .publisher == {
    repository: "weita2026/ait-native",
    workflow: "pypi-publish.yml",
    environment: "pypi"
  } and
  .endpoints.github == {repository: "weita2026/ait-native", prerelease: false} and
  .endpoints.pypi.identity == "ait-native" and
  .endpoints.pypi.trusted_publisher == {
    repository: "weita2026/ait-native",
    workflow: "pypi-publish.yml",
    environment: "pypi"
  } and
  .endpoints.npm.registry == "https://registry.npmjs.org" and
  .endpoints.npm.dist_tag == "rc" and
  .endpoints.npm.credential_secret == "AIT_NPM_TOKEN" and
  .endpoints.npm.packages == [
    "@wa120/ait-native",
    "@wa120/ait-native-darwin-arm64",
    "@wa120/ait-native-darwin-x64",
    "@wa120/ait-native-linux-arm64",
    "@wa120/ait-native-linux-x64",
    "@wa120/ait-native-win32-arm64",
    "@wa120/ait-native-win32-x64"
  ] and
  (.endpoints.npm | has("frozen_missing_repository_metadata") | not) and
  .endpoints.homebrew == {
    repository: "weita2026/homebrew-ait-native",
    branch: "main",
    formula_path: "Formula/ait-native-rc.rb",
    tap: "weita2026/ait-native",
    deploy_key_secret: "AIT_HOMEBREW_DEPLOY_KEY"
  } and
  .endpoints.apt.repository == "weita2026/apt-ait-native" and
  .endpoints.apt.branch == "main" and
  .endpoints.apt.base_url == "https://raw.githubusercontent.com/weita2026/apt-ait-native/main" and
  .endpoints.apt.suite == "testing" and
  .endpoints.apt.component == "main" and
  (.endpoints.apt.signing_fingerprint | test("^[0-9A-F]{40}$")) and
  .endpoints.winget == {
    identity: "Weita.AitNative",
    route: "validation",
    community_manifest_submission: false
  } and
  .endpoints.oci.dockerfile_frontend == "docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e" and
  .endpoints.oci.base_image == "docker.io/library/debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241" and
  .endpoints.oci.images == ["ghcr.io/weita2026/ait-server", "ghcr.io/weita2026/ait-runner"] and
  .endpoints.oci.immutable_tag == "1.0.0-rc.4" and
  .endpoints.oci.moving_tag == "rc"
' "${endpoint_config}" >/dev/null; then
  printf 'endpoint configuration does not match the exact admitted RC route\n' >&2
  exit 65
fi

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
  .dossier.frozen_checksum_count == 48 and
  .dossier.native_promotion_readback_equal == true and
  .dossier.admission_replay == {
    model: "immutable-tag-plus-hash-pinned-control-patch/v1",
    patch: "ci/release_family_rc4_admission.patch",
    patch_sha256: "28d17fa83806498479fee233c8e0ea0defdc337893ed96a667d599e6baaadf0f",
    rust_toolchain: "1.96.0",
    family_packages_input_sha256: "ad5212e194db9a52b049d3334a157959102f115aeeb64f43ff0974328af2e4b3",
    family_packages_output_sha256: "0e7f95bb81dca170343b4b8d2b48949756be76b30956aec6080eee87b2b027d6",
    family_release_input_sha256: "771dd056d3b21c86a63f060bdc44c80bc48717bde3075efd5b2173eb02d68b0f",
    family_release_output_sha256: "ac3d39e4c588aeb500900150dfa51097088d8524264b504f4ee4306de5af7a32"
  } and
  .authorization.required == true and
  .authorization.granted == true and
  .authorization.exact_digest_approval == true and
  .authorization.boundary == "github_protected_environment" and
  .authorization.protected_environment == "rc-promotion" and
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
  printf 'protected authorization evidence does not authorize this exact RC\n' >&2
  exit 65
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
  ait-release.promotion.json \
  frozen \
  packages | LC_ALL=C sort >"${expected_top}"
find "${dossier_root}" -mindepth 1 -maxdepth 1 -exec basename {} \; |
  LC_ALL=C sort >"${actual_top}"
if ! diff -u "${expected_top}" "${actual_top}"; then
  printf 'family dossier top-level inventory is not exact\n' >&2
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

if [[ $(verify_checksum_inventory "${frozen_checksums}" "${frozen_root}" 'frozen inventory') != 48 ]]; then
  printf 'frozen checksum inventory must contain exactly 48 entries\n' >&2
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

release_id=$(jq -er '.release.id' "${endpoint_config}")
release_version=$(jq -er '.release.version' "${endpoint_config}")
npm_package_names=${temporary_root}/npm-package-names
: >"${npm_package_names}"
for channel in apt homebrew npm pypi winget; do
  channel_root=${dossier_root}/packages/${channel}
  receipt=${channel_root}/ait-release.package.json
  checksums=${channel_root}/SHA256SUMS
  require_regular_file "${receipt}" "${channel} package receipt"
  require_regular_file "${checksums}" "${channel} package checksums"
  expected_receipt_sha=$(jq -er --arg channel "${channel}" \
    '.dossier.packages[] | select(.channel == $channel) | .receipt_sha256' \
    "${protected_evidence}")
  expected_checksum_sha=$(jq -er --arg channel "${channel}" \
    '.dossier.packages[] | select(.channel == $channel) | .checksum_sha256' \
    "${protected_evidence}")
  expected_artifact_count=$(jq -er --arg channel "${channel}" \
    '.dossier.packages[] | select(.channel == $channel) | .artifact_count' \
    "${protected_evidence}")
  if [[ $(sha256_file "${receipt}") != "${expected_receipt_sha}" ||
    $(sha256_file "${checksums}") != "${expected_checksum_sha}" ||
    $(verify_checksum_inventory "${checksums}" "${channel_root}" "${channel} package inventory") != \
      "$((expected_artifact_count + 1))" ]]; then
    printf '%s package receipt or checksum evidence drifted\n' "${channel}" >&2
    exit 65
  fi
  if ! jq -e --arg channel "${channel}" --arg release_id "${release_id}" '
    .contract == "ait.release.family.package/v1" and
    .status == "assembled" and
    .release_id == $release_id and
    .version == "1.0.0-rc.4" and
    .tag == "v1.0.0-rc.4" and
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
      .route == {community_manifest_submission: false, route: "validation"} and
      all(.artifacts[];
        if .kind == "winget-portable-zip" then true
        else .metadata.community_manifest_submission == false end)
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
        ! jq -e --arg version "${release_version}" \
          --slurpfile config "${endpoint_config}" '
            .name as $name |
            .version == $version and
            any($config[0].endpoints.npm.packages[]; . == $name) and
            .repository == {
              type: "git",
              url: "git+https://github.com/weita2026/ait-native.git",
              directory: "ait-node"
            }
          ' "${npm_package_manifest}" >/dev/null; then
        printf 'frozen npm package repository metadata is not exact: %s\n' \
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

copy_asset "${endpoint_config}" 'ait-release.endpoints.json'
copy_asset "${protected_evidence}" 'ait-release.protected-promotion.json'
copy_asset "${dossier_root}/ait-monorepo-source.json" 'ait-monorepo-source.json'
copy_asset "${dossier_root}/ait-native-source-tree.tar.gz" 'ait-native-source-tree.tar.gz'
copy_asset "${dossier_root}/ait-public-git-source.evidence.json" 'ait-public-git-source.evidence.json'
copy_asset "${dossier_root}/ait-release.build.json" 'ait-release.build.json'
copy_asset "${dossier_root}/ait-release.candidate.json" 'ait-release.candidate.json'
copy_asset "${dossier_root}/ait-release.check.json" 'ait-release.check.json'
copy_asset "${dossier_root}/ait-release.promotion.json" 'ait-release.promotion.json'
copy_asset "${frozen_manifest}" 'ait-release-family.manifest.json'
copy_asset "${frozen_checksums}" 'ait-release-frozen.SHA256SUMS'

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
jq -n \
  --arg contract 'ait.release.family.endpoint-publication/v1' \
  --arg status 'ready_for_authenticated_endpoint_preflight' \
  --arg release_id "${release_id}" \
  --arg version "${release_version}" \
  --arg tag "$(jq -er '.release.tag' "${endpoint_config}")" \
  --arg endpoint_config_sha256 "$(sha256_file "${endpoint_config}")" \
  --arg protected_evidence_sha256 "$(sha256_file "${protected_evidence}")" \
  --arg release_checksums_sha256 "$(sha256_file "${release_checksums}")" \
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
        winget_community_submission: "forbidden_for_rc"
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

chmod 0755 "${staging}" "${assets}" "${staging}/oci" \
  "${staging}/oci/ait-server" "${staging}/oci/ait-runner"
find "${assets}" -type f -exec chmod 0644 {} +
chmod 0644 "${endpoint_receipt}"
mv "${staging}" "${output_root}"
printf '%s\n' "${output_root}"
