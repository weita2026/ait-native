#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf '%s\n' \
    'usage: release_endpoint_remote.sh <preflight-oci|preflight|publish-github|publish-pypi|publish-npm|publish-homebrew|publish-apt|readback-oci|readback> <endpoint-config> <publication-stage>' >&2
  exit 64
fi

mode=$1
endpoint_config=$2
publication_stage=$3

case "${mode}" in
  preflight-oci | preflight | publish-github | publish-pypi | publish-npm | publish-homebrew | publish-apt | readback-oci | readback) ;;
  *)
    printf 'unsupported endpoint-publication mode: %s\n' "${mode}" >&2
    exit 64
    ;;
esac

for command in awk base64 cmp curl diff find git grep jq npm openssl sed sort ssh-keygen tar wc; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf 'required remote-publication command is unavailable: %s\n' "${command}" >&2
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

sha1_file() {
  local path=$1
  if command -v sha1sum >/dev/null 2>&1; then
    sha1sum "${path}" | awk '{print $1}'
  else
    shasum -a 1 "${path}" | awk '{print $1}'
  fi
}

sha512_integrity() {
  local path=$1
  printf 'sha512-%s\n' "$(openssl dgst -sha512 -binary "${path}" | openssl base64 -A)"
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

require_environment() {
  local variable
  for variable in "$@"; do
    if [[ -z ${!variable:-} ]]; then
      printf 'required remote-publication environment is missing: %s\n' "${variable}" >&2
      return 64
    fi
  done
}

require_regular_file "${endpoint_config}" 'endpoint configuration'
require_real_directory "${publication_stage}" 'publication stage'
stage_receipt=${publication_stage}/ait-release.endpoint-publication.json
assets=${publication_stage}/assets
require_regular_file "${stage_receipt}" 'endpoint-publication staging receipt'
require_real_directory "${assets}" 'release asset stage'

endpoint_config=$(cd "$(dirname -- "${endpoint_config}")" && pwd -P)/$(basename -- "${endpoint_config}")
publication_stage=$(cd "${publication_stage}" && pwd -P)
stage_receipt=${publication_stage}/ait-release.endpoint-publication.json
assets=${publication_stage}/assets
evidence_root=${publication_stage}/evidence
mkdir -p "${evidence_root}"

if ! jq -e --arg expected "$(sha256_file "${endpoint_config}")" --slurpfile config "${endpoint_config}" '
  .contract == "ait.release.family.endpoint-publication/v1" and
  .status == "ready_for_authenticated_endpoint_preflight" and
  .release_id == $config[0].release.id and
  .version == $config[0].release.version and
  .tag == $config[0].release.tag and
  .endpoint_config_sha256 == $expected and
  .protected_evidence_sha256 == $config[0].protected_authorization.evidence_sha256 and
  .endpoints == $config[0].endpoints and
  .mutation == {
    artifact_rebuild: false,
    component_rebuild: false,
    credentials_loaded: false,
    registry_write: false,
    github_release_write: false,
    endpoint_repository_write: false,
    tag_write: false,
    ait_remote_release_activation: false,
    service_mutation: false
  }
' "${stage_receipt}" >/dev/null; then
  printf 'endpoint-publication staging receipt is not exact\n' >&2
  exit 65
fi

release_id=$(jq -er '.release.id' "${endpoint_config}")
release_version=$(jq -er '.release.version' "${endpoint_config}")
python_version=$(jq -er '.release.python_version' "${endpoint_config}")
release_tag=$(jq -er '.release.tag' "${endpoint_config}")
source_commit=$(jq -er '.release.source_commit' "${endpoint_config}")
github_repository=$(jq -er '.endpoints.github.repository' "${endpoint_config}")
npm_registry=$(jq -er '.endpoints.npm.registry' "${endpoint_config}")
npm_registry_host=${npm_registry#https://}
npm_registry_host=${npm_registry_host%/}

verify_release_assets() {
  local checksum_file=${assets}/SHA256SUMS
  local expected_count line digest name path actual_count=0
  require_regular_file "${checksum_file}" 'release asset checksum inventory'
  if [[ $(sha256_file "${checksum_file}") != \
      $(jq -er '.release_checksums_sha256' "${stage_receipt}") ]]; then
    printf 'release asset checksum inventory digest drifted\n' >&2
    return 65
  fi
  while IFS= read -r line || [[ -n ${line} ]]; do
    if [[ ! ${line} =~ ^([0-9a-f]{64})\ \ (.+)$ ]]; then
      printf 'release asset checksum inventory contains a malformed row\n' >&2
      return 65
    fi
    digest=${BASH_REMATCH[1]}
    name=${BASH_REMATCH[2]}
    case "${name}" in
      '' | /* | *'/'* | *'..'*)
        printf 'release asset checksum inventory contains an unsafe name: %s\n' \
          "${name}" >&2
        return 65
        ;;
    esac
    path=${assets}/${name}
    require_regular_file "${path}" 'release asset'
    if [[ $(sha256_file "${path}") != "${digest}" ]]; then
      printf 'release asset digest drifted: %s\n' "${name}" >&2
      return 65
    fi
    actual_count=$((actual_count + 1))
  done <"${checksum_file}"
  expected_count=$(jq -er '.release_asset_count' "${stage_receipt}")
  if [[ ${actual_count} != "${expected_count}" ||
    $(find "${assets}" -mindepth 1 -maxdepth 1 -type f | wc -l | tr -d '[:space:]') != \
      "$((expected_count + 1))" ]]; then
    printf 'release asset inventory count drifted\n' >&2
    return 65
  fi
}

verify_release_assets

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-endpoint-remote.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-endpoint-remote.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected endpoint-remote path: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

github_release_asset_name() {
  local local_name=$1
  case "${local_name}" in
    *.deb) printf '%s\n' "${local_name//\~/.}" ;;
    *) printf '%s\n' "${local_name}" ;;
  esac
}

github_release_asset_map() {
  local output=$1
  local raw=${temporary_root}/github-asset-map-raw
  local local_asset local_name remote_name
  : >"${raw}"
  while IFS= read -r local_asset; do
    local_name=$(basename -- "${local_asset}")
    remote_name=$(github_release_asset_name "${local_name}")
    printf '%s\t%s\n' "${remote_name}" "${local_asset}" >>"${raw}"
  done < <(find "${assets}" -mindepth 1 -maxdepth 1 -type f -print | LC_ALL=C sort)
  printf '%s\t%s\n' 'ait-release.endpoint-publication.json' "${stage_receipt}" >>"${raw}"
  LC_ALL=C sort "${raw}" >"${output}"
  if ! awk -F '\t' '
    seen[$1]++ { print $1; bad = 1 }
    END { exit bad }
  ' "${output}"; then
    printf 'GitHub Release filename normalization creates an asset collision\n' >&2
    return 65
  fi
}

github_release_local_path() {
  local asset_map=$1
  local remote_name=$2
  awk -F '\t' -v name="${remote_name}" '
    $1 == name {
      found += 1
      path = substr($0, index($0, "\t") + 1)
    }
    END {
      if (found != 1) exit 1
      print path
    }
  ' "${asset_map}"
}

write_npm_config() {
  local npmrc=$1
  require_environment AIT_NPM_TOKEN
  umask 077
  printf '//%s/:_authToken=%s\n' "${npm_registry_host}" "${AIT_NPM_TOKEN}" >"${npmrc}"
}

npm_package_rows() {
  local package_archive package_name package_version
  find "${assets}" -mindepth 1 -maxdepth 1 -type f -name '*.tgz' -print |
    LC_ALL=C sort |
    while IFS= read -r package_archive; do
      package_name=$(tar -xOf "${package_archive}" package/package.json | jq -er .name)
      package_version=$(tar -xOf "${package_archive}" package/package.json | jq -er .version)
      printf '%s\t%s\t%s\n' "${package_name}" "${package_version}" "${package_archive}"
    done
}

npm_provenance_policy() {
  local package_name=$1
  local archive_sha256=$2
  local repository_url=$3
  local expected_repository=https://github.com/${github_repository}
  local admitted_frozen_sha256

  case "${repository_url}" in
    "${expected_repository}" | "git+${expected_repository}.git")
      printf '%s\n' '--provenance'
      return 0
      ;;
  esac

  admitted_frozen_sha256=$(jq -er --arg package_name "${package_name}" '
    if .endpoints.npm.frozen_missing_repository_metadata.external_github_attestation_required == true then
      .endpoints.npm.frozen_missing_repository_metadata.archives[$package_name] // ""
    else
      ""
    end
  ' "${endpoint_config}")
  if [[ ${repository_url} == '' &&
    ${release_version} == 1.0.0-rc.3 &&
    ${source_commit} == ba368cf4d0750035345f14a8a91c22fb9e450260 &&
    ${admitted_frozen_sha256} == "${archive_sha256}" ]]; then
    printf '%s\n' '--provenance=false'
    return 0
  fi

  printf 'npm package repository metadata does not admit provenance: %s\n' \
    "${package_name}" >&2
  return 65
}

npm_publish_provenance_flag() {
  local package_name=$1
  local package_archive=$2
  local package_metadata repository_url archive_sha256
  package_metadata=$(tar -xOf "${package_archive}" package/package.json)
  repository_url=$(jq -r '
    if .repository == null then
      ""
    elif (.repository | type) == "string" then
      .repository
    elif (.repository | type) == "object" and
      ((.repository.url // "") | type) == "string" then
      (.repository.url // "")
    else
      error("package repository metadata has an unsupported shape")
    end
  ' <<<"${package_metadata}")
  archive_sha256=$(sha256_file "${package_archive}")
  npm_provenance_policy "${package_name}" "${archive_sha256}" "${repository_url}"
}

remove_matching_npm_prerelease_latest_tag() {
  local npmrc=$1
  local package_name=$2
  local package_version=$3
  local configured_tag=$4
  local tags latest_version
  if [[ ${configured_tag} == latest || ${package_version} != *-* ]]; then
    return 0
  fi
  tags=$(NPM_CONFIG_USERCONFIG="${npmrc}" npm dist-tag ls "${package_name}" \
    --registry "${npm_registry}")
  latest_version=$(printf '%s\n' "${tags}" | awk -F ': ' '
    $1 == "latest" {
      count += 1
      value = $2
    }
    END {
      if (count > 1) exit 65
      if (count == 1) print value
    }
  ')
  if [[ ${latest_version} == "${package_version}" ]]; then
    NPM_CONFIG_USERCONFIG="${npmrc}" npm dist-tag rm \
      "${package_name}" latest --registry "${npm_registry}" >/dev/null
  fi
}

validate_npm_dist_tags() {
  local metadata=$1
  local package_name=$2
  local package_version=$3
  local configured_tag=$4
  if ! jq -e --arg tag "${configured_tag}" --arg version "${package_version}" \
    '."dist-tags"[$tag] == $version' "${metadata}" >/dev/null; then
    printf 'npm RC dist-tag readback failed: %s@%s\n' \
      "${package_name}" "${package_version}" >&2
    return 65
  fi
  if [[ ${configured_tag} != latest ]] &&
    jq -e --arg version "${package_version}" \
      '."dist-tags".latest == $version' "${metadata}" >/dev/null; then
    printf 'npm prerelease remains the default latest tag: %s@%s\n' \
      "${package_name}" "${package_version}" >&2
    return 65
  fi
}

validate_npm_remote_state() {
  local require_published=${1:-false}
  local expected_names=${temporary_root}/expected-npm-names
  local actual_names=${temporary_root}/actual-npm-names
  local package_name package_version package_archive metadata status remote_integrity remote_shasum
  local configured_tag
  configured_tag=$(jq -er '.endpoints.npm.dist_tag' "${endpoint_config}")
  jq -r '.endpoints.npm.packages[]' "${endpoint_config}" | LC_ALL=C sort >"${expected_names}"
  npm_package_rows | awk -F '\t' '{print $1}' | LC_ALL=C sort >"${actual_names}"
  if ! diff -u "${expected_names}" "${actual_names}"; then
    printf 'npm staged package identities are not exact\n' >&2
    return 65
  fi
  while IFS=$'\t' read -r package_name package_version package_archive; do
    if [[ ${package_version} != "${release_version}" ]]; then
      printf 'npm staged package version drifted: %s\n' "${package_name}" >&2
      return 65
    fi
    metadata=${temporary_root}/npm-${package_name}.json
    status=$(curl --silent --show-error --location --output "${metadata}" --write-out '%{http_code}' \
      "${npm_registry}/${package_name}")
    case "${status}" in
      404)
        if [[ ${require_published} == true ]]; then
          printf 'npm package is still unpublished: %s@%s\n' \
            "${package_name}" "${release_version}" >&2
          return 65
        fi
        ;;
      200)
        if jq -e --arg version "${release_version}" '.versions[$version] == null' \
          "${metadata}" >/dev/null; then
          if [[ ${require_published} == true ]]; then
            printf 'npm package version is still unpublished: %s@%s\n' \
              "${package_name}" "${release_version}" >&2
            return 65
          fi
        else
          remote_integrity=$(jq -er --arg version "${release_version}" \
            '.versions[$version].dist.integrity' "${metadata}")
          remote_shasum=$(jq -er --arg version "${release_version}" \
            '.versions[$version].dist.shasum' "${metadata}")
          if [[ ${remote_integrity} != "$(sha512_integrity "${package_archive}")" ||
            ${remote_shasum} != "$(sha1_file "${package_archive}")" ]]; then
            printf 'npm already contains conflicting bytes: %s@%s\n' \
              "${package_name}" "${release_version}" >&2
            return 65
          fi
          if [[ ${require_published} == true ]]; then
            validate_npm_dist_tags "${metadata}" "${package_name}" \
              "${release_version}" "${configured_tag}"
          fi
        fi
        ;;
      *)
        printf 'npm registry preflight returned HTTP %s for %s\n' \
          "${status}" "${package_name}" >&2
        return 69
        ;;
    esac
  done < <(npm_package_rows)
}

validate_pypi_remote_state() {
  local require_published=${1:-false}
  local metadata=${temporary_root}/pypi.json
  local status remote_files expected_files filename remote_sha wheel
  status=$(curl --silent --show-error --location \
    --header 'Cache-Control: no-cache' --output "${metadata}" --write-out '%{http_code}' \
    "https://pypi.org/pypi/ait-native/json")
  if [[ ${status} != 200 ]] ||
    ! jq -e '.info.name == "ait-native" and (.releases["0.10.6"] | length) == 2' \
      "${metadata}" >/dev/null; then
    printf 'PyPI project identity or prior ownership lineage is unavailable\n' >&2
    return 65
  fi
  remote_files=$(jq -r --arg version "${python_version}" '.releases[$version] | length' "${metadata}")
  expected_files=$(find "${assets}" -mindepth 1 -maxdepth 1 -type f -name 'ait_native-*.whl' |
    wc -l | tr -d '[:space:]')
  case "${remote_files}" in
    0)
      if [[ ${require_published} == true ]]; then
        printf 'PyPI RC wheel set is not visible yet\n' >&2
        return 75
      fi
      ;;
    "${expected_files}")
      while IFS= read -r wheel; do
        filename=$(basename -- "${wheel}")
        remote_sha=$(jq -er --arg version "${python_version}" --arg filename "${filename}" '
          .releases[$version][] | select(.filename == $filename) | .digests.sha256
        ' "${metadata}")
        if [[ ${remote_sha} != "$(sha256_file "${wheel}")" ]]; then
          printf 'PyPI already contains conflicting bytes: %s\n' "${filename}" >&2
          return 65
        fi
      done < <(find "${assets}" -mindepth 1 -maxdepth 1 -type f -name 'ait_native-*.whl' |
        LC_ALL=C sort)
      ;;
    *)
      if [[ ${require_published} == true &&
        ${remote_files} =~ ^[0-9]+$ && ${remote_files} -lt ${expected_files} ]]; then
        printf 'PyPI RC wheel set is only partially visible\n' >&2
        return 75
      fi
      printf 'PyPI contains a partial RC wheel set\n' >&2
      return 65
      ;;
  esac
}

wait_for_pypi_remote_state() {
  local attempt=1
  local max_attempts=12
  local readback_status
  while ((attempt <= max_attempts)); do
    if validate_pypi_remote_state true; then
      return 0
    else
      readback_status=$?
    fi
    if [[ ${readback_status} != 75 ]]; then
      return "${readback_status}"
    fi
    if ((attempt == max_attempts)); then
      printf 'PyPI RC wheel set did not become fully visible after %s attempts\n' \
        "${max_attempts}" >&2
      return 65
    fi
    printf 'waiting for PyPI RC wheel-set visibility (%s/%s)\n' \
      "${attempt}" "${max_attempts}" >&2
    sleep 5
    attempt=$((attempt + 1))
  done
}

prepare_github_ssh() {
  local key_value=$1
  local key_path=$2
  local known_hosts=$3
  umask 077
  printf '%s\n' "${key_value}" >"${key_path}"
  chmod 0600 "${key_path}"
  ssh-keyscan -t ed25519 github.com >"${known_hosts}" 2>/dev/null
  local fingerprint
  fingerprint=$(ssh-keygen -lf "${known_hosts}" -E sha256 | awk '{print $2}')
  if [[ ${fingerprint} != 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU' ]]; then
    printf 'GitHub SSH host fingerprint is not exact\n' >&2
    return 65
  fi
}

validate_deploy_key() {
  local repository=$1
  local branch=$2
  local key_value=$3
  local label=$4
  local key_path=${temporary_root}/${label}.key
  local known_hosts=${temporary_root}/${label}.known-hosts
  local clone_root=${temporary_root}/${label}-clone
  prepare_github_ssh "${key_value}" "${key_path}" "${known_hosts}"
  GIT_SSH_COMMAND="ssh -i ${key_path} -o IdentitiesOnly=yes -o UserKnownHostsFile=${known_hosts}" \
    git clone --quiet --depth 1 --branch "${branch}" \
      "git@github.com:${repository}.git" "${clone_root}"
  git -C "${clone_root}" \
    -c user.name='AIT Native Release Preflight' \
    -c user.email='253238140+weita2026@users.noreply.github.com' \
    commit --allow-empty -m 'AIT Native deploy-key write preflight (not pushed)' >/dev/null
  GIT_SSH_COMMAND="ssh -i ${key_path} -o IdentitiesOnly=yes -o UserKnownHostsFile=${known_hosts}" \
    git -C "${clone_root}" push --dry-run origin "HEAD:refs/heads/${branch}" >/dev/null
}

validate_apt_signing_key() {
  require_environment AIT_APT_SIGNING_KEY_B64 AIT_APT_SIGNING_PASSPHRASE \
    AIT_APT_SIGNING_FINGERPRINT
  local expected_fingerprint
  expected_fingerprint=$(jq -er '.endpoints.apt.signing_fingerprint' "${endpoint_config}")
  if [[ ${AIT_APT_SIGNING_FINGERPRINT} != "${expected_fingerprint}" ]]; then
    printf 'apt signing fingerprint environment does not match endpoint configuration\n' >&2
    return 65
  fi
  local gnupg_root=${temporary_root}/gnupg
  mkdir -m 0700 "${gnupg_root}"
  printf '%s' "${AIT_APT_SIGNING_KEY_B64}" | base64 --decode >"${temporary_root}/apt-secret.asc"
  printf '%s' "${AIT_APT_SIGNING_PASSPHRASE}" >"${temporary_root}/apt-passphrase"
  chmod 0600 "${temporary_root}/apt-secret.asc" "${temporary_root}/apt-passphrase"
  GNUPGHOME="${gnupg_root}" gpg --batch --import "${temporary_root}/apt-secret.asc" >/dev/null 2>&1
  local actual_fingerprint
  actual_fingerprint=$(GNUPGHOME="${gnupg_root}" gpg --batch --with-colons \
    --list-secret-keys | awk -F: '$1 == "fpr" {print $10; exit}')
  if [[ ${actual_fingerprint} != "${expected_fingerprint}" ]]; then
    printf 'apt signing secret does not match its public fingerprint\n' >&2
    return 65
  fi
  printf 'ait-native apt signing preflight\n' >"${temporary_root}/apt-signing-test"
  GNUPGHOME="${gnupg_root}" gpg --batch --yes --pinentry-mode loopback \
    --passphrase-file "${temporary_root}/apt-passphrase" \
    --local-user "${expected_fingerprint}" --detach-sign \
    "${temporary_root}/apt-signing-test"
  GNUPGHOME="${gnupg_root}" gpg --batch --verify \
    "${temporary_root}/apt-signing-test.sig" "${temporary_root}/apt-signing-test" \
    >/dev/null 2>&1
}

validate_public_tag() {
  local tag_rows=${temporary_root}/tag-rows
  GIT_ASKPASS=/usr/bin/false GIT_TERMINAL_PROMPT=0 \
    git -c credential.helper= ls-remote --tags \
      "https://github.com/${github_repository}.git" \
      "${release_tag}" "${release_tag}^{}" >"${tag_rows}"
  if [[ $(wc -l <"${tag_rows}" | tr -d '[:space:]') != 2 ||
    $(awk -v ref="refs/tags/${release_tag}^{}" '$2 == ref {print $1}' "${tag_rows}") != \
      "${source_commit}" ]]; then
    printf 'public RC tag does not resolve to the exact frozen source commit\n' >&2
    return 65
  fi
}

run_authenticated_preflight_check() {
  local label=$1
  shift
  local exit_code
  printf 'authenticated endpoint preflight start: %s\n' "${label}"
  set +e
  (
    set -e
    "$@"
  )
  exit_code=$?
  set -e
  if ((exit_code != 0)); then
    printf 'authenticated endpoint preflight failed: %s (exit %s)\n' \
      "${label}" "${exit_code}" >&2
    return "${exit_code}"
  fi
  printf 'authenticated endpoint preflight pass: %s\n' "${label}"
}

validate_github_repository_token_identity() {
  local repository_record=${temporary_root}/repository.json
  curl --fail --silent --show-error --location \
    --header "Authorization: Bearer ${AIT_GITHUB_TOKEN}" \
    --header 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/${github_repository}" --output "${repository_record}"
  jq -e '
    .full_name == "weita2026/ait-native" and
    .private == false
  ' "${repository_record}" >/dev/null
}

validate_npm_authenticated_publisher() {
  local npmrc=${temporary_root}/npmrc
  local npm_user
  write_npm_config "${npmrc}"
  if ! npm_user=$(NPM_CONFIG_USERCONFIG="${npmrc}" \
    npm whoami --registry "${npm_registry}" 2>/dev/null); then
    printf 'npm credential does not authenticate with the declared registry\n' >&2
    return 65
  fi
  if [[ -z ${npm_user} ]]; then
    printf 'npm credential does not identify an authenticated publisher\n' >&2
    return 65
  fi
}

require_preflight_receipt() {
  local preflight=${evidence_root}/ait-release.endpoint-preflight.json
  require_regular_file "${preflight}" 'endpoint preflight receipt'
  if ! jq -e \
    --arg release_id "${release_id}" \
    --arg version "${release_version}" \
    --arg tag "${release_tag}" \
    --arg stage_receipt_sha256 "$(sha256_file "${stage_receipt}")" '
      .contract == "ait.release.family.endpoint-preflight/v1" and
      .status == "pass" and
      .release_id == $release_id and
      .version == $version and
      .tag == $tag and
      .stage_receipt_sha256 == $stage_receipt_sha256 and
      .mutation.registry_write == false and
      .mutation.github_release_write == false and
      .mutation.endpoint_repository_write == false and
      .mutation.artifact_rebuild == false and
      .mutation.component_rebuild == false and
      .mutation.tag_write == false
    ' "${preflight}" >/dev/null; then
    printf 'endpoint preflight receipt is not exact\n' >&2
    return 65
  fi
  require_environment AIT_STAGE_ATTESTATION_VERIFIED
  if [[ ${AIT_STAGE_ATTESTATION_VERIFIED} != true ]]; then
    printf 'release-stage attestation has not been verified\n' >&2
    return 65
  fi
}

validate_github_release_state() {
  local require_complete=${1:-false}
  local release_record=${temporary_root}/github-release.json
  local asset_map=${temporary_root}/github-asset-map
  local expected_names=${temporary_root}/github-expected-names
  local remote_names=${temporary_root}/github-existing-names
  local status name digest local_path
  github_release_asset_map "${asset_map}"
  awk -F '\t' '{print $1}' "${asset_map}" >"${expected_names}"
  status=$(curl --silent --show-error --location --output "${release_record}" --write-out '%{http_code}' \
    --header "Authorization: Bearer ${AIT_GITHUB_TOKEN}" \
    --header 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/${github_repository}/releases/tags/${release_tag}")
  case "${status}" in
    404) ;;
    200)
      if ! jq -e --arg tag "${release_tag}" '
        .tag_name == $tag and .draft == false and .prerelease == true
      ' "${release_record}" >/dev/null; then
        printf 'existing GitHub Release route conflicts with the RC contract\n' >&2
        return 65
      fi
      jq -r '.assets[].name' "${release_record}" | LC_ALL=C sort >"${remote_names}"
      if ! awk 'NR == FNR {allowed[$0] = 1; next} !($0 in allowed) {print; bad = 1} END {exit bad}' \
        "${expected_names}" "${remote_names}"; then
        printf 'existing GitHub Release contains an unexpected asset\n' >&2
        return 65
      fi
      if [[ ${require_complete} == true ]] &&
        ! diff -u "${expected_names}" "${remote_names}"; then
        printf 'existing GitHub Release asset inventory is incomplete\n' >&2
        return 65
      fi
      while IFS=$'\t' read -r name digest; do
        if ! local_path=$(github_release_local_path "${asset_map}" "${name}"); then
          printf 'existing GitHub Release asset has no staged source: %s\n' "${name}" >&2
          return 65
        fi
        require_regular_file "${local_path}" 'existing GitHub Release asset source'
        if [[ ! ${digest} =~ ^sha256:[0-9a-f]{64}$ ||
          ${digest} != "sha256:$(sha256_file "${local_path}")" ]]; then
          printf 'existing GitHub Release asset conflicts: %s\n' "${name}" >&2
          return 65
        fi
      done < <(jq -r '.assets[] | [.name, .digest] | @tsv' "${release_record}")
      ;;
    *)
      printf 'GitHub Release preflight returned HTTP %s\n' "${status}" >&2
      return 69
      ;;
  esac
}

verify_apt_repository_clone() {
  local clone_root=$1
  local suite=$2
  local component=$3
  local require_candidate_assets=${4:-true}
  local verify_root
  local apt_root apt_log search_output
  local asset name relative packages_path expected_sha matched_count
  local -a apt_options
  if [[ ${require_candidate_assets} != true && ${require_candidate_assets} != false ]]; then
    printf 'apt candidate verification selector is invalid\n' >&2
    return 64
  fi
  if [[ ${require_candidate_assets} == true ]]; then
    while IFS= read -r asset; do
      name=$(basename -- "${asset}")
      case "${name}" in
        ait-native_*) relative=pool/main/a/ait-native/${name} ;;
        ait-runner_*) relative=pool/main/a/ait-runner/${name} ;;
        *)
          printf 'unexpected apt package identity: %s\n' "${name}" >&2
          return 65
          ;;
      esac
      require_regular_file "${clone_root}/${relative}" 'apt repository package'
      expected_sha=$(sha256_file "${asset}")
      if [[ $(sha256_file "${clone_root}/${relative}") != "${expected_sha}" ]]; then
        printf 'apt repository package digest drifted: %s\n' "${name}" >&2
        return 65
      fi
      matched_count=0
      for packages_path in \
        "${clone_root}/dists/${suite}/${component}/binary-amd64/Packages" \
        "${clone_root}/dists/${suite}/${component}/binary-arm64/Packages"; do
        require_regular_file "${packages_path}" 'apt Packages index'
        if awk -v filename="${relative}" -v sha="${expected_sha}" '
          BEGIN {RS=""; FS="\n"}
          {
            found_filename = 0
            found_sha = 0
            for (i = 1; i <= NF; i++) {
              if ($i == "Filename: " filename) found_filename = 1
              if ($i == "SHA256: " sha) found_sha = 1
            }
            if (found_filename && found_sha) matched = 1
          }
          END {exit !matched}
        ' "${packages_path}"; then
          matched_count=$((matched_count + 1))
        fi
      done
      if [[ ${matched_count} != 1 ]]; then
        printf 'apt Packages indexes do not select exactly one copy of %s\n' "${name}" >&2
        return 65
      fi
    done < <(find "${assets}" -mindepth 1 -maxdepth 1 -type f -name '*.deb' |
      LC_ALL=C sort)
  fi

  require_regular_file "${clone_root}/dists/${suite}/Release" 'apt Release metadata'
  require_regular_file "${clone_root}/dists/${suite}/InRelease" 'apt InRelease signature'
  require_regular_file "${clone_root}/dists/${suite}/Release.gpg" 'apt detached signature'
  require_regular_file "${clone_root}/ait-native-archive-keyring.gpg" 'apt archive keyring'
  verify_root=$(mktemp -d "${temporary_root}/apt-verify.XXXXXX")
  mkdir -m 0700 "${verify_root}/gnupg"
  GNUPGHOME="${verify_root}/gnupg" gpg --batch --import \
    "${clone_root}/ait-native-archive-keyring.gpg" >/dev/null 2>&1
  if [[ $(GNUPGHOME="${verify_root}/gnupg" gpg --batch --with-colons --list-keys |
      awk -F: '$1 == "fpr" {print $10; exit}') != "${AIT_APT_SIGNING_FINGERPRINT}" ]]; then
    printf 'apt repository public key fingerprint drifted\n' >&2
    return 65
  fi
  GNUPGHOME="${verify_root}/gnupg" gpg --batch --verify \
    "${clone_root}/dists/${suite}/InRelease" >/dev/null 2>&1
  GNUPGHOME="${verify_root}/gnupg" gpg --batch --verify \
    "${clone_root}/dists/${suite}/Release.gpg" \
    "${clone_root}/dists/${suite}/Release" >/dev/null 2>&1
  GNUPGHOME="${verify_root}/gnupg" gpg --batch --decrypt \
    "${clone_root}/dists/${suite}/InRelease" >"${verify_root}/release" 2>/dev/null
  if ! cmp "${verify_root}/release" "${clone_root}/dists/${suite}/Release"; then
    printf 'apt inline and detached Release metadata differ\n' >&2
    return 65
  fi
  awk '
    /^SHA256:/ {in_sha = 1; next}
    in_sha && /^[^ ]/ {exit}
    in_sha && /^ / {print $1 "  " $3}
  ' "${clone_root}/dists/${suite}/Release" >"${verify_root}/sha256sums"
  if [[ $(wc -l <"${verify_root}/sha256sums" | tr -d '[:space:]') != 4 ]] ||
    ! (cd "${clone_root}/dists/${suite}" && sha256sum -c "${verify_root}/sha256sums" >/dev/null); then
    printf 'apt Release checksum readback failed\n' >&2
    return 65
  fi

  for name in apt-get apt-cache; do
    if ! command -v "${name}" >/dev/null 2>&1; then
      printf 'required apt searchability command is unavailable: %s\n' "${name}" >&2
      return 69
    fi
  done
  apt_root=$(mktemp -d "${temporary_root}/apt-client.XXXXXX")
  mkdir -p "${apt_root}/lists/partial" "${apt_root}/cache/archives/partial"
  : >"${apt_root}/status"
  cp "${clone_root}/ait-native-archive-keyring.gpg" "${apt_root}/archive-keyring.gpg"
  chmod 0644 "${apt_root}/status" "${apt_root}/archive-keyring.gpg"
  printf 'deb [signed-by=%s] file:%s %s %s\n' \
    "${apt_root}/archive-keyring.gpg" "${clone_root}" "${suite}" "${component}" \
    >"${apt_root}/sources.list"
  apt_options=(
    -o "Dir::Etc::sourcelist=${apt_root}/sources.list"
    -o 'Dir::Etc::sourceparts=-'
    -o "Dir::State::status=${apt_root}/status"
    -o "Dir::State::lists=${apt_root}/lists"
    -o "Dir::Cache=${apt_root}/cache"
    -o 'APT::Get::List-Cleanup=0'
    -o 'Acquire::Languages=none'
    -o 'Debug::NoLocking=1'
  )
  apt_log=${apt_root}/update.log
  if ! apt-get "${apt_options[@]}" update >"${apt_log}" 2>&1; then
    printf 'apt client could not update from the signed repository\n' >&2
    sed -n '1,160p' "${apt_log}" >&2
    return 65
  fi
  for name in ait-native ait-runner; do
    search_output=$(apt-cache "${apt_options[@]}" search --names-only "^${name}$")
    if ! awk -v package="${name}" '
      $1 == package && $2 == "-" {found = 1}
      END {exit !found}
    ' <<<"${search_output}"; then
      printf 'apt-cache search did not discover %s\n' "${name}" >&2
      return 65
    fi
  done
}

inspect_oci_image() {
  local component=$1
  local image=$2
  local require_published=$3
  local context=${publication_stage}/oci/${component}
  local immutable_tag
  local reference manifest digest platform_count architecture child_digest
  local container extract_root actual_binary image_record expected_license
  require_real_directory "${context}" "${component} OCI context"
  immutable_tag=$(jq -er '.endpoints.oci.immutable_tag' "${endpoint_config}")
  reference=${image}:${immutable_tag}
  if ! manifest=$(docker buildx imagetools inspect \
      --format '{{json .Manifest}}' "${reference}" 2>/dev/null); then
    if [[ ${require_published} == true ]]; then
      printf 'immutable OCI image is still unpublished: %s\n' "${reference}" >&2
      return 65
    fi
    jq -n --arg component "${component}" --arg image "${image}" \
      '{component: $component, image: $image, present: false, digest: null}'
    return 0
  fi
  digest=$(jq -er '.digest' <<<"${manifest}")
  if [[ ! ${digest} =~ ^sha256:[0-9a-f]{64}$ ]]; then
    printf 'OCI manifest digest is invalid: %s\n' "${reference}" >&2
    return 65
  fi
  platform_count=$(jq -r '[.manifests[] | select(.platform.os == "linux") |
    select(.platform.architecture == "amd64" or .platform.architecture == "arm64")] |
    length' <<<"${manifest}")
  if [[ ${platform_count} != 2 ]] ||
    ! jq -e '
      ([.manifests[] | select(.platform.os == "linux") |
        select(.platform.architecture == "amd64" or .platform.architecture == "arm64") |
        .platform.architecture] | sort) == ["amd64", "arm64"] and
      ([.manifests[] | select(.platform.os != "unknown")] | length) == 2
    ' <<<"${manifest}" >/dev/null; then
    printf 'OCI manifest platform inventory is not exact: %s\n' "${reference}" >&2
    return 65
  fi
  for architecture in amd64 arm64; do
    child_digest=$(jq -er --arg architecture "${architecture}" '
      .manifests[] |
      select(.platform.os == "linux" and .platform.architecture == $architecture) |
      .digest
    ' <<<"${manifest}")
    docker pull --quiet --platform "linux/${architecture}" \
      "${image}@${child_digest}" >/dev/null
    container=$(docker create --platform "linux/${architecture}" \
      "${image}@${child_digest}")
    extract_root=$(mktemp -d "${temporary_root}/${component}-${architecture}.XXXXXX")
    docker cp "${container}:/usr/local/bin/${component}" \
      "${extract_root}/${component}"
    docker cp "${container}:/usr/share/ait-native/provenance.json" \
      "${extract_root}/provenance.json"
    mkdir "${extract_root}/licenses"
    docker cp "${container}:/usr/share/licenses/${component}/." \
      "${extract_root}/licenses"
    docker rm -f "${container}" >/dev/null
    actual_binary=${extract_root}/${component}
    if ! cmp "${context}/bin/${architecture}/${component}" "${actual_binary}" ||
      ! cmp "${context}/provenance.json" "${extract_root}/provenance.json" ||
      ! diff -r "${context}/licenses" "${extract_root}/licenses" >/dev/null; then
      printf 'OCI filesystem readback differs from the frozen context: %s/%s\n' \
        "${component}" "${architecture}" >&2
      return 65
    fi
    image_record=${extract_root}/image.json
    docker image inspect "${image}@${child_digest}" >"${image_record}"
    expected_license=Apache-2.0
    if [[ ${component} == ait-server ]]; then
      expected_license=AGPL-3.0-only
    fi
    if ! jq -e \
      --arg component "${component}" \
      --arg license "${expected_license}" \
      --arg source_commit "${source_commit}" \
      --arg version "${release_version}" '
        length == 1 and
        .[0].Config.User == "65532:65532" and
        .[0].Config.Labels["org.opencontainers.image.source"] ==
          "https://github.com/weita2026/ait-native" and
        .[0].Config.Labels["org.opencontainers.image.title"] == $component and
        .[0].Config.Labels["org.opencontainers.image.licenses"] == $license and
        .[0].Config.Labels["org.opencontainers.image.revision"] == $source_commit and
        .[0].Config.Labels["org.opencontainers.image.version"] == $version and
        .[0].Config.Entrypoint == ["/usr/local/bin/" + $component] and
        if $component == "ait-server" then
          .[0].Config.Cmd == ["run", "--init-if-missing", "--defer-ci-admission"] and
          any(.[0].Config.Env[]; . == "AITSERVER_LISTEN=0.0.0.0:8088") and
          any(.[0].Config.Env[]; . == "AIT_NATIVE_SERVER_DATA=/var/lib/ait/server-data")
        else
          ((.[0].Config.Cmd == null) or (.[0].Config.Cmd == [])) and
          .[0].Config.WorkingDir == "/workspace"
        end
      ' "${image_record}" >/dev/null; then
      printf 'OCI image configuration readback failed: %s/%s\n' \
        "${component}" "${architecture}" >&2
      return 65
    fi
  done
  jq -n \
    --arg component "${component}" \
    --arg image "${image}" \
    --arg digest "${digest}" \
    '{component: $component, image: $image, present: true, digest: $digest}'
}

inspect_oci_remote_state() {
  local require_published=$1
  local rows=${temporary_root}/oci-rows.jsonl
  local component image
  : >"${rows}"
  while IFS=$'\t' read -r component image; do
    inspect_oci_image "${component}" "${image}" "${require_published}" >>"${rows}"
  done < <(jq -r '
    [.endpoints.oci.images[]] |
    ["ait-server", .[0]], ["ait-runner", .[1]] | @tsv
  ' "${endpoint_config}")
  jq -s \
    --arg contract 'ait.release.endpoint.oci-state/v1' \
    --arg release_id "${release_id}" \
    --arg version "${release_version}" \
    --argjson require_published "${require_published}" '
      {
        contract: $contract,
        release_id: $release_id,
        version: $version,
        require_published: $require_published,
        images: (map({key: .component, value: {
          image: .image, present: .present, digest: .digest
        }}) | from_entries),
        component_rebuild: false,
        filesystem_readback: all(.[]; if .present then true else ($require_published | not) end)
      }
    ' "${rows}" >"${evidence_root}/oci-state.json"
}

case "${mode}" in
  preflight-oci)
    if ! command -v docker >/dev/null 2>&1; then
      printf 'required OCI inspector command is unavailable: docker\n' >&2
      exit 69
    fi
    inspect_oci_remote_state false
    ;;

  preflight)
    require_environment AIT_GITHUB_TOKEN AIT_NPM_TOKEN AIT_HOMEBREW_DEPLOY_KEY \
      AIT_APT_REPO_DEPLOY_KEY AIT_APT_SIGNING_KEY_B64 \
      AIT_APT_SIGNING_PASSPHRASE AIT_APT_SIGNING_FINGERPRINT \
      AIT_PYPI_OIDC_PREFLIGHT AIT_GHCR_PREFLIGHT
    if [[ ${AIT_PYPI_OIDC_PREFLIGHT} != pass || ${AIT_GHCR_PREFLIGHT} != pass ]]; then
      printf 'PyPI OIDC or GHCR authenticated preflight did not pass\n' >&2
      exit 65
    fi
    run_authenticated_preflight_check \
      'GitHub repository token identity' validate_github_repository_token_identity
    run_authenticated_preflight_check \
      'npm authenticated publisher identity' validate_npm_authenticated_publisher
    run_authenticated_preflight_check \
      'npm staged identities and remote state' validate_npm_remote_state
    run_authenticated_preflight_check \
      'PyPI project lineage and remote state' validate_pypi_remote_state
    run_authenticated_preflight_check \
      'public RC tag identity' validate_public_tag
    run_authenticated_preflight_check \
      'GitHub Release restart state' validate_github_release_state
    run_authenticated_preflight_check \
      'Homebrew deploy key dry-run' validate_deploy_key \
      "$(jq -er '.endpoints.homebrew.repository' "${endpoint_config}")" \
      "$(jq -er '.endpoints.homebrew.branch' "${endpoint_config}")" \
      "${AIT_HOMEBREW_DEPLOY_KEY}" homebrew
    run_authenticated_preflight_check \
      'APT deploy key dry-run' validate_deploy_key \
      "$(jq -er '.endpoints.apt.repository' "${endpoint_config}")" \
      "$(jq -er '.endpoints.apt.branch' "${endpoint_config}")" \
      "${AIT_APT_REPO_DEPLOY_KEY}" apt
    run_authenticated_preflight_check \
      'APT signing key' validate_apt_signing_key
    jq -n \
      --arg contract 'ait.release.family.endpoint-preflight/v1' \
      --arg status 'pass' \
      --arg release_id "${release_id}" \
      --arg version "${release_version}" \
      --arg tag "${release_tag}" \
      --arg stage_receipt_sha256 "$(sha256_file "${stage_receipt}")" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          version: $version,
          tag: $tag,
          stage_receipt_sha256: $stage_receipt_sha256,
          checks: {
            github_repository_token_identity: "pass",
            public_tag_exact: "pass",
            github_release_state: "pass",
            pypi_oidc_project_lineage_and_remote_state: "pass",
            npm_authenticated_creation_path_and_remote_state: "pass",
            homebrew_deploy_key_write_dry_run: "pass",
            apt_deploy_key_write_dry_run: "pass",
            apt_signing_key: "pass",
            ghcr_authenticated_creation_or_exact_restart_path: "pass"
          },
          mutation: {
            credentials_loaded: true,
            registry_write: false,
            github_release_write: false,
            endpoint_repository_write: false,
            artifact_rebuild: false,
            component_rebuild: false,
            tag_write: false
          },
          next_action: "attest_then_publish_exact_stage"
        }
      ' >"${evidence_root}/ait-release.endpoint-preflight.json"
    ;;

  publish-github)
    require_environment AIT_GITHUB_TOKEN
    require_preflight_receipt
    validate_github_release_state
    release_record=${temporary_root}/github-release-current.json
    status=$(curl --silent --show-error --location --output "${release_record}" --write-out '%{http_code}' \
      --header "Authorization: Bearer ${AIT_GITHUB_TOKEN}" \
      --header 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/${github_repository}/releases/tags/${release_tag}")
    notes=${temporary_root}/release-notes.md
    cat >"${notes}" <<'NOTES'
# AIT Native 1.0.0 RC 3

This prerelease promotes the exact protected `v1.0.0-rc.3` family bytes.
It provides the language-neutral `ait` command and an inactive-by-default
`ait-server`; Python, Node.js, .NET, PHP, C, C++, Java, mixed-language, and
non-code repositories use the same explicit workflow.

Shortest workflow after installation:

```text
cd <repository>
ait init
```

Then ask an AIT-aware coding agent to make the change. The generated
`AGENTS.md` block directs Plan binding, Task worktree use, Snapshot creation,
validation, and local land.

The attached `SHA256SUMS`, package receipts, protected-promotion evidence, and
GitHub attestations bind every downloadable byte. WinGet remains on the RC
validation-manifest route and is not submitted to the stable community catalog.
NOTES
    if [[ ${status} == 404 ]]; then
      GH_TOKEN="${AIT_GITHUB_TOKEN}" gh release create "${release_tag}" \
        --repo "${github_repository}" \
        --title 'AIT Native 1.0.0 RC 3' \
        --notes-file "${notes}" \
        --prerelease \
        --verify-tag
    elif [[ ${status} != 200 ]]; then
      printf 'GitHub Release creation preflight returned HTTP %s\n' "${status}" >&2
      exit 69
    fi
    GH_TOKEN="${AIT_GITHUB_TOKEN}" gh release edit "${release_tag}" \
      --repo "${github_repository}" \
      --title 'AIT Native 1.0.0 RC 3' \
      --notes-file "${notes}" \
      --prerelease
    asset_map=${temporary_root}/github-asset-map
    github_release_asset_map "${asset_map}"
    remote_names=${temporary_root}/github-remote-names
    GH_TOKEN="${AIT_GITHUB_TOKEN}" gh api \
      "repos/${github_repository}/releases/tags/${release_tag}" \
      --jq '.assets[].name' | LC_ALL=C sort >"${remote_names}"
    missing_assets=()
    while IFS=$'\t' read -r name local_asset; do
      if ! grep -Fx "${name}" "${remote_names}" >/dev/null; then
        missing_assets+=("${local_asset}")
      fi
    done <"${asset_map}"
    if (( ${#missing_assets[@]} > 0 )); then
      GH_TOKEN="${AIT_GITHUB_TOKEN}" gh release upload "${release_tag}" \
        --repo "${github_repository}" "${missing_assets[@]}"
    fi
    release_record=${temporary_root}/github-release-published.json
    GH_TOKEN="${AIT_GITHUB_TOKEN}" gh api \
      "repos/${github_repository}/releases/tags/${release_tag}" >"${release_record}"
    expected_count=$(wc -l <"${asset_map}" | tr -d '[:space:]')
    if [[ $(jq -r '.assets | length' "${release_record}") != "${expected_count}" ]]; then
      printf 'GitHub Release asset count is incomplete after upload\n' >&2
      exit 65
    fi
    while IFS=$'\t' read -r name digest; do
      if ! local_path=$(github_release_local_path "${asset_map}" "${name}"); then
        printf 'GitHub Release asset has no staged source after upload: %s\n' "${name}" >&2
        exit 65
      fi
      if [[ ${digest} != "sha256:$(sha256_file "${local_path}")" ]]; then
        printf 'GitHub Release digest readback failed: %s\n' "${name}" >&2
        exit 65
      fi
    done < <(jq -r '.assets[] | [.name, .digest] | @tsv' "${release_record}")
    jq -n \
      --arg contract 'ait.release.endpoint.github/v1' \
      --arg status 'published_and_read_back' \
      --arg release_id "${release_id}" \
      --arg tag "${release_tag}" \
      --arg url "$(jq -er .html_url "${release_record}")" \
      --argjson asset_count "${expected_count}" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          tag: $tag,
          url: $url,
          prerelease: true,
          asset_count: $asset_count,
          digest_readback: true,
          component_rebuild: false
        }
      ' >"${evidence_root}/github.json"
    ;;

  publish-pypi)
    require_preflight_receipt
    wait_for_pypi_remote_state
    wheel_count=$(find "${assets}" -mindepth 1 -maxdepth 1 -type f \
      -name 'ait_native-*.whl' | wc -l | tr -d '[:space:]')
    jq -n \
      --arg contract 'ait.release.endpoint.pypi/v1' \
      --arg status 'published_and_read_back' \
      --arg release_id "${release_id}" \
      --arg version "${python_version}" \
      --argjson wheel_count "${wheel_count}" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          version: $version,
          wheel_count: $wheel_count,
          trusted_publisher: true,
          digest_readback: true,
          component_rebuild: false
        }
      ' >"${evidence_root}/pypi.json"
    ;;

  publish-npm)
    require_environment AIT_NPM_TOKEN
    require_preflight_receipt
    npmrc=${temporary_root}/npmrc
    write_npm_config "${npmrc}"
    validate_npm_remote_state
    npm_rows=${temporary_root}/npm-rows
    npm_package_rows >"${npm_rows}"
    while IFS=$'\t' read -r package_name package_version package_archive; do
      if [[ ${package_name} == ait-native ]]; then
        continue
      fi
      if ! curl --fail --silent --show-error \
        "${npm_registry}/${package_name}/${release_version}" >/dev/null 2>&1; then
        npm_provenance_flag=$(npm_publish_provenance_flag \
          "${package_name}" "${package_archive}")
        if [[ ${npm_provenance_flag} == --provenance=false ]]; then
          printf 'using exact frozen npm metadata exception: %s@%s\n' \
            "${package_name}" "${package_version}" >&2
        fi
        NPM_CONFIG_USERCONFIG="${npmrc}" npm publish "${package_archive}" \
          --registry "${npm_registry}" --tag rc --access public \
          "${npm_provenance_flag}"
      fi
    done <"${npm_rows}"
    while IFS=$'\t' read -r package_name package_version package_archive; do
      if [[ ${package_name} != ait-native ]]; then
        continue
      fi
      if ! curl --fail --silent --show-error \
        "${npm_registry}/${package_name}/${release_version}" >/dev/null 2>&1; then
        npm_provenance_flag=$(npm_publish_provenance_flag \
          "${package_name}" "${package_archive}")
        if [[ ${npm_provenance_flag} == --provenance=false ]]; then
          printf 'using exact frozen npm metadata exception: %s@%s\n' \
            "${package_name}" "${package_version}" >&2
        fi
        NPM_CONFIG_USERCONFIG="${npmrc}" npm publish "${package_archive}" \
          --registry "${npm_registry}" --tag rc --access public \
          "${npm_provenance_flag}"
      fi
    done <"${npm_rows}"
    while IFS=$'\t' read -r package_name package_version package_archive; do
      NPM_CONFIG_USERCONFIG="${npmrc}" npm dist-tag add \
        "${package_name}@${package_version}" \
        "$(jq -er '.endpoints.npm.dist_tag' "${endpoint_config}")" \
        --registry "${npm_registry}" >/dev/null
      remove_matching_npm_prerelease_latest_tag "${npmrc}" \
        "${package_name}" "${package_version}" \
        "$(jq -er '.endpoints.npm.dist_tag' "${endpoint_config}")"
    done <"${npm_rows}"
    validate_npm_remote_state true
    package_count=$(wc -l <"${npm_rows}" | tr -d '[:space:]')
    jq -n \
      --arg contract 'ait.release.endpoint.npm/v1' \
      --arg status 'published_and_read_back' \
      --arg release_id "${release_id}" \
      --arg version "${release_version}" \
      --arg dist_tag "$(jq -er '.endpoints.npm.dist_tag' "${endpoint_config}")" \
      --argjson package_count "${package_count}" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          version: $version,
          dist_tag: $dist_tag,
          package_count: $package_count,
          digest_readback: true,
          external_github_attestation: true,
          npm_registry_provenance: false,
          component_rebuild: false
        }
      ' >"${evidence_root}/npm.json"
    ;;

  publish-homebrew)
    require_environment AIT_HOMEBREW_DEPLOY_KEY
    require_preflight_receipt
    require_regular_file "${evidence_root}/github.json" 'GitHub endpoint receipt'
    repository=$(jq -er '.endpoints.homebrew.repository' "${endpoint_config}")
    branch=$(jq -er '.endpoints.homebrew.branch' "${endpoint_config}")
    formula_path=$(jq -er '.endpoints.homebrew.formula_path' "${endpoint_config}")
    key_path=${temporary_root}/homebrew.key
    known_hosts=${temporary_root}/homebrew.known-hosts
    clone_root=${temporary_root}/homebrew-repository
    prepare_github_ssh "${AIT_HOMEBREW_DEPLOY_KEY}" "${key_path}" "${known_hosts}"
    GIT_SSH_COMMAND="ssh -i ${key_path} -o IdentitiesOnly=yes -o UserKnownHostsFile=${known_hosts}" \
      git clone --quiet --depth 1 --branch "${branch}" \
        "git@github.com:${repository}.git" "${clone_root}"
    mkdir -p "${clone_root}/$(dirname -- "${formula_path}")"
    cp "${assets}/ait-native-rc.rb" "${clone_root}/${formula_path}"
    if ! git -C "${clone_root}" diff --quiet -- "${formula_path}" ||
      [[ -n $(git -C "${clone_root}" status --porcelain --untracked-files=all) ]]; then
      git -C "${clone_root}" add -- "${formula_path}"
      git -C "${clone_root}" \
        -c user.name='AIT Native Release' \
        -c user.email='253238140+weita2026@users.noreply.github.com' \
        commit -m "Publish ait-native ${release_version} formula" >/dev/null
      GIT_SSH_COMMAND="ssh -i ${key_path} -o IdentitiesOnly=yes -o UserKnownHostsFile=${known_hosts}" \
        git -C "${clone_root}" push origin "HEAD:refs/heads/${branch}"
    fi
    published_commit=$(git -C "${clone_root}" rev-parse HEAD)
    readback_root=${temporary_root}/homebrew-public-readback
    GIT_ASKPASS=/usr/bin/false GIT_TERMINAL_PROMPT=0 \
      git -c credential.helper= clone --quiet --depth 1 --branch "${branch}" \
        "https://github.com/${repository}.git" "${readback_root}"
    if [[ $(git -C "${readback_root}" rev-parse HEAD) != "${published_commit}" ]] ||
      ! cmp "${assets}/ait-native-rc.rb" "${readback_root}/${formula_path}"; then
      printf 'Homebrew formula readback differs from the frozen formula\n' >&2
      exit 65
    fi
    remote_url="https://raw.githubusercontent.com/${repository}/${published_commit}/${formula_path}"
    jq -n \
      --arg contract 'ait.release.endpoint.homebrew/v1' \
      --arg status 'published_and_read_back' \
      --arg release_id "${release_id}" \
      --arg formula_sha256 "$(sha256_file "${assets}/ait-native-rc.rb")" \
      --arg url "${remote_url}" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          formula_sha256: $formula_sha256,
          url: $url,
          stable_formula_mutation: false,
          digest_readback: true,
          component_rebuild: false
        }
      ' >"${evidence_root}/homebrew.json"
    ;;

  publish-apt)
    require_environment AIT_APT_REPO_DEPLOY_KEY AIT_APT_SIGNING_KEY_B64 \
      AIT_APT_SIGNING_PASSPHRASE AIT_APT_SIGNING_FINGERPRINT
    require_preflight_receipt
    require_regular_file "${evidence_root}/github.json" 'GitHub endpoint receipt'
    for command in apt-cache apt-get dpkg-scanpackages gpg gzip md5sum sha1sum sha256sum; do
      if ! command -v "${command}" >/dev/null 2>&1; then
        printf 'required apt publisher command is unavailable: %s\n' "${command}" >&2
        exit 69
      fi
    done
    validate_apt_signing_key
    repository=$(jq -er '.endpoints.apt.repository' "${endpoint_config}")
    branch=$(jq -er '.endpoints.apt.branch' "${endpoint_config}")
    suite=$(jq -er '.endpoints.apt.suite' "${endpoint_config}")
    component=$(jq -er '.endpoints.apt.component' "${endpoint_config}")
    key_path=${temporary_root}/apt.key
    known_hosts=${temporary_root}/apt.known-hosts
    clone_root=${temporary_root}/apt-repository
    prepare_github_ssh "${AIT_APT_REPO_DEPLOY_KEY}" "${key_path}" "${known_hosts}"
    GIT_SSH_COMMAND="ssh -i ${key_path} -o IdentitiesOnly=yes -o UserKnownHostsFile=${known_hosts}" \
      git clone --quiet --depth 1 --branch "${branch}" \
        "git@github.com:${repository}.git" "${clone_root}"
    apt_update_required=true
    if [[ -f ${clone_root}/dists/${suite}/InRelease ]]; then
      verify_apt_repository_clone "${clone_root}" "${suite}" "${component}" false
      candidate_complete=true
      while IFS= read -r asset; do
        name=$(basename -- "${asset}")
        case "${name}" in
          ait-native_*) relative=pool/main/a/ait-native/${name} ;;
          ait-runner_*) relative=pool/main/a/ait-runner/${name} ;;
          *)
            printf 'unexpected apt package identity: %s\n' "${name}" >&2
            exit 65
            ;;
        esac
        candidate_path=${clone_root}/${relative}
        if [[ -e ${candidate_path} || -L ${candidate_path} ]]; then
          require_regular_file "${candidate_path}" 'existing apt candidate package'
          if [[ $(sha256_file "${candidate_path}") != $(sha256_file "${asset}") ]]; then
            printf 'existing apt candidate package conflicts: %s\n' "${name}" >&2
            exit 65
          fi
        else
          candidate_complete=false
        fi
      done < <(find "${assets}" -mindepth 1 -maxdepth 1 -type f -name '*.deb' |
        LC_ALL=C sort)
      candidate_check=${temporary_root}/apt-candidate-check.log
      if [[ ${candidate_complete} == true ]] &&
        verify_apt_repository_clone "${clone_root}" "${suite}" "${component}" true \
          >"${candidate_check}" 2>&1; then
        apt_update_required=false
      fi
    else
      if [[ -e ${clone_root}/pool || -e ${clone_root}/dists ||
        -e ${clone_root}/ait-native-archive-keyring.gpg ||
        -e ${clone_root}/ait-native-archive-keyring.asc ]]; then
        printf 'apt repository contains an incomplete prior publication\n' >&2
        exit 65
      fi
    fi
    if [[ ${apt_update_required} == true ]]; then
      mkdir -p "${clone_root}/pool/main/a/ait-native" \
        "${clone_root}/pool/main/a/ait-runner" \
        "${clone_root}/dists/${suite}/${component}/binary-amd64" \
        "${clone_root}/dists/${suite}/${component}/binary-arm64"
      while IFS= read -r asset; do
        name=$(basename -- "${asset}")
        case "${name}" in
          ait-native_*) relative=pool/main/a/ait-native/${name} ;;
          ait-runner_*) relative=pool/main/a/ait-runner/${name} ;;
          *)
            printf 'unexpected apt package identity: %s\n' "${name}" >&2
            exit 65
            ;;
        esac
        candidate_path=${clone_root}/${relative}
        if [[ -e ${candidate_path} || -L ${candidate_path} ]]; then
          require_regular_file "${candidate_path}" 'existing apt candidate package'
          if [[ $(sha256_file "${candidate_path}") != $(sha256_file "${asset}") ]]; then
            printf 'existing apt candidate package conflicts: %s\n' "${name}" >&2
            exit 65
          fi
        else
          cp "${asset}" "${candidate_path}"
        fi
      done < <(find "${assets}" -mindepth 1 -maxdepth 1 -type f -name '*.deb' |
        LC_ALL=C sort)
      for architecture in amd64 arm64; do
        packages_path="${clone_root}/dists/${suite}/${component}/binary-${architecture}/Packages"
        (
          cd "${clone_root}"
          dpkg-scanpackages --arch "${architecture}" --multiversion pool /dev/null
        ) >"${packages_path}"
        gzip -n -9 -c "${packages_path}" >"${packages_path}.gz"
      done
      release_path=${clone_root}/dists/${suite}/Release
      source_date_epoch=$(jq -er '.source_date_epoch' \
        "${assets}/ait-release-apt.package.json")
      release_date=$(date -u -d "@${source_date_epoch}" '+%a, %d %b %Y %H:%M:%S +0000')
      {
        printf 'Origin: AIT Native\n'
        printf 'Label: AIT Native\n'
        printf 'Suite: %s\n' "${suite}"
        printf 'Codename: %s\n' "${suite}"
        printf 'Date: %s\n' "${release_date}"
        printf 'Architectures: amd64 arm64\n'
        printf 'Components: %s\n' "${component}"
        printf 'Description: Signed AIT Native release candidate repository\n'
        for algorithm in MD5Sum SHA1 SHA256; do
          printf '%s:\n' "${algorithm}"
          find "${clone_root}/dists/${suite}" -type f \
            ! -name Release ! -name InRelease ! -name Release.gpg -print |
            LC_ALL=C sort |
            while IFS= read -r member; do
              relative=${member#"${clone_root}/dists/${suite}/"}
              case "${algorithm}" in
                MD5Sum) digest=$(md5sum "${member}" | awk '{print $1}') ;;
                SHA1) digest=$(sha1sum "${member}" | awk '{print $1}') ;;
                SHA256) digest=$(sha256sum "${member}" | awk '{print $1}') ;;
              esac
              size=$(wc -c <"${member}" | tr -d '[:space:]')
              printf ' %s %16s %s\n' "${digest}" "${size}" "${relative}"
            done
        done
      } >"${release_path}"
      gnupg_root=${temporary_root}/apt-publish-gnupg
      mkdir -m 0700 "${gnupg_root}"
      printf '%s' "${AIT_APT_SIGNING_KEY_B64}" | base64 --decode >"${temporary_root}/apt-publish-secret.asc"
      printf '%s' "${AIT_APT_SIGNING_PASSPHRASE}" >"${temporary_root}/apt-publish-passphrase"
      chmod 0600 "${temporary_root}/apt-publish-secret.asc" \
        "${temporary_root}/apt-publish-passphrase"
      GNUPGHOME="${gnupg_root}" gpg --batch --import \
        "${temporary_root}/apt-publish-secret.asc" >/dev/null 2>&1
      GNUPGHOME="${gnupg_root}" gpg --batch --yes --pinentry-mode loopback \
        --passphrase-file "${temporary_root}/apt-publish-passphrase" \
        --local-user "${AIT_APT_SIGNING_FINGERPRINT}" --digest-algo SHA256 \
        --clearsign --output "${clone_root}/dists/${suite}/InRelease" "${release_path}"
      GNUPGHOME="${gnupg_root}" gpg --batch --yes --pinentry-mode loopback \
        --passphrase-file "${temporary_root}/apt-publish-passphrase" \
        --local-user "${AIT_APT_SIGNING_FINGERPRINT}" --digest-algo SHA256 \
        --armor --detach-sign --output "${clone_root}/dists/${suite}/Release.gpg" \
        "${release_path}"
      GNUPGHOME="${gnupg_root}" gpg --batch --export \
        "${AIT_APT_SIGNING_FINGERPRINT}" >"${clone_root}/ait-native-archive-keyring.gpg"
      GNUPGHOME="${gnupg_root}" gpg --batch --armor --export \
        "${AIT_APT_SIGNING_FINGERPRINT}" >"${clone_root}/ait-native-archive-keyring.asc"
      {
        printf '# AIT Native apt repository\n\n'
        # shellcheck disable=SC2016
        printf 'Exact RC route: `%s`; signing fingerprint:\n' "${suite}"
        # shellcheck disable=SC2016
        printf '`%s`.\n\n' "${AIT_APT_SIGNING_FINGERPRINT}"
        printf '```sh\n'
        printf 'curl -fsSL https://raw.githubusercontent.com/%s/%s/ait-native-archive-keyring.gpg \\\n' \
          "${repository}" "${branch}"
        printf '  | sudo tee /usr/share/keyrings/ait-native-archive-keyring.gpg >/dev/null\n'
        printf 'echo "deb [signed-by=/usr/share/keyrings/ait-native-archive-keyring.gpg] https://raw.githubusercontent.com/%s/%s %s %s" \\\n' \
          "${repository}" "${branch}" "${suite}" "${component}"
        printf '  | sudo tee /etc/apt/sources.list.d/ait-native.list\n'
        printf 'sudo apt update\n'
        printf 'apt-cache search --names-only "^ait-native$"\n'
        printf 'apt-cache search --names-only "^ait-runner$"\n'
        printf 'sudo apt install ait-native\n'
        printf '```\n'
      } >"${clone_root}/README.md"
      verify_apt_repository_clone "${clone_root}" "${suite}" "${component}" true
    fi
    if [[ -n $(git -C "${clone_root}" status --porcelain --untracked-files=all) ]]; then
      git -C "${clone_root}" add --all
      git -C "${clone_root}" \
        -c user.name='AIT Native Release' \
        -c user.email='253238140+weita2026@users.noreply.github.com' \
        commit -m "Publish signed ait-native ${release_version} apt repository" >/dev/null
      GIT_SSH_COMMAND="ssh -i ${key_path} -o IdentitiesOnly=yes -o UserKnownHostsFile=${known_hosts}" \
        git -C "${clone_root}" push origin "HEAD:refs/heads/${branch}"
    fi
    base_url=$(jq -er '.endpoints.apt.base_url' "${endpoint_config}")
    readback_root=${temporary_root}/apt-public-readback
    GIT_ASKPASS=/usr/bin/false GIT_TERMINAL_PROMPT=0 \
      git -c credential.helper= clone --quiet --depth 1 --branch "${branch}" \
        "https://github.com/${repository}.git" "${readback_root}"
    verify_apt_repository_clone "${readback_root}" "${suite}" "${component}" true
    jq -n \
      --arg contract 'ait.release.endpoint.apt/v1' \
      --arg status 'published_signed_and_read_back' \
      --arg release_id "${release_id}" \
      --arg suite "${suite}" \
      --arg base_url "${base_url}" \
      --arg fingerprint "${AIT_APT_SIGNING_FINGERPRINT}" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          suite: $suite,
          base_url: $base_url,
          signing_fingerprint: $fingerprint,
          signature_readback: true,
          package_digest_readback: true,
          apt_cache_search: true,
          component_rebuild: false
        }
      ' >"${evidence_root}/apt.json"
    ;;

  readback-oci)
    if ! command -v docker >/dev/null 2>&1; then
      printf 'required OCI inspector command is unavailable: docker\n' >&2
      exit 69
    fi
    require_preflight_receipt
    inspect_oci_remote_state true
    ;;

  readback)
    require_environment AIT_GITHUB_TOKEN AIT_OCI_SERVER_DIGEST AIT_OCI_RUNNER_DIGEST \
      AIT_APT_SIGNING_FINGERPRINT
    require_preflight_receipt
    validate_public_tag
    validate_github_release_state true
    validate_npm_remote_state true
    wait_for_pypi_remote_state
    for receipt_name in github pypi npm homebrew apt; do
      require_regular_file "${evidence_root}/${receipt_name}.json" \
        "${receipt_name} endpoint receipt"
    done
    require_regular_file "${evidence_root}/oci-state.json" 'OCI endpoint receipt'
    if ! jq -e \
      --arg server_digest "${AIT_OCI_SERVER_DIGEST}" \
      --arg runner_digest "${AIT_OCI_RUNNER_DIGEST}" '
        .contract == "ait.release.endpoint.oci-state/v1" and
        .require_published == true and
        .filesystem_readback == true and
        .images["ait-server"].present == true and
        .images["ait-server"].digest == $server_digest and
        .images["ait-runner"].present == true and
        .images["ait-runner"].digest == $runner_digest
      ' "${evidence_root}/oci-state.json" >/dev/null; then
      printf 'OCI endpoint receipt does not match the supplied digests\n' >&2
      exit 65
    fi
    homebrew_repository=$(jq -er '.endpoints.homebrew.repository' "${endpoint_config}")
    homebrew_branch=$(jq -er '.endpoints.homebrew.branch' "${endpoint_config}")
    homebrew_formula_path=$(jq -er '.endpoints.homebrew.formula_path' "${endpoint_config}")
    homebrew_readback=${temporary_root}/homebrew-final-readback
    GIT_ASKPASS=/usr/bin/false GIT_TERMINAL_PROMPT=0 \
      git -c credential.helper= clone --quiet --depth 1 --branch "${homebrew_branch}" \
        "https://github.com/${homebrew_repository}.git" "${homebrew_readback}"
    if ! cmp "${assets}/ait-native-rc.rb" \
      "${homebrew_readback}/${homebrew_formula_path}"; then
      printf 'final Homebrew formula readback failed\n' >&2
      exit 65
    fi
    apt_repository=$(jq -er '.endpoints.apt.repository' "${endpoint_config}")
    apt_branch=$(jq -er '.endpoints.apt.branch' "${endpoint_config}")
    apt_suite=$(jq -er '.endpoints.apt.suite' "${endpoint_config}")
    apt_component=$(jq -er '.endpoints.apt.component' "${endpoint_config}")
    apt_readback=${temporary_root}/apt-final-readback
    GIT_ASKPASS=/usr/bin/false GIT_TERMINAL_PROMPT=0 \
      git -c credential.helper= clone --quiet --depth 1 --branch "${apt_branch}" \
        "https://github.com/${apt_repository}.git" "${apt_readback}"
    verify_apt_repository_clone "${apt_readback}" "${apt_suite}" "${apt_component}" true
    if ! jq -e '
      .contract == "ait.release.endpoint.apt/v1" and
      .status == "published_signed_and_read_back" and
      .signature_readback == true and
      .package_digest_readback == true and
      .apt_cache_search == true
    ' "${evidence_root}/apt.json" >/dev/null; then
      printf 'APT endpoint receipt does not prove apt-cache searchability\n' >&2
      exit 65
    fi
    if [[ ! ${AIT_OCI_SERVER_DIGEST} =~ ^sha256:[0-9a-f]{64}$ ||
      ! ${AIT_OCI_RUNNER_DIGEST} =~ ^sha256:[0-9a-f]{64}$ ]]; then
      printf 'OCI image digest output is invalid\n' >&2
      exit 65
    fi
    jq -n \
      --arg contract 'ait.release.family.endpoint-readback/v1' \
      --arg status 'published_pending_clean_host_smoke' \
      --arg release_id "${release_id}" \
      --arg version "${release_version}" \
      --arg tag "${release_tag}" \
      --arg server_digest "${AIT_OCI_SERVER_DIGEST}" \
      --arg runner_digest "${AIT_OCI_RUNNER_DIGEST}" \
      --slurpfile config "${endpoint_config}" '
        {
          contract: $contract,
          status: $status,
          release_id: $release_id,
          version: $version,
          tag: $tag,
          endpoints: {
            github: "published_and_read_back",
            pypi: "published_and_read_back",
            npm: "published_and_read_back",
            homebrew: "published_and_read_back",
            apt: "published_signed_and_read_back",
            winget: "validation_assets_published_no_community_submission",
            oci: {
              server: $server_digest,
              runner: $runner_digest,
              immutable_tag: $config[0].endpoints.oci.immutable_tag,
              moving_tag: $config[0].endpoints.oci.moving_tag
            }
          },
          mutation: {
            artifact_rebuild: false,
            component_rebuild: false,
            registry_write: true,
            github_release_write: true,
            endpoint_repository_write: true,
            tag_write: false,
            ait_remote_release_activation: false,
            service_mutation: false
          },
          next_action: "run_all_declared_clean_host_install_upgrade_uninstall_smoke"
        }
      ' >"${evidence_root}/ait-release.endpoint-readback.json"
    ;;
esac
