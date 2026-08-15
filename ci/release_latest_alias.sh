#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
operator=${repo_root}/ci/release_operator.sh

usage() {
  printf '%s\n' \
    'usage: release_latest_alias.sh <apply|verify> <endpoint-config> <operator-status> <evidence-output>' >&2
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

require_regular_file() {
  local input=$1
  local label=$2
  [[ -f ${input} && ! -L ${input} ]] ||
    fail 66 "${label} must be a regular non-symlink file: ${input}"
}

canonical_file() {
  local input=$1
  (cd "$(dirname -- "${input}")" && printf '%s/%s\n' "$(pwd -P)" "$(basename -- "${input}")")
}

oci_digest() {
  local reference=$1
  docker buildx imagetools inspect "${reference}" \
    --format '{{json .Manifest.Digest}}' 2>/dev/null |
    jq -er 'select(type == "string" and test("^sha256:[0-9a-f]{64}$"))'
}

readonly readback_attempts=12
readonly readback_delay_seconds=5

wait_for_github_latest() {
  local expected=$1
  local attempt=1
  local actual
  while ((attempt <= readback_attempts)); do
    actual=$(gh api "repos/${repository}/releases/latest" --jq '.tag_name' 2>/dev/null || true)
    if [[ ${actual} == "${expected}" ]]; then
      printf '%s\n' "${actual}"
      return 0
    fi
    if ((attempt < readback_attempts)); then
      printf 'waiting for GitHub latest visibility (%s/%s)\n' \
        "${attempt}" "${readback_attempts}" >&2
      sleep "${readback_delay_seconds}"
    fi
    attempt=$((attempt + 1))
  done
  return 1
}

wait_for_npm_dist_tags() {
  local package_name=$1
  local expected_version=$2
  local channel=$3
  local attempt=1
  local tags
  while ((attempt <= readback_attempts)); do
    tags=$(npm view "${package_name}" dist-tags --json --registry "${npm_registry}" \
      2>/dev/null || true)
    if jq -e --arg version "${expected_version}" --arg channel "${channel}" '
      .latest == $version and (if $channel == "rc" then .rc == $version else true end)
    ' <<<"${tags}" >/dev/null 2>&1; then
      printf '%s\n' "${tags}"
      return 0
    fi
    if ((attempt < readback_attempts)); then
      printf 'waiting for npm dist-tag visibility: %s (%s/%s)\n' \
        "${package_name}" "${attempt}" "${readback_attempts}" >&2
      sleep "${readback_delay_seconds}"
    fi
    attempt=$((attempt + 1))
  done
  return 1
}

wait_for_oci_digest() {
  local reference=$1
  local expected=$2
  local attempt=1
  local actual
  while ((attempt <= readback_attempts)); do
    actual=$(oci_digest "${reference}" 2>/dev/null || true)
    if [[ ${actual} == "${expected}" ]]; then
      printf '%s\n' "${actual}"
      return 0
    fi
    if ((attempt < readback_attempts)); then
      printf 'waiting for OCI tag visibility: %s (%s/%s)\n' \
        "${reference}" "${attempt}" "${readback_attempts}" >&2
      sleep "${readback_delay_seconds}"
    fi
    attempt=$((attempt + 1))
  done
  return 1
}

[[ $# -eq 4 ]] || usage
mode=$1
endpoint_config=$2
operator_status=$3
evidence_output=$4
[[ ${mode} == apply || ${mode} == verify ]] || usage

for command in docker gh jq npm sleep; do
  command -v "${command}" >/dev/null 2>&1 ||
    fail 69 "required latest-alias command is unavailable: ${command}"
done
require_regular_file "${operator}" 'release operator'
require_regular_file "${endpoint_config}" 'endpoint configuration'
require_regular_file "${operator_status}" 'operator status'
[[ ${evidence_output} == /* ]] || fail 64 'latest-alias evidence output must be absolute'
[[ ! -e ${evidence_output} && ! -L ${evidence_output} ]] ||
  fail 73 "latest-alias evidence output already exists: ${evidence_output}"
[[ -d $(dirname -- "${evidence_output}") && ! -L $(dirname -- "${evidence_output}") ]] ||
  fail 66 'latest-alias evidence parent must be a real directory'

endpoint_config=$(canonical_file "${endpoint_config}")
operator_status=$(canonical_file "${operator_status}")
evidence_output=$(canonical_file "${evidence_output}")
bash "${operator}" validate-config --config "${endpoint_config}" >/dev/null

if ! jq -e --slurpfile config "${endpoint_config}" '
  ($config[0]) as $c |
  .contract == "ait.release.operator.status/v1" and
  .status == "published_pending_clean_host_smoke" and
  .release == {id: $c.release.id, tag: $c.release.tag, version: $c.release.version} and
  .platforms.github == "published_and_read_back" and
  .platforms.pypi == "published_and_read_back" and
  .platforms.npm == "published_and_read_back" and
  .platforms.homebrew == "published_and_read_back" and
  .platforms.apt == "published_signed_and_read_back" and
  .platforms.oci.immutable_tag == $c.endpoints.oci.immutable_tag and
  .platforms.oci.moving_tag == $c.endpoints.oci.moving_tag and
  (.platforms.oci.server | test("^sha256:[0-9a-f]{64}$")) and
  (.platforms.oci.runner | test("^sha256:[0-9a-f]{64}$"))
' "${operator_status}" >/dev/null; then
  fail 65 'operator status is incomplete or belongs to another release'
fi

IFS=$'\t' read -r release_id release_version release_channel release_tag \
  release_commit python_version repository npm_registry npm_route oci_immutable_tag \
  oci_moving_tag homebrew_formula apt_suite winget_route winget_submission < <(
  jq -er '[
    .release.id, .release.version, .release.channel, .release.tag,
    .release.source_commit, .release.python_version,
    .endpoints.github.repository, .endpoints.npm.registry,
    .endpoints.npm.dist_tag, .endpoints.oci.immutable_tag,
    .endpoints.oci.moving_tag, .endpoints.homebrew.formula_path,
    .endpoints.apt.suite, .endpoints.winget.route,
    (.endpoints.winget.community_manifest_submission | tostring)
  ] | @tsv' "${endpoint_config}"
)

if [[ ${release_channel} == rc ]]; then
  [[ ${release_version} =~ ^[0-9]+\.[0-9]+\.[0-9]+-rc\.[1-9][0-9]*$ &&
    ${npm_route} == rc && ${oci_moving_tag} == rc &&
    ${homebrew_formula} == Formula/ait-native-rc.rb && ${apt_suite} == testing &&
    ${winget_route} == validation && ${winget_submission} == false ]] ||
    fail 65 'RC latest promotion requires the unchanged RC endpoint routes'
else
  [[ ${release_channel} == stable &&
    ${release_version} =~ ^[0-9]+\.[0-9]+\.[0-9]+$ &&
    ${npm_route} == latest && ${oci_moving_tag} == latest ]] ||
    fail 65 'stable latest promotion requires the stable endpoint routes'
fi
[[ ${release_tag} == v${release_version} && ${oci_immutable_tag} == "${release_version}" ]] ||
  fail 65 'latest promotion release identity is inconsistent'

if [[ ${mode} == apply && ${AIT_RELEASE_LATEST_RELEASE_ID:-} != "${release_id}" ]]; then
  fail 77 'apply requires AIT_RELEASE_LATEST_RELEASE_ID to equal the exact approved Release ID'
fi

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-latest.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-latest.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected latest-alias path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

release_record=${temporary_root}/github-release.json
tag_ref=${temporary_root}/github-tag-ref.json
tag_record=${temporary_root}/github-tag.json
gh api "repos/${repository}/releases/tags/${release_tag}" >"${release_record}"
if ! jq -e --arg tag "${release_tag}" '
  .tag_name == $tag and .draft == false and .prerelease == false and
  (.id | type == "number" and . > 0)
' "${release_record}" >/dev/null; then
  fail 65 'GitHub Release is not an eligible non-draft latest release'
fi
github_release_id=$(jq -er '.id' "${release_record}")
gh api "repos/${repository}/git/ref/tags/${release_tag}" >"${tag_ref}"
if ! jq -e '.object.type == "tag" and (.object.sha | test("^[0-9a-f]{40}$"))' \
  "${tag_ref}" >/dev/null; then
  fail 65 'GitHub release tag is not an annotated tag object'
fi
github_tag_object=$(jq -er '.object.sha' "${tag_ref}")
gh api "repos/${repository}/git/tags/${github_tag_object}" >"${tag_record}"
if ! jq -e --arg commit "${release_commit}" \
  '.object.type == "commit" and .object.sha == $commit' "${tag_record}" >/dev/null; then
  fail 65 'GitHub annotated tag does not peel to the approved source commit'
fi

github_before=$(gh api "repos/${repository}/releases/latest" --jq '.tag_name' 2>/dev/null || true)
github_write=false
if [[ ${mode} == apply && ${github_before} != "${release_tag}" ]]; then
  gh api --method PATCH "repos/${repository}/releases/${github_release_id}" \
    -f make_latest=true >/dev/null
  github_write=true
fi
github_after=$(wait_for_github_latest "${release_tag}") ||
  fail 65 'GitHub latest Release did not converge to the approved tag'

npm_rows=${temporary_root}/npm-rows.jsonl
: >"${npm_rows}"
npm_write_count=0
while IFS= read -r package_name; do
  published_version=$(npm view "${package_name}@${release_version}" version --json \
    --registry "${npm_registry}" | jq -er 'select(type == "string")')
  [[ ${published_version} == "${release_version}" ]] ||
    fail 65 "npm exact version readback failed: ${package_name}@${release_version}"
  tags_before=$(npm view "${package_name}" dist-tags --json --registry "${npm_registry}")
  if [[ ${release_channel} == rc ]] && ! jq -e --arg version "${release_version}" \
    '.rc == $version' <<<"${tags_before}" >/dev/null; then
    fail 65 "npm rc tag does not retain the approved version: ${package_name}"
  fi
  before_latest=$(jq -r '.latest // ""' <<<"${tags_before}")
  package_write=false
  if [[ ${mode} == apply && ${before_latest} != "${release_version}" ]]; then
    npm dist-tag add "${package_name}@${release_version}" latest \
      --registry "${npm_registry}" >/dev/null
    package_write=true
    npm_write_count=$((npm_write_count + 1))
  fi
  wait_for_npm_dist_tags \
    "${package_name}" "${release_version}" "${release_channel}" >/dev/null ||
    fail 65 "npm latest/rc tag did not converge: ${package_name}"
  jq -cn --arg package "${package_name}" --arg before "${before_latest}" \
    --arg after "${release_version}" --argjson mutated "${package_write}" \
    '{package: $package, before: (if $before == "" then null else $before end), after: $after, rc_retained: true, mutated: $mutated}' \
    >>"${npm_rows}"
done < <(jq -er '.endpoints.npm.packages[]' "${endpoint_config}")
npm_packages=$(jq -s 'sort_by(.package)' "${npm_rows}")

oci_rows=${temporary_root}/oci-rows.jsonl
: >"${oci_rows}"
oci_write_count=0
while IFS= read -r image; do
  component=${image##*/}
  case "${component}" in
    ait-server) status_key=server ;;
    ait-runner) status_key=runner ;;
    *) fail 65 "OCI image is not a supported AIT release component: ${image}" ;;
  esac
  expected_digest=$(jq -er --arg component "${status_key}" \
    '.platforms.oci[$component] | select(test("^sha256:[0-9a-f]{64}$"))' \
    "${operator_status}")
  immutable_digest=$(oci_digest "${image}@${expected_digest}")
  [[ ${immutable_digest} == "${expected_digest}" ]] ||
    fail 65 "OCI immutable digest readback failed: ${image}"
  if [[ ${release_channel} == rc ]]; then
    rc_digest=$(oci_digest "${image}:rc")
    [[ ${rc_digest} == "${expected_digest}" ]] ||
      fail 65 "OCI rc tag does not retain the approved digest: ${image}"
  fi
  before_latest=$(oci_digest "${image}:latest" 2>/dev/null || true)
  image_write=false
  if [[ ${mode} == apply && ${before_latest} != "${expected_digest}" ]]; then
    docker buildx imagetools create --tag "${image}:latest" \
      "${image}@${expected_digest}" >/dev/null
    image_write=true
    oci_write_count=$((oci_write_count + 1))
  fi
  after_latest=$(wait_for_oci_digest "${image}:latest" "${expected_digest}") ||
    fail 65 "OCI latest tag did not converge: ${image}"
  jq -cn --arg image "${image}" --arg before "${before_latest}" \
    --arg after "${after_latest}" --argjson mutated "${image_write}" \
    '{image: $image, before: (if $before == "" then null else $before end), after: $after, rc_retained: true, mutated: $mutated}' \
    >>"${oci_rows}"
done < <(jq -er '.endpoints.oci.images[]' "${endpoint_config}")
oci_images=$(jq -s 'sort_by(.image)' "${oci_rows}")

config_sha256=$(sha256_file "${endpoint_config}")
status_sha256=$(sha256_file "${operator_status}")
jq -S -n \
  --arg mode "${mode}" \
  --arg release_id "${release_id}" \
  --arg version "${release_version}" \
  --arg python_version "${python_version}" \
  --arg channel "${release_channel}" \
  --arg tag "${release_tag}" \
  --arg source_commit "${release_commit}" \
  --arg config_sha256 "${config_sha256}" \
  --arg status_sha256 "${status_sha256}" \
  --arg github_before "${github_before}" \
  --arg github_after "${github_after}" \
  --arg homebrew_formula "${homebrew_formula}" \
  --arg apt_suite "${apt_suite}" \
  --arg winget_route "${winget_route}" \
  --argjson winget_submission "${winget_submission}" \
  --argjson github_write "${github_write}" \
  --argjson npm_write_count "${npm_write_count}" \
  --argjson oci_write_count "${oci_write_count}" \
  --argjson npm_packages "${npm_packages}" \
  --argjson oci_images "${oci_images}" '
  {
    contract: "ait.release.latest-alias/v1",
    status: (if $mode == "apply" then "promoted_and_read_back" else "verified" end),
    release: {
      id: $release_id,
      version: $version,
      python_version: $python_version,
      channel: $channel,
      tag: $tag,
      source_commit: $source_commit
    },
    source_evidence: {
      endpoint_config_sha256: $config_sha256,
      operator_status_sha256: $status_sha256
    },
    aliases: {
      github: {alias: "latest", before: (if $github_before == "" then null else $github_before end), after: $github_after},
      npm: {alias: "latest", packages: $npm_packages, rc_alias_retained: true},
      oci: {alias: "latest", images: $oci_images, rc_alias_retained: true}
    },
    native_prerelease_routes: {
      pypi: {
        mutable_latest_alias_supported: false,
        exact_selector: ("ait-native==" + $python_version),
        default_pip_prerelease_resolution: false
      },
      homebrew: {
        mutable_latest_alias_supported: false,
        latest_rc_formula: $homebrew_formula,
        stable_formula_unchanged: true
      },
      apt: {
        mutable_latest_alias_supported: false,
        latest_rc_suite: $apt_suite,
        stable_suite_unchanged: true
      },
      winget: {
        mutable_latest_alias_supported: false,
        route: $winget_route,
        community_manifest_submission: $winget_submission
      }
    },
    mutation: {
      github_release_write: $github_write,
      npm_dist_tag_write_count: $npm_write_count,
      oci_tag_write_count: $oci_write_count,
      artifact_rebuild: false,
      component_rebuild: false,
      immutable_version_write: false,
      tag_write: false,
      pypi_write: false,
      homebrew_write: false,
      apt_write: false,
      winget_write: false,
      ait_remote_release_activation: false
    }
  }
' >"${temporary_root}/evidence.json"
mv "${temporary_root}/evidence.json" "${evidence_output}"
printf '%s\n' "${evidence_output}"
