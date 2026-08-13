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
    id: "REL-FAM-600EFDC327FE7860",
    version: "1.0.0-rc.3",
    python_version: "1.0.0rc3",
    tag: "v1.0.0-rc.3",
    source_commit: "ba368cf4d0750035345f14a8a91c22fb9e450260",
    coordinator_snapshot: "SNP-B0271928FD9B",
    frozen_manifest_sha256: "2a228253ceea6f793df050e9bf2fc14f240c8d9db5ebdcbcbc6133e61e6238fe",
    frozen_checksums_sha256: "9fd126c61d716a3e8056e598ba6d3ecee992b0bbc7e073470887e49d11877747"
  } and
  .source_dossier == {
    workflow_run_id: 31664713921,
    workflow_run_attempt: 1,
    workflow_control_commit: "93f2589d8eb7404400617169598427aaef3ff8af",
    artifact_id: 9167933771,
    artifact_digest: "sha256:08afc391688c902f3c2259392286b51612e6b6eb0aa51c388e8e513329705823"
  } and
  .protected_authorization == {
    workflow_run_id: 31666479359,
    workflow_run_attempt: 1,
    workflow_control_commit: "93f2589d8eb7404400617169598427aaef3ff8af",
    artifact_id: 9168120753,
    artifact_digest: "sha256:54079d53bc3e115f314d99591228cc80dbab4c56c5ae361530f9c490c0764be9",
    evidence_sha256: "cc18cf39db59147d5ee94359f0c00813be6841bf317686451eafd1152f870b32"
  } and
  .publisher == {
    repository: "weita2026/ait-native",
    workflow: "pypi-publish.yml",
    environment: "pypi"
  } and
  .endpoints.github == {repository: "weita2026/ait-native", prerelease: true} and
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
    "ait-native",
    "ait-native-ait-darwin-arm64",
    "ait-native-ait-darwin-x64",
    "ait-native-ait-linux-arm64",
    "ait-native-ait-linux-x64",
    "ait-native-ait-win32-arm64",
    "ait-native-ait-win32-x64"
  ] and
  .endpoints.npm.frozen_missing_repository_metadata == {
    external_github_attestation_required: true,
    archives: {
      "ait-native": "8862dc3621320fda30e6923c85eee872751bfc92d95f319382b5b690540392f8",
      "ait-native-ait-darwin-arm64": "262b2860df61c64dd8c358d0e36c5ae136ae0f98ee1bbc04511ba0608313abd2",
      "ait-native-ait-darwin-x64": "42cb08e1651e8d96cd4dfc56cabf63ce0c50629b9c122ce065351cf67747e870",
      "ait-native-ait-linux-arm64": "e59c1d29819454d20943dad038e7e3273d114c29033ff22d2430ae427778b221",
      "ait-native-ait-linux-x64": "b594c192d921aa2e9c0fd6868d477b76fb431bac6ef0b89c8c0f011ac5cc1843",
      "ait-native-ait-win32-arm64": "510ad5a977948c9a71c5434f721370fd2265b7665d3198bac597e563d2d4a8be",
      "ait-native-ait-win32-x64": "7e09475a008b9a36993c9efe89ee99fd493da0457d06ca349143449e95b3298b"
    }
  } and
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
  .endpoints.oci.immutable_tag == "1.0.0-rc.3" and
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
  .authorization.required == true and
  .authorization.granted == true and
  .authorization.exact_digest_approval == true and
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
    .version == "1.0.0-rc.3" and
    .tag == "v1.0.0-rc.3" and
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
    copy_asset "${source}" "$(basename -- "${source}")"
  done < <(jq -r '.artifacts[] | [.path, .sha256, (.size_bytes | tostring)] | @tsv' "${receipt}")
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
