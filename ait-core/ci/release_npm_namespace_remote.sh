#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf '%s\n' \
    'usage: release_npm_namespace_remote.sh <preflight|publish|readback> <config> <supplement-stage>' >&2
  exit 64
fi

mode=$1
config=$2
stage=$3
script_root=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
dist_tags_filter=${script_root}/release_npm_namespace_dist_tags.jq
case "${mode}" in
  preflight | publish | readback) ;;
  *)
    printf 'unsupported npm namespace supplement mode: %s\n' "${mode}" >&2
    exit 64
    ;;
esac

for command in awk curl find jq npm openssl sort tar wc; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf 'required npm namespace supplement command is unavailable: %s\n' \
      "${command}" >&2
    exit 69
  fi
done

sha256_file() {
  local file=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
  else
    shasum -a 256 "${file}" | awk '{print $1}'
  fi
}

sha1_file() {
  local file=$1
  if command -v sha1sum >/dev/null 2>&1; then
    sha1sum "${file}" | awk '{print $1}'
  else
    shasum -a 1 "${file}" | awk '{print $1}'
  fi
}

sha512_integrity() {
  local file=$1
  printf 'sha512-%s\n' \
    "$(openssl dgst -sha512 -binary "${file}" | openssl base64 -A)"
}

require_regular_file() {
  local file=$1
  local label=$2
  if [[ ! -f ${file} || -L ${file} ]]; then
    printf '%s must be a regular non-symlink file: %s\n' "${label}" "${file}" >&2
    return 66
  fi
}

require_real_directory() {
  local directory=$1
  local label=$2
  if [[ ${directory} != /* || ! -d ${directory} || -L ${directory} ]]; then
    printf '%s must be an absolute real directory: %s\n' \
      "${label}" "${directory}" >&2
    return 66
  fi
}

require_environment() {
  local variable
  for variable in "$@"; do
    if [[ -z ${!variable:-} ]]; then
      printf 'required npm namespace supplement environment is missing: %s\n' \
        "${variable}" >&2
      return 64
    fi
  done
}

require_regular_file "${config}" 'npm namespace supplement configuration'
require_regular_file "${dist_tags_filter}" 'npm namespace supplement dist-tag filter'
require_real_directory "${stage}" 'npm namespace supplement stage'
config=$(cd "$(dirname -- "${config}")" && pwd -P)/$(basename -- "${config}")
stage=$(cd "${stage}" && pwd -P)
stage_receipt=${stage}/ait-release.npm-namespace-supplement.json
checksums=${stage}/SHA256SUMS
packages_root=${stage}/packages
evidence_root=${stage}/evidence
require_regular_file "${stage_receipt}" 'npm namespace supplement stage receipt'
require_regular_file "${checksums}" 'npm namespace supplement checksums'
require_real_directory "${packages_root}" 'npm namespace supplement package root'
mkdir -p "${evidence_root}"

registry=$(jq -er '.registry.url' "${config}")
registry_host=${registry#https://}
registry_host=${registry_host%/}
version=$(jq -er '.release.version' "${config}")
dist_tag=$(jq -er '.registry.dist_tag' "${config}")

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-npm-namespace-remote.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-npm-namespace-remote.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected npm supplement path: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

verify_stage() {
  local expected_config_sha actual_names expected_names count=0
  local line digest filename archive package_name package_version package_repository
  local receipt_sha receipt_sha1 receipt_integrity receipt_size actual_size
  expected_config_sha=$(sha256_file "${config}")
  if ! jq -e \
    --arg config_sha "${expected_config_sha}" \
    --slurpfile config "${config}" '
      .contract == "ait.release.npm-namespace-supplement.stage/v1" and
      .status == "ready_for_authenticated_npm_preflight" and
      .release == $config[0].release and
      .node_source.repository == $config[0].node_source.repository and
      .node_source.snapshot == $config[0].node_source.snapshot and
      .node_source.snapshot_manifest_hash == $config[0].node_source.snapshot_manifest_hash and
      .node_source.snapshot_created_at_s == $config[0].node_source.snapshot_created_at_s and
      .node_source.binding_repository == $config[0].node_source.binding_repository and
      .node_source.binding_snapshot == $config[0].node_source.binding_snapshot and
      .publisher == $config[0].publisher and
      .toolchain == $config[0].build_toolchain and
      .config_sha256 == $config_sha and
      (.packages | length) == 7 and
      ([.packages[].order] == [1, 2, 3, 4, 5, 6, 7]) and
      ([.packages[].package] == $config[0].registry.packages[1:] + [$config[0].registry.top_level_package]) and
      ([.packages[].version] | all(. == $config[0].release.version)) and
      ([.packages[0:6][].target] == [$config[0].addons[].target]) and
      .packages[6].target == null and
      (.addon_mappings | length) == 6 and
      ([.addon_mappings[].native_bytes_identical] | all(. == true)) and
      .mutation == $config[0].mutation and
      .mutation.native_addon_rebuild == false and
      .mutation.tag_write == false and
      .mutation.github_release_write == false and
      .mutation.existing_unscoped_package_write == false
    ' "${stage_receipt}" >/dev/null; then
    printf 'npm namespace supplement stage receipt is not exact\n' >&2
    return 65
  fi

  expected_names=${temporary_root}/expected-package-files
  actual_names=${temporary_root}/actual-package-files
  jq -r '.packages[].filename' "${stage_receipt}" | LC_ALL=C sort >"${expected_names}"
  find "${packages_root}" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; |
    LC_ALL=C sort >"${actual_names}"
  if ! diff -u "${expected_names}" "${actual_names}"; then
    printf 'npm namespace supplement package inventory drifted\n' >&2
    return 65
  fi
  if find "${packages_root}" -mindepth 1 -maxdepth 1 ! -type f -print -quit |
    grep -q .; then
    printf 'npm namespace supplement package root contains a non-file\n' >&2
    return 65
  fi

  while IFS= read -r line || [[ -n ${line} ]]; do
    if [[ ! ${line} =~ ^([0-9a-f]{64})\ \ ([^/]+)$ ]]; then
      printf 'npm namespace supplement checksum row is malformed\n' >&2
      return 65
    fi
    digest=${BASH_REMATCH[1]}
    filename=${BASH_REMATCH[2]}
    archive=${packages_root}/${filename}
    require_regular_file "${archive}" 'staged npm package'
    if [[ $(sha256_file "${archive}") != "${digest}" ]]; then
      printf 'staged npm package SHA-256 drifted: %s\n' "${filename}" >&2
      return 65
    fi
    count=$((count + 1))
  done <"${checksums}"
  if [[ ${count} != 7 ]]; then
    printf 'npm namespace supplement checksum inventory must contain seven rows\n' >&2
    return 65
  fi

  while IFS=$'\t' read -r package_name filename receipt_sha receipt_sha1 \
    receipt_integrity receipt_size; do
    archive=${packages_root}/${filename}
    require_regular_file "${archive}" 'staged npm package'
    actual_size=$(wc -c <"${archive}" | tr -d '[:space:]')
    if [[ $(sha256_file "${archive}") != "${receipt_sha}" ||
      $(sha1_file "${archive}") != "${receipt_sha1}" ||
      $(sha512_integrity "${archive}") != "${receipt_integrity}" ||
      ${actual_size} != "${receipt_size}" ]]; then
      printf 'staged npm package receipt drifted: %s\n' "${package_name}" >&2
      return 65
    fi
    package_version=$(tar -xOf "${archive}" package/package.json | jq -er .version)
    package_repository=$(tar -xOf "${archive}" package/package.json | jq -cer '.repository')
    if [[ $(tar -xOf "${archive}" package/package.json | jq -er .name) != "${package_name}" ||
      ${package_version} != "${version}" ||
      ${package_repository} != \
        '{"type":"git","url":"git+https://github.com/weita2026/ait-native.git","directory":"ait-node"}' ]]; then
      printf 'staged npm package metadata drifted: %s\n' "${package_name}" >&2
      return 65
    fi
    if tar -xOf "${archive}" package/package.json | jq -e '
      .dependencies != null or .scripts.preinstall != null or .scripts.install != null or
      .scripts.postinstall != null or .scripts.prepack != null
    ' >/dev/null; then
      printf 'staged npm package gained a dependency or lifecycle hook: %s\n' \
        "${package_name}" >&2
      return 65
    fi
  done < <(jq -r '.packages[] | [.package, .filename, .sha256, .sha1, .integrity, (.size_bytes | tostring)] | @tsv' "${stage_receipt}")

  local addon_index target expected_native native_file native_sha native_size
  for addon_index in 0 1 2 3 4 5; do
    package_name=$(jq -er ".packages[${addon_index}].package" "${stage_receipt}")
    filename=$(jq -er ".packages[${addon_index}].filename" "${stage_receipt}")
    target=$(jq -er ".packages[${addon_index}].target" "${stage_receipt}")
    expected_native=$(jq -er ".addons[${addon_index}].native_sha256" "${config}")
    native_size=$(jq -er ".addons[${addon_index}].native_size_bytes" "${config}")
    archive=${packages_root}/${filename}
    native_file=${temporary_root}/native-${addon_index}.node
    tar -xOf "${archive}" package/native/ait_napi.node >"${native_file}"
    native_sha=$(sha256_file "${native_file}")
    if [[ ${target} != $(jq -er ".addons[${addon_index}].target" "${config}") ||
      ${native_sha} != "${expected_native}" ||
      $(wc -c <"${native_file}" | tr -d '[:space:]') != "${native_size}" ]]; then
      printf 'staged scoped addon native identity drifted: %s\n' "${package_name}" >&2
      return 65
    fi
    if ! tar -xOf "${archive}" package/provenance.json | jq -e \
      --arg package "${package_name}" \
      --arg target "${target}" \
      --arg native_sha "${expected_native}" '
        .schema == "ait.node.napi-platform-addon-provenance/v1" and
        .family_version == "1.0.0-rc.3" and
        .package == $package and
        .target == $target and
        .binding_repository == "ait-core" and
        .binding_snapshot == "SNP-158C9C5BB3D7" and
        .source_artifact.sha256 == $native_sha and
        .installed_path == "native/ait_napi.node"
      ' >/dev/null; then
      printf 'staged scoped addon provenance drifted: %s\n' "${package_name}" >&2
      return 65
    fi
  done

  local top_filename
  top_filename=$(jq -er '.packages[6].filename' "${stage_receipt}")
  if tar -tzf "${packages_root}/${top_filename}" |
    grep -E '(^|/)(native/|[^/]+\.node$)' >/dev/null; then
    printf 'top-level scoped package contains native implementation bytes\n' >&2
    return 65
  fi
}

write_npm_config() {
  local npmrc=$1
  require_environment AIT_NPM_TOKEN
  umask 077
  printf '//%s/:_authToken=%s\n' "${registry_host}" "${AIT_NPM_TOKEN}" >"${npmrc}"
}

encoded_package() {
  local package_name=$1
  printf '%s\n' "${package_name/\//%2f}"
}

registry_metadata() {
  local package_name=$1
  local output=$2
  local authenticated=${3:-false}
  local status
  local -a headers=()
  if [[ ${authenticated} == true ]]; then
    require_environment AIT_NPM_TOKEN
    headers=(--header "Authorization: Bearer ${AIT_NPM_TOKEN}")
  fi
  status=$(curl --silent --show-error --location \
    --output "${output}" --write-out '%{http_code}' \
    --header 'Accept: application/vnd.npm.install-v1+json' \
    "${headers[@]}" \
    "${registry}/$(encoded_package "${package_name}")")
  case "${status}" in
    200 | 404) printf '%s\n' "${status}" ;;
    *)
      printf 'npm registry returned HTTP %s for %s\n' "${status}" "${package_name}" >&2
      return 69
      ;;
  esac
}

remote_state_rows() {
  local require_published=$1
  local authenticated=$2
  local output=$3
  local index=0 package_name expected_sha1 expected_integrity metadata status
  local remote_sha1 remote_integrity remote_version state
  : >"${output}"
  while IFS=$'\t' read -r package_name expected_sha1 expected_integrity; do
    metadata=${temporary_root}/registry-${index}.json
    status=$(registry_metadata "${package_name}" "${metadata}" "${authenticated}")
    if [[ ${status} == 404 ]]; then
      if [[ ${require_published} == true ]]; then
        return 75
      fi
      state=absent
      remote_sha1=''
      remote_integrity=''
      remote_version=''
    else
      remote_sha1=$(jq -er --arg version "${version}" \
        '.versions[$version].dist.shasum // ""' "${metadata}")
      remote_integrity=$(jq -er --arg version "${version}" \
        '.versions[$version].dist.integrity // ""' "${metadata}")
      if [[ -z ${remote_sha1} && -z ${remote_integrity} ]]; then
        if [[ ${require_published} == true ]]; then
          return 75
        fi
        state=absent
        remote_version=''
      elif [[ ${remote_sha1} != "${expected_sha1}" ||
        ${remote_integrity} != "${expected_integrity}" ]]; then
        printf 'npm registry already contains different bytes: %s@%s\n' \
          "${package_name}" "${version}" >&2
        return 65
      else
        state=exact
        remote_version=${version}
      fi
    fi
    jq -cn \
      --arg package "${package_name}" \
      --arg version "${remote_version}" \
      --arg state "${state}" \
      --arg shasum "${remote_sha1}" \
      --arg integrity "${remote_integrity}" \
      '{package: $package, version: $version, state: $state, shasum: $shasum, integrity: $integrity}' \
      >>"${output}"
    index=$((index + 1))
  done < <(jq -r '.packages[] | [.package, .sha1, .integrity] | @tsv' "${stage_receipt}")
}

validate_dist_tags() {
  local metadata=$1
  local package_name=$2
  if ! jq -e --arg tag "${dist_tag}" --arg version "${version}" \
    -f "${dist_tags_filter}" \
    "${metadata}" >/dev/null; then
    printf 'npm RC-only dist-tag readback failed: %s@%s\n' \
      "${package_name}" "${version}" >&2
    return 75
  fi
}

wait_for_exact_registry() {
  local authenticated=$1
  local attempts=${2:-120}
  local index package_name metadata status state_rows
  for ((index = 1; index <= attempts; index += 1)); do
    state_rows=${temporary_root}/readback-rows-${index}.jsonl
    if remote_state_rows true "${authenticated}" "${state_rows}"; then
      local tags_ready=true row_index=0
      while IFS= read -r package_name; do
        metadata=${temporary_root}/registry-tags-${index}-${row_index}.json
        status=$(registry_metadata "${package_name}" "${metadata}" "${authenticated}")
        if [[ ${status} != 200 ]] || ! validate_dist_tags "${metadata}" "${package_name}"; then
          tags_ready=false
          break
        fi
        row_index=$((row_index + 1))
      done < <(jq -r '.packages[].package' "${stage_receipt}")
      if [[ ${tags_ready} == true ]]; then
        cp "${state_rows}" "${temporary_root}/final-registry-rows.jsonl"
        return 0
      fi
    else
      local result=$?
      if [[ ${result} != 75 ]]; then
        return "${result}"
      fi
    fi
    if ((index < attempts)); then
      sleep 5
    fi
  done
  printf 'scoped npm RC set did not become fully visible after %s attempts\n' \
    "${attempts}" >&2
  return 75
}

write_evidence() {
  local output=$1
  local contract=$2
  local status=$3
  local rows=$4
  local username=${5:-}
  jq -s \
    --arg contract "${contract}" \
    --arg status "${status}" \
    --arg release_id "$(jq -er '.release.id' "${config}")" \
    --arg version "${version}" \
    --arg tag "$(jq -er '.release.tag' "${config}")" \
    --arg config_sha256 "$(sha256_file "${config}")" \
    --arg stage_receipt_sha256 "$(sha256_file "${stage_receipt}")" \
    --arg username "${username}" \
    --argjson mutation "$(jq -c '.mutation' "${config}")" '
      {
        contract: $contract,
        status: $status,
        release_id: $release_id,
        version: $version,
        tag: $tag,
        config_sha256: $config_sha256,
        stage_receipt_sha256: $stage_receipt_sha256,
        authenticated_username: (if $username == "" then null else $username end),
        packages: .,
        mutation: $mutation
      }
    ' "${rows}" >"${output}"
}

case "${mode}" in
  preflight)
    verify_stage
    require_environment AIT_NPM_TOKEN
    npmrc=${temporary_root}/npmrc
    write_npm_config "${npmrc}"
    username=$(NPM_CONFIG_USERCONFIG="${npmrc}" npm whoami --registry "${registry}")
    expected_username=$(jq -er '.publisher.npm_username' "${config}")
    if [[ ${username} != "${expected_username}" ]]; then
      printf 'npm publisher identity must be %s, got %s\n' \
        "${expected_username}" "${username}" >&2
      exit 65
    fi
    rows=${temporary_root}/preflight-rows.jsonl
    remote_state_rows false true "${rows}"
    write_evidence \
      "${evidence_root}/npm-preflight.json" \
      'ait.release.npm-namespace-supplement.preflight/v1' \
      'ready_for_attested_scoped_publication' \
      "${rows}" \
      "${username}"
    printf '%s\n' '{"mode":"preflight","package_count":7,"status":"pass"}'
    ;;
  publish)
    verify_stage
    require_environment \
      AIT_NPM_TOKEN AIT_STAGE_ATTESTATION_VERIFIED GITHUB_ACTIONS \
      GITHUB_REPOSITORY GITHUB_REF
    if [[ ${AIT_STAGE_ATTESTATION_VERIFIED} != true ||
      ${GITHUB_ACTIONS} != true ||
      ${GITHUB_REPOSITORY} != weita2026/ait-native ||
      ${GITHUB_REF} != refs/heads/main ]]; then
      printf 'npm namespace publication is restricted to attested public main\n' >&2
      exit 65
    fi
    preflight=${evidence_root}/npm-preflight.json
    require_regular_file "${preflight}" 'npm namespace supplement preflight evidence'
    if ! jq -e \
      --arg config_sha "$(sha256_file "${config}")" \
      --arg stage_sha "$(sha256_file "${stage_receipt}")" '
        .contract == "ait.release.npm-namespace-supplement.preflight/v1" and
        .status == "ready_for_attested_scoped_publication" and
        .release_id == "REL-FAM-600EFDC327FE7860" and
        .version == "1.0.0-rc.3" and
        .config_sha256 == $config_sha and
        .stage_receipt_sha256 == $stage_sha and
        .authenticated_username == "wa120" and
        (.packages | length) == 7 and
        ([.packages[].state] | all(. == "absent" or . == "exact"))
      ' "${preflight}" >/dev/null; then
      printf 'npm namespace supplement preflight evidence is not exact\n' >&2
      exit 65
    fi
    npmrc=${temporary_root}/npmrc
    write_npm_config "${npmrc}"
    username=$(NPM_CONFIG_USERCONFIG="${npmrc}" npm whoami --registry "${registry}")
    if [[ ${username} != wa120 ]]; then
      printf 'npm publisher identity changed after preflight\n' >&2
      exit 65
    fi
    current_rows=${temporary_root}/publish-start-rows.jsonl
    remote_state_rows false true "${current_rows}"
    index=0
    while IFS=$'\t' read -r package_name filename expected_sha1 expected_integrity; do
      state=$(sed -n "$((index + 1))p" "${current_rows}" | jq -er .state)
      archive=${packages_root}/${filename}
      if [[ ${state} == absent ]]; then
        NPM_CONFIG_USERCONFIG="${npmrc}" npm publish "${archive}" \
          --access public \
          --ignore-scripts \
          --provenance \
          --registry "${registry}" \
          --tag "${dist_tag}"
      fi
      NPM_CONFIG_USERCONFIG="${npmrc}" npm dist-tag add \
        "${package_name}@${version}" "${dist_tag}" \
        --registry "${registry}" >/dev/null
      index=$((index + 1))
    done < <(jq -r '.packages[] | [.package, .filename, .sha1, .integrity] | @tsv' "${stage_receipt}")
    wait_for_exact_registry true
    write_evidence \
      "${evidence_root}/npm-publish.json" \
      'ait.release.npm-namespace-supplement.publication/v1' \
      'scoped_packages_published_and_authenticated_read_back' \
      "${temporary_root}/final-registry-rows.jsonl" \
      "${username}"
    printf '%s\n' '{"mode":"publish","package_count":7,"status":"pass"}'
    ;;
  readback)
    verify_stage
    wait_for_exact_registry false
    write_evidence \
      "${evidence_root}/npm-readback.json" \
      'ait.release.npm-namespace-supplement.readback/v1' \
      'scoped_packages_anonymously_read_back' \
      "${temporary_root}/final-registry-rows.jsonl"
    printf '%s\n' '{"mode":"readback","package_count":7,"status":"pass"}'
    ;;
esac
