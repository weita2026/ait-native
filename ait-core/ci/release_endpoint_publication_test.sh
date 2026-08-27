#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
workflow=${repo_root}/.github/workflows/pypi-publish.yml
prepublish_workflow=${repo_root}/.github/workflows/ait-release-prepublish-clean-host.yml
endpoint_config_source=${repo_root}/release/endpoint-publication.rc4.json
endpoint_defaults=${repo_root}/release/endpoint-publication.defaults.json
operator=${repo_root}/ci/release_operator.sh
preparer=${repo_root}/ci/release_endpoint_publication.sh
remote=${repo_root}/ci/release_endpoint_remote.sh
prepublish_stage=${repo_root}/ci/release_prepublish_stage.sh
prepublish_oci=${repo_root}/ci/release_prepublish_oci.sh
prepublish_verify=${repo_root}/ci/release_prepublish_verify.mjs
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-endpoint-publication-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-endpoint-publication-test.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected endpoint test path: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

endpoint_config=${temporary_root}/endpoint-config.json
jq '.release.channel = "rc" | .endpoints.github.prerelease = true' \
  "${endpoint_config_source}" >"${endpoint_config}"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected endpoint-publication failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

expect_rejection() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected endpoint contract rejection: %s\n' "${label}" >&2
    return 1
  fi
}

for path in "${workflow}" "${prepublish_workflow}" "${endpoint_config}" \
  "${endpoint_defaults}" "${operator}" "${preparer}" "${remote}" \
  "${prepublish_stage}" "${prepublish_oci}" "${prepublish_verify}" \
  "${repo_root}/release/oci/ait-server.Dockerfile" \
  "${repo_root}/release/oci/ait-runner.Dockerfile"; do
  if [[ ! -f ${path} || -L ${path} ]]; then
    printf 'endpoint-publication input is unavailable: %s\n' "${path}" >&2
    exit 66
  fi
done

bash -n "${preparer}"
bash -n "${remote}"
bash -n "${prepublish_stage}"
bash -n "${prepublish_oci}"
node --check "${prepublish_verify}"
bash "${operator}" validate-config --config "${endpoint_config}" >/dev/null
# shellcheck disable=SC2016 # These are literal source-contract fragments.
for required_apt_contract in \
  'local require_candidate_assets=${4:-true}' \
  'verify_apt_repository_clone "${clone_root}" "${suite}" "${component}" false' \
  'dpkg-scanpackages --arch "${architecture}" --multiversion pool /dev/null' \
  'for name in ait-native ait-runner; do' \
  'search --names-only "^${name}$"' \
  'apt_cache_search: true'; do
  if ! grep -F -- "${required_apt_contract}" "${remote}" >/dev/null; then
    printf 'APT publisher is missing searchable multiversion behavior: %s\n' \
      "${required_apt_contract}" >&2
    exit 65
  fi
done
if grep -F 'apt repository package inventory is not exact' "${remote}" >/dev/null; then
  printf 'APT publisher still rejects a repository that retains older versions\n' >&2
  exit 65
fi
jq -e '
  .contract == "ait.release.family.endpoints/v1" and
  .release == {
    id: "REL-FAM-701E789EBD8B6848",
    version: "1.0.0-rc.4",
    channel: "rc",
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
  .endpoints.github == {
    repository: "weita2026/ait-native",
    prerelease: true
  } and
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
  .endpoints.winget == {
    identity: "Weita.AitNative",
    route: "validation",
    community_manifest_submission: false
  } and
  .endpoints.oci.dockerfile_frontend ==
    "docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e" and
  .endpoints.oci.images == [
    "ghcr.io/weita2026/ait-server",
    "ghcr.io/weita2026/ait-runner"
  ]
' "${endpoint_config}" >/dev/null

for required in \
  'name: ait release endpoint publication' \
  'actions: read' \
  'attestations: write' \
  'contents: write' \
  'id-token: write' \
  'packages: write' \
  '      name: pypi' \
  'protected_run_id:' \
  'endpoint_config_sha256:' \
  'endpoint_config_b64:' \
  'reuse_frozen_candidate:' \
  'candidate_run_id:' \
  'candidate_artifact_id:' \
  'candidate_artifact_digest:' \
  'candidate_status_sha256:' \
  'uses: ./.github/workflows/ait-release-prepublish-clean-host.yml' \
  'needs: prepublish' \
  'control/ci/release_operator.sh validate-config' \
  'control/ci/release_prepublish_verify.mjs qualify' \
  'control/ci/release_prepublish_oci.sh publish' \
  'control/ci/release_endpoint_remote.sh preflight' \
  'secrets.AIT_NPM_TOKEN' \
  'docker logout ghcr.io' \
  'name: ait-endpoint-publication-${{ inputs.release_id }}'; do
  grep -F -- "${required}" "${workflow}" >/dev/null
done
for required_prepublish_contract in \
  'name: ait release prepublish clean host' \
  'permissions:' \
  'actions: read' \
  'contents: read' \
  'control/ci/release_prepublish_stage.sh' \
  'control/ci/release_prepublish_verify.mjs stage' \
  'control/ci/release_clean_host.mjs aggregate' \
  'Download the previously frozen candidate for control-only retry' \
  'cmp "${comparison_root}/ait-release.clean-host-matrix.json" "${matrix}"' \
  'run-id: ${{ needs.stage.outputs.candidate_run_id }}' \
  'ait-prepublish-candidate-${{ inputs.release_id }}' \
  'ait-prepublish-clean-host-${{ inputs.release_id }}'; do
  grep -F -- "${required_prepublish_contract}" "${prepublish_workflow}" >/dev/null
done
grep -F 'release_endpoint_publication.sh' "${prepublish_stage}" >/dev/null
if grep -E '(^|[[:space:]])(gh release|npm publish|twine upload|docker push)([[:space:]]|$)' \
  "${prepublish_workflow}" "${prepublish_stage}" >/dev/null; then
  printf 'prepublish gate contains a public endpoint write\n' >&2
  exit 65
fi
for required_preparer_contract in \
  'immutable-tag-native-admission/v1' \
  'frozen npm package platform or repository metadata is not exact' \
  'ait.node.napi-platform-packages/v2' \
  'ait.node.napi-platform-addon/v2' \
  'git+https://github.com/weita2026/ait-native.git' \
  'frozen npm package identity inventory is not exact'; do
  grep -F -- "${required_preparer_contract}" "${preparer}" >/dev/null
done
for required_npm_readback_contract in \
  'validate_npm_registry_platform_readback() {' \
  'wait_for_npm_remote_state() {' \
  'npm package set did not become fully visible after %s attempts' \
  'npm registry platform metadata differs from staged bytes' \
  'platform_metadata_readback: true' \
  'npm endpoint receipt does not prove platform metadata readback'; do
  if ! grep -F -- "${required_npm_readback_contract}" "${remote}" >/dev/null; then
    printf 'npm publisher is missing exact platform readback: %s\n' \
      "${required_npm_readback_contract}" >&2
    exit 65
  fi
done
for retired_workflow_constant in \
  31716406486 9188270344 31721469565 9189654794 \
  e8925452d487665ea6fd67503278f48baa3eca68 \
  99191fe336d76e5ebba06333ac8a9338f4381763 \
  4a5c025cdcb8626fe93c71cb4f2780eda1057f979eaa114e4e3e0e3a5dd17e09 \
  endpoint-publication.rc4.json; do
  if grep -F -- "${retired_workflow_constant}" "${workflow}" >/dev/null; then
    printf 'endpoint workflow retains a release-specific constant: %s\n' \
      "${retired_workflow_constant}" >&2
    exit 65
  fi
done
grep -F -- '--prerelease="${github_prerelease}"' "${remote}" >/dev/null
grep -F -- '--latest=false' "${remote}" >/dev/null
grep -F -- '--argjson prerelease "${github_prerelease}"' "${remote}" >/dev/null
grep -F '.prerelease == $prerelease' "${remote}" >/dev/null
if grep -F 'continue-on-error:' "${workflow}" >/dev/null; then
  printf 'endpoint publication must not hide a failed endpoint\n' >&2
  exit 65
fi
for required_pypi_readback_contract in \
  'wait_for_pypi_remote_state() {' \
  'local max_attempts=12' \
  'if [[ ${readback_status} != 75 ]]; then' \
  'wait_for_pypi_remote_state'; do
  if ! grep -F -- "${required_pypi_readback_contract}" "${remote}" >/dev/null; then
    printf 'PyPI publisher is missing bounded readback behavior: %s\n' \
      "${required_pypi_readback_contract}" >&2
    exit 65
  fi
done
if ! awk '
  /- name: Publish the signed apt repository/ { apt_step = NR }
  /- name: Publish npm implementation payloads and command envelope/ { npm_step = NR }
  END { exit !(apt_step > 0 && npm_step > apt_step) }
' "${workflow}"; then
  printf 'npm must remain visible and run after independent endpoint writes\n' >&2
  exit 65
fi
grep -F '      - name: Complete independent endpoint readback' \
  "${workflow}" >/dev/null
grep -F 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU' \
  "${remote}" >/dev/null
if grep -F 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPtwkmmLoMI' \
  "${remote}" >/dev/null; then
  printf 'endpoint publisher retains the retired GitHub Ed25519 host fingerprint\n' >&2
  exit 65
fi
if awk '
  /^[[:space:]]*uses:/ &&
  $0 !~ /^[[:space:]]*uses:[[:space:]]+\.\// &&
  $0 !~ /@[0-9a-f]{40}([[:space:]]|$)/ { print; bad = 1 }
  END { exit bad }
' \
  "${workflow}"; then
  :
else
  printf 'endpoint publisher contains an unpinned action\n' >&2
  exit 65
fi
for forbidden in \
  'cargo build' \
  'cargo install' \
  'maturin build' \
  'npm run build' \
  'wingetcreate submit' \
  'winget-pkgs' \
  'AIT_RELEASE_SERVER_URL'; do
  if grep -F -- "${forbidden}" "${workflow}" >/dev/null; then
    printf 'endpoint publisher contains forbidden behavior: %s\n' "${forbidden}" >&2
    exit 65
  fi
done

extract_remote_function() {
  local function_name=$1
  awk -v signature="${function_name}() {" '
    $0 == signature { capture = 1 }
    capture { print }
    capture && $0 == "}" { exit }
  ' "${remote}"
}
extract_preparer_function() {
  local function_name=$1
  awk -v signature="${function_name}() {" '
    $0 == signature { capture = 1 }
    capture { print }
    capture && $0 == "}" { exit }
  ' "${preparer}"
}
eval "$(extract_remote_function github_release_asset_name)"
eval "$(extract_remote_function github_release_asset_map)"
eval "$(extract_remote_function github_release_local_path)"
eval "$(extract_remote_function npm_registry_package_path)"
eval "$(extract_remote_function npm_provenance_policy)"
eval "$(extract_remote_function remove_matching_npm_prerelease_latest_tag)"
eval "$(extract_remote_function validate_npm_dist_tags)"
eval "$(extract_remote_function validate_npm_registry_platform_readback)"
eval "$(extract_remote_function validate_npm_remote_state)"
eval "$(extract_remote_function wait_for_npm_remote_state)"
eval "$(extract_remote_function validate_pypi_remote_state)"
eval "$(extract_remote_function wait_for_pypi_remote_state)"
eval "$(extract_preparer_function npm_addon_platform)"
eval "$(extract_preparer_function validate_npm_payload_contract)"
eval "$(extract_preparer_function validate_npm_package_archive)"

test "$(npm_registry_package_path '@wa120/ait-native')" = \
  '@wa120%2Fait-native'
test "$(npm_registry_package_path 'ait-native')" = 'ait-native'
grep -F 'if [[ ${package_name} == @wa120/ait-native ]]' "${remote}" >/dev/null
grep -F 'if [[ ${package_name} != @wa120/ait-native ]]' "${remote}" >/dev/null

export release_version=1.0.0-rc.4
export npm_registry=https://registry.npmjs.org
npm_package_rows() {
  jq -r '.endpoints.npm.packages[]' "${endpoint_config}" |
    while IFS= read -r package_name; do
      printf '%s\t%s\t%s\n' \
        "${package_name}" "${release_version}" "${temporary_root}/unused.tgz"
    done
}
npm_visibility_mode=package-missing
curl() {
  local output=
  while (($# > 0)); do
    case "$1" in
      --output) output=$2; shift 2 ;;
      *) shift ;;
    esac
  done
  [[ -n ${output} ]]
  case "${npm_visibility_mode}" in
    package-missing)
      printf '{}\n' >"${output}"
      printf '404'
      ;;
    version-missing)
      printf '{"versions": {}}\n' >"${output}"
      printf '200'
      ;;
    *) return 64 ;;
  esac
}
validate_npm_remote_state
if validate_npm_remote_state true \
  >"${temporary_root}/npm-package-missing.stdout" \
  2>"${temporary_root}/npm-package-missing.stderr"; then
  printf 'expected retryable missing npm package state\n' >&2
  exit 1
else
  test "$?" = 75
fi
grep -F 'npm package is still unpublished:' \
  "${temporary_root}/npm-package-missing.stderr" >/dev/null
npm_visibility_mode=version-missing
validate_npm_remote_state
if validate_npm_remote_state true \
  >"${temporary_root}/npm-version-missing.stdout" \
  2>"${temporary_root}/npm-version-missing.stderr"; then
  printf 'expected retryable missing npm version state\n' >&2
  exit 1
else
  test "$?" = 75
fi
grep -F 'npm package version is still unpublished:' \
  "${temporary_root}/npm-version-missing.stderr" >/dev/null
unset -f curl npm_package_rows

pypi_assets=${temporary_root}/pypi-restart-assets
mkdir "${pypi_assets}"
printf 'wheel-one\n' >"${pypi_assets}/ait_native-1.0.0rc4-one.whl"
printf 'wheel-two\n' >"${pypi_assets}/ait_native-1.0.0rc4-two.whl"
assets=${pypi_assets}
python_version=1.0.0rc4
pypi_response=${temporary_root}/pypi-response.json
jq -n \
  --arg filename 'ait_native-1.0.0rc4-one.whl' \
  --arg digest "$(sha256_file "${pypi_assets}/ait_native-1.0.0rc4-one.whl")" '
  {
    info: {name: "ait-native"},
    releases: {
      "0.10.6": [{}, {}],
      "1.0.0rc4": [{filename: $filename, digests: {sha256: $digest}}]
    }
  }
' >"${pypi_response}"
curl() {
  local output=
  while (($# > 0)); do
    case "$1" in
      --output) output=$2; shift 2 ;;
      *) shift ;;
    esac
  done
  cp "${pypi_response}" "${output}"
  printf '200'
}
validate_pypi_remote_state
if validate_pypi_remote_state true \
  >"${temporary_root}/pypi-partial.stdout" \
  2>"${temporary_root}/pypi-partial.stderr"; then
  printf 'expected retryable exact partial PyPI state\n' >&2
  exit 1
else
  test "$?" = 75
fi
grep -F 'PyPI release wheel set is only partially visible' \
  "${temporary_root}/pypi-partial.stderr" >/dev/null
jq '.releases["1.0.0rc4"][0].digests.sha256 = ("f" * 64)' \
  "${pypi_response}" >"${pypi_response}.new"
mv "${pypi_response}.new" "${pypi_response}"
expect_failure pypi-partial-digest-drift validate_pypi_remote_state
grep -F 'PyPI already contains conflicting bytes:' \
  "${temporary_root}/pypi-partial-digest-drift.stderr" >/dev/null
unset -f curl

pypi_readback_attempts=0
validate_pypi_remote_state() {
  pypi_readback_attempts=$((pypi_readback_attempts + 1))
  if ((pypi_readback_attempts < 3)); then
    return 75
  fi
  return 0
}
sleep() {
  :
}
wait_for_pypi_remote_state \
  >"${temporary_root}/pypi-eventual-success.stdout" \
  2>"${temporary_root}/pypi-eventual-success.stderr"
test "${pypi_readback_attempts}" = 3

pypi_readback_attempts=0
validate_pypi_remote_state() {
  pypi_readback_attempts=$((pypi_readback_attempts + 1))
  return 65
}
if wait_for_pypi_remote_state \
  >"${temporary_root}/pypi-hard-failure.stdout" \
  2>"${temporary_root}/pypi-hard-failure.stderr"; then
  printf 'expected hard PyPI readback failure\n' >&2
  exit 1
fi
test "${pypi_readback_attempts}" = 1

pypi_readback_attempts=0
validate_pypi_remote_state() {
  pypi_readback_attempts=$((pypi_readback_attempts + 1))
  return 75
}
if wait_for_pypi_remote_state \
  >"${temporary_root}/pypi-eventual-timeout.stdout" \
  2>"${temporary_root}/pypi-eventual-timeout.stderr"; then
  printf 'expected bounded PyPI readback timeout\n' >&2
  exit 1
fi
test "${pypi_readback_attempts}" = 12
grep -F 'PyPI release wheel set did not become fully visible after 12 attempts' \
  "${temporary_root}/pypi-eventual-timeout.stderr" >/dev/null

npm_readback_attempts=0
validate_npm_remote_state() {
  npm_readback_attempts=$((npm_readback_attempts + 1))
  if ((npm_readback_attempts < 3)); then
    return 75
  fi
  return 0
}
wait_for_npm_remote_state \
  >"${temporary_root}/npm-eventual-success.stdout" \
  2>"${temporary_root}/npm-eventual-success.stderr"
test "${npm_readback_attempts}" = 3

npm_readback_attempts=0
validate_npm_remote_state() {
  npm_readback_attempts=$((npm_readback_attempts + 1))
  return 65
}
if wait_for_npm_remote_state \
  >"${temporary_root}/npm-hard-failure.stdout" \
  2>"${temporary_root}/npm-hard-failure.stderr"; then
  printf 'expected hard npm readback failure\n' >&2
  exit 1
fi
test "${npm_readback_attempts}" = 1

npm_readback_attempts=0
validate_npm_remote_state() {
  npm_readback_attempts=$((npm_readback_attempts + 1))
  return 75
}
if wait_for_npm_remote_state \
  >"${temporary_root}/npm-eventual-timeout.stdout" \
  2>"${temporary_root}/npm-eventual-timeout.stderr"; then
  printf 'expected bounded npm readback timeout\n' >&2
  exit 1
fi
test "${npm_readback_attempts}" = 12
grep -F 'npm package set did not become fully visible after 12 attempts' \
  "${temporary_root}/npm-eventual-timeout.stderr" >/dev/null
unset -f validate_npm_remote_state validate_pypi_remote_state sleep \
  wait_for_npm_remote_state wait_for_pypi_remote_state

github_asset_fixture=${temporary_root}/github-asset-fixture
mkdir -p "${github_asset_fixture}/assets"
assets=${github_asset_fixture}/assets
stage_receipt=${github_asset_fixture}/ait-release.endpoint-publication.json
: >"${assets}/ait-native_1.0.0~rc.4_amd64.deb"
: >"${assets}/plain.zip"
: >"${stage_receipt}"
github_asset_map=${github_asset_fixture}/asset-map
github_release_asset_map "${github_asset_map}"
test "$(wc -l <"${github_asset_map}" | tr -d '[:space:]')" = 3
grep -F $'ait-native_1.0.0.rc.4_amd64.deb\t'"${assets}/ait-native_1.0.0~rc.4_amd64.deb" \
  "${github_asset_map}" >/dev/null
test "$(github_release_local_path \
  "${github_asset_map}" 'ait-native_1.0.0.rc.4_amd64.deb')" = \
  "${assets}/ait-native_1.0.0~rc.4_amd64.deb"
test "$(github_release_asset_name 'plain~name.zip')" = 'plain~name.zip'
: >"${assets}/ait-native_1.0.0.rc.4_amd64.deb"
expect_failure github-asset-name-collision \
  github_release_asset_map "${github_asset_fixture}/colliding-map"

npm_tag_mock=${temporary_root}/npm-tag-mock
mkdir "${npm_tag_mock}"
npm_tag_log=${temporary_root}/npm-tag.log
npm_tag_log_q=$(printf '%q' "${npm_tag_log}")
# shellcheck disable=SC2016 # These are literal lines for the mock executable.
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'if [[ $1 == dist-tag && $2 == ls ]]; then' \
  '  case "$3" in' \
  '    matching) printf '\''latest: 1.0.0-rc.4\nrc: 1.0.0-rc.4\n'\''' \
  '      ;;' \
  '    stable) printf '\''latest: 0.9.0\nrc: 1.0.0-rc.4\n'\''' \
  '      ;;' \
  '    no-latest) printf '\''rc: 1.0.0-rc.4\n'\''' \
  '      ;;' \
  '    *) exit 64 ;;' \
  '  esac' \
  '  exit 0' \
  'fi' \
  'if [[ $1 == dist-tag && $2 == rm ]]; then' \
  "  printf '%s\\n' \"\$*\" >>${npm_tag_log_q}" \
  '  exit 0' \
  'fi' \
  'exit 64' \
  >"${npm_tag_mock}/npm"
chmod 0700 "${npm_tag_mock}/npm"
: >"${npm_tag_log}"
export npm_registry=https://registry.npmjs.org
npmrc=${temporary_root}/npm-tag-test.npmrc
: >"${npmrc}"
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0-rc.4 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" stable 1.0.0-rc.4 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" no-latest 1.0.0-rc.4 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0 latest
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0-rc.4 latest
test "$(wc -l <"${npm_tag_log}" | tr -d '[:space:]')" = 1
grep -Fx 'dist-tag rm matching latest --registry https://registry.npmjs.org' \
  "${npm_tag_log}" >/dev/null

jq -n '{"dist-tags": {rc: "1.0.0-rc.4", latest: "0.9.0"}}' \
  >"${temporary_root}/npm-tags-valid.json"
validate_npm_dist_tags "${temporary_root}/npm-tags-valid.json" \
  @wa120/ait-native 1.0.0-rc.4 rc
jq -n '{"dist-tags": {rc: "1.0.0-rc.4", latest: "1.0.0-rc.4"}}' \
  >"${temporary_root}/npm-tags-rc-latest.json"
expect_failure npm-rc-remains-latest validate_npm_dist_tags \
  "${temporary_root}/npm-tags-rc-latest.json" @wa120/ait-native 1.0.0-rc.4 rc
grep -F 'npm prerelease remains the default latest tag: @wa120/ait-native@1.0.0-rc.4' \
  "${temporary_root}/npm-rc-remains-latest.stderr" >/dev/null
jq -n '{"dist-tags": {rc: "1.0.0-rc.0", latest: "0.9.0"}}' \
  >"${temporary_root}/npm-tags-wrong-rc.json"
expect_failure npm-rc-tag-drift validate_npm_dist_tags \
  "${temporary_root}/npm-tags-wrong-rc.json" @wa120/ait-native 1.0.0-rc.4 rc
grep -F 'npm configured dist-tag readback failed: @wa120/ait-native@1.0.0-rc.4' \
  "${temporary_root}/npm-rc-tag-drift.stderr" >/dev/null

export github_repository=weita2026/ait-native
export release_version=1.0.0-rc.4
export source_commit=ea2d347010d3ead2cdfb304e6df448cbf9fe0c4e

npm_contract_fixture=${temporary_root}/npm-contract-v2.json
jq -n --arg version "${release_version}" '
  [
    ["aarch64-apple-darwin", "darwin", "arm64", null],
    ["x86_64-apple-darwin", "darwin", "x64", null],
    ["aarch64-unknown-linux-gnu", "linux", "arm64", "glibc"],
    ["x86_64-unknown-linux-gnu", "linux", "x64", "glibc"],
    ["aarch64-pc-windows-msvc", "win32", "arm64", null],
    ["x86_64-pc-windows-msvc", "win32", "x64", null]
  ] | {
    schema: "ait.node.napi-platform-packages/v2",
    family_version: $version,
    top_level_package: "@wa120/ait-native",
    payloads: map({
      target: .[0],
      os: .[1],
      cpu: .[2],
      libc: .[3],
      component: "ait-node",
      package: ("@wa120/ait-native-" + .[1] + "-" + .[2]),
      version: $version,
      binding_repository: "ait-core",
      binding_snapshot: "SNP-AAAAAAAAAAAA",
      license: "Apache-2.0",
      addon: "native/ait_napi.node"
    })
  }
' >"${npm_contract_fixture}"
validate_npm_payload_contract "${npm_contract_fixture}"
for mutation in linux-null linux-musl darwin-glibc linux-missing; do
  case "${mutation}" in
    linux-null)
      jq '.payloads[2].libc = null' "${npm_contract_fixture}"
      ;;
    linux-musl)
      jq '.payloads[2].libc = "musl"' "${npm_contract_fixture}"
      ;;
    darwin-glibc)
      jq '.payloads[0].libc = "glibc"' "${npm_contract_fixture}"
      ;;
    linux-missing)
      jq 'del(.payloads[2].libc)' "${npm_contract_fixture}"
      ;;
  esac >"${temporary_root}/npm-contract-${mutation}.json"
  expect_rejection "npm-contract-${mutation}" validate_npm_payload_contract \
    "${temporary_root}/npm-contract-${mutation}.json"
done

npm_envelope=${temporary_root}/npm-envelope
mkdir -p "${npm_envelope}/package/lib"
cp "${npm_contract_fixture}" \
  "${npm_envelope}/package/lib/npm-payload-contract.json"
jq -n --arg version "${release_version}" '
  {
    name: "@wa120/ait-native",
    version: $version,
    repository: {
      type: "git",
      url: "git+https://github.com/weita2026/ait-native.git",
      directory: "ait-node"
    },
    optionalDependencies: {
      "@wa120/ait-native-darwin-arm64": $version,
      "@wa120/ait-native-darwin-x64": $version,
      "@wa120/ait-native-linux-arm64": $version,
      "@wa120/ait-native-linux-x64": $version,
      "@wa120/ait-native-win32-arm64": $version,
      "@wa120/ait-native-win32-x64": $version
    }
  }
' >"${npm_envelope}/package/package.json"
tar -czf "${npm_envelope}.tgz" -C "${npm_envelope}" package
validate_npm_package_archive "${npm_envelope}.tgz" \
  "${npm_envelope}/package/package.json"
jq '.libc = ["glibc"]' "${npm_envelope}/package/package.json" \
  >"${temporary_root}/npm-envelope-platform-drift.json"
expect_rejection npm-envelope-platform-drift validate_npm_package_archive \
  "${npm_envelope}.tgz" "${temporary_root}/npm-envelope-platform-drift.json"

write_npm_addon_fixture() {
  local root=$1
  local package_name=$2
  local target=$3
  local os=$4
  local cpu=$5
  local libc_mode=$6
  local selected_libc=${libc_mode}
  local has_manifest_libc=true
  if [[ ${libc_mode} == none ]]; then
    selected_libc=none
    has_manifest_libc=false
  elif [[ ${libc_mode} == missing ]]; then
    selected_libc=glibc
    has_manifest_libc=false
  fi
  mkdir -p "${root}/package/native"
  jq -n \
    --arg package "${package_name}" \
    --arg version "${release_version}" \
    --arg target "${target}" \
    --arg os "${os}" \
    --arg cpu "${cpu}" \
    --arg libc "${selected_libc}" \
    --argjson has_manifest_libc "${has_manifest_libc}" '
      ({
        name: $package,
        version: $version,
        repository: {
          type: "git",
          url: "git+https://github.com/weita2026/ait-native.git",
          directory: "ait-node"
        },
        os: [$os],
        cpu: [$cpu],
        aitNativeAddon: {
          schema: "ait.node.napi-platform-addon/v2",
          component: "ait-node",
          target: $target,
          libc: (if $libc == "none" then null else $libc end),
          addon: "native/ait_napi.node",
          binding_repository: "ait-core",
          binding_snapshot: "SNP-AAAAAAAAAAAA"
        }
      } + if $has_manifest_libc then {libc: [$libc]} else {} end)
    ' >"${root}/package/package.json"
  jq -n \
    --arg package "${package_name}" \
    --arg version "${release_version}" \
    --arg target "${target}" \
    --arg os "${os}" \
    --arg cpu "${cpu}" \
    --arg libc "${selected_libc}" '
      {
        schema: "ait.node.napi-platform-addon-provenance/v2",
        family_version: $version,
        package: $package,
        target: $target,
        os: $os,
        cpu: $cpu,
        libc: (if $libc == "none" then null else $libc end),
        component: "ait-node",
        package_source_repository: "ait-node",
        binding_repository: "ait-core",
        binding_snapshot: "SNP-AAAAAAAAAAAA",
        installed_path: "native/ait_napi.node"
      }
    ' >"${root}/package/provenance.json"
  printf 'fixture addon\n' >"${root}/package/native/ait_napi.node"
  tar -czf "${root}.tgz" -C "${root}" package
}

linux_addon=${temporary_root}/npm-linux-glibc
write_npm_addon_fixture "${linux_addon}" \
  @wa120/ait-native-linux-x64 x86_64-unknown-linux-gnu linux x64 glibc
validate_npm_package_archive "${linux_addon}.tgz" \
  "${linux_addon}/package/package.json"

darwin_addon=${temporary_root}/npm-darwin
write_npm_addon_fixture "${darwin_addon}" \
  @wa120/ait-native-darwin-arm64 aarch64-apple-darwin darwin arm64 none
validate_npm_package_archive "${darwin_addon}.tgz" \
  "${darwin_addon}/package/package.json"

for mutation in linux-missing linux-musl darwin-glibc; do
  invalid_addon=${temporary_root}/npm-${mutation}
  case "${mutation}" in
    linux-missing)
      write_npm_addon_fixture "${invalid_addon}" \
        @wa120/ait-native-linux-x64 x86_64-unknown-linux-gnu linux x64 missing
      ;;
    linux-musl)
      write_npm_addon_fixture "${invalid_addon}" \
        @wa120/ait-native-linux-x64 x86_64-unknown-linux-gnu linux x64 musl
      ;;
    darwin-glibc)
      write_npm_addon_fixture "${invalid_addon}" \
        @wa120/ait-native-darwin-arm64 aarch64-apple-darwin darwin arm64 glibc
      ;;
  esac
  expect_rejection "npm-addon-${mutation}" validate_npm_package_archive \
    "${invalid_addon}.tgz" "${invalid_addon}/package/package.json"
done

npm_registry_fixture=${temporary_root}/npm-registry-platform.json
jq -n \
  --arg version "${release_version}" \
  --slurpfile staged "${linux_addon}/package/package.json" \
  '{versions: {($version): $staged[0]}}' >"${npm_registry_fixture}"
validate_npm_registry_platform_readback "${npm_registry_fixture}" \
  "${release_version}" "${linux_addon}.tgz"
jq --arg version "${release_version}" \
  '.versions[$version].libc = ["musl"]' "${npm_registry_fixture}" \
  >"${temporary_root}/npm-registry-platform-drift.json"
expect_rejection npm-registry-platform-drift \
  validate_npm_registry_platform_readback \
  "${temporary_root}/npm-registry-platform-drift.json" \
  "${release_version}" "${linux_addon}.tgz"

jq -n \
  --arg version "${release_version}" \
  --slurpfile staged "${darwin_addon}/package/package.json" \
  '{versions: {($version): $staged[0]}}' \
  >"${temporary_root}/npm-registry-darwin.json"
validate_npm_registry_platform_readback \
  "${temporary_root}/npm-registry-darwin.json" \
  "${release_version}" "${darwin_addon}.tgz"
jq --arg version "${release_version}" \
  '.versions[$version].libc = ["glibc"]' \
  "${temporary_root}/npm-registry-darwin.json" \
  >"${temporary_root}/npm-registry-darwin-drift.json"
expect_rejection npm-registry-darwin-drift \
  validate_npm_registry_platform_readback \
  "${temporary_root}/npm-registry-darwin-drift.json" \
  "${release_version}" "${darwin_addon}.tgz"

test "$(npm_provenance_policy \
  future-package https://github.com/weita2026/ait-native)" = \
  '--provenance'
test "$(npm_provenance_policy \
  future-package git+https://github.com/weita2026/ait-native.git)" = \
  '--provenance'
expect_failure npm-provenance-wrong-repository npm_provenance_policy \
  @wa120/ait-native-darwin-arm64 \
  https://github.com/example/other
expect_failure npm-provenance-missing-repository npm_provenance_policy \
  @wa120/ait-native-darwin-arm64 ''
for failure in wrong-repository missing-repository; do
  grep -F 'npm package repository metadata does not admit provenance:' \
    "${temporary_root}/npm-provenance-${failure}.stderr" >/dev/null
done

for component in ait-server ait-runner; do
  dockerfile=${repo_root}/release/oci/${component}.Dockerfile
  grep -F '# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e' \
    "${dockerfile}" >/dev/null
  grep -F 'FROM docker.io/library/debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241' \
    "${dockerfile}" >/dev/null
  grep -F "COPY --chmod=0755 bin/\${TARGETARCH}/${component} /usr/local/bin/${component}" \
    "${dockerfile}" >/dev/null
  if [[ ${component} == ait-server ]]; then
    if grep -Eiq '^[[:space:]]*CMD([[:space:]]|$)' "${dockerfile}"; then
      printf 'ait-server OCI recipe must not declare a Docker CMD\n' >&2
      exit 65
    fi
    grep -F '((.[0].Config.Cmd == null) or (.[0].Config.Cmd == [])) and' \
      "${remote}" >/dev/null
    if grep -F 'AITSERVER_LISTEN=' "${dockerfile}" >/dev/null; then
      printf 'ait-server OCI recipe restored the retired listener environment control\n' >&2
      exit 65
    fi
  fi
  if grep -E '(apt-get|cargo|curl|wget|git clone)' "${dockerfile}" >/dev/null; then
    printf '%s OCI recipe contains a build or download command\n' "${component}" >&2
    exit 65
  fi
done

fixture=${temporary_root}/fixture
mkdir -p "${fixture}/assets"
: >"${fixture}/assets/SHA256SUMS"
jq -n \
  --arg endpoint_config_sha256 "$(sha256_file "${endpoint_config}")" \
  --arg release_checksums_sha256 "$(sha256_file "${fixture}/assets/SHA256SUMS")" \
  --slurpfile config "${endpoint_config}" '
    {
      contract: "ait.release.family.endpoint-publication/v1",
      status: "ready_for_authenticated_endpoint_preflight",
      release_id: $config[0].release.id,
      version: $config[0].release.version,
      tag: $config[0].release.tag,
      endpoint_config_sha256: $endpoint_config_sha256,
      protected_evidence_sha256: $config[0].protected_authorization.evidence_sha256,
      release_checksums_sha256: $release_checksums_sha256,
      release_asset_count: 0,
      endpoints: $config[0].endpoints,
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
      }
    }
  ' >"${fixture}/ait-release.endpoint-publication.json"

missing_credentials() {
  unset AIT_GITHUB_TOKEN AIT_NPM_TOKEN AIT_HOMEBREW_DEPLOY_KEY \
    AIT_APT_REPO_DEPLOY_KEY AIT_APT_SIGNING_KEY_B64 \
    AIT_APT_SIGNING_PASSPHRASE AIT_APT_SIGNING_FINGERPRINT \
    AIT_PYPI_OIDC_PREFLIGHT AIT_GHCR_PREFLIGHT
  "${remote}" preflight "${endpoint_config}" "${fixture}"
}
expect_failure missing-credentials missing_credentials
grep -F 'required remote-publication environment is missing: AIT_GITHUB_TOKEN' \
  "${temporary_root}/missing-credentials.stderr" >/dev/null
if find "${fixture}/evidence" -mindepth 1 -type f -print -quit 2>/dev/null | grep -q .; then
  printf 'missing-credential preflight wrote endpoint evidence\n' >&2
  exit 65
fi

mock_bin=${temporary_root}/mock-bin
mkdir "${mock_bin}"
# shellcheck disable=SC2016 # These are literal lines for the mock executable.
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'output=' \
  'while (($# > 0)); do' \
  '  if [[ $1 == --output ]]; then output=$2; shift 2; continue; fi' \
  '  shift' \
  'done' \
  '[[ -n ${output} ]]' \
  'printf '\''{"full_name":"weita2026/ait-native","private":false}\n'\'' >"${output}"' \
  >"${mock_bin}/curl"
# shellcheck disable=SC2016 # These are literal lines for the mock executable.
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'if [[ ${1:-} == whoami ]]; then printf '\''wa120\n'\''; exit 0; fi' \
  'exit 64' \
  >"${mock_bin}/npm"
chmod 0700 "${mock_bin}/curl" "${mock_bin}/npm"
authenticated_github_token_without_user_permissions() {
  export AIT_GITHUB_TOKEN='secret-github-value'
  export AIT_NPM_TOKEN='secret-npm-value'
  export AIT_HOMEBREW_DEPLOY_KEY='secret-homebrew-value'
  export AIT_APT_REPO_DEPLOY_KEY='secret-apt-deploy-value'
  export AIT_APT_SIGNING_KEY_B64='secret-apt-key-value'
  export AIT_APT_SIGNING_PASSPHRASE='secret-apt-passphrase-value'
  export AIT_APT_SIGNING_FINGERPRINT='secret-apt-fingerprint-value'
  export AIT_PYPI_OIDC_PREFLIGHT=pass
  export AIT_GHCR_PREFLIGHT=pass
  PATH="${mock_bin}:${PATH}" "${remote}" preflight "${endpoint_config}" "${fixture}"
}
expect_failure authenticated-github authenticated_github_token_without_user_permissions
grep -F 'authenticated endpoint preflight start: GitHub repository token identity' \
  "${temporary_root}/authenticated-github.stdout" >/dev/null
grep -F 'authenticated endpoint preflight pass: GitHub repository token identity' \
  "${temporary_root}/authenticated-github.stdout" >/dev/null
grep -F 'authenticated endpoint preflight pass: npm authenticated publisher identity' \
  "${temporary_root}/authenticated-github.stdout" >/dev/null
grep -F 'authenticated endpoint preflight failed: npm staged identities and remote state (exit 65)' \
  "${temporary_root}/authenticated-github.stderr" >/dev/null
for secret_value in \
  secret-github-value secret-npm-value secret-homebrew-value \
  secret-apt-deploy-value secret-apt-key-value secret-apt-passphrase-value \
  secret-apt-fingerprint-value; do
  if grep -F "${secret_value}" \
    "${temporary_root}/authenticated-github.stdout" \
    "${temporary_root}/authenticated-github.stderr" >/dev/null; then
    printf 'authenticated preflight diagnostic exposed a credential\n' >&2
    exit 65
  fi
done

tampered=${temporary_root}/tampered
cp -R "${fixture}" "${tampered}"
printf 'tampered\n' >>"${tampered}/assets/SHA256SUMS"
expect_failure tampered-stage "${remote}" preflight "${endpoint_config}" "${tampered}"
grep -F 'release asset checksum inventory digest drifted' \
  "${temporary_root}/tampered-stage.stderr" >/dev/null

if [[ -n ${AIT_RELEASE_ENDPOINT_DOSSIER:-} ||
  -n ${AIT_RELEASE_ENDPOINT_PROTECTED_EVIDENCE:-} ]]; then
  if [[ -z ${AIT_RELEASE_ENDPOINT_DOSSIER:-} ||
    -z ${AIT_RELEASE_ENDPOINT_PROTECTED_EVIDENCE:-} ]]; then
    printf 'real endpoint fixture requires both dossier and protected evidence\n' >&2
    exit 64
  fi
  real_stage=${temporary_root}/real-stage
  "${preparer}" "${endpoint_config}" \
    "${AIT_RELEASE_ENDPOINT_DOSSIER}" \
    "${AIT_RELEASE_ENDPOINT_PROTECTED_EVIDENCE}" \
    "${real_stage}" >/dev/null
  jq -e '
    .status == "ready_for_authenticated_endpoint_preflight" and
    .release_asset_count == 82 and
    .mutation.registry_write == false and
    .mutation.github_release_write == false
  ' "${real_stage}/ait-release.endpoint-publication.json" >/dev/null
  test "$(find "${real_stage}/assets" -mindepth 1 -maxdepth 1 -type f | wc -l | tr -d '[:space:]')" = 83
fi

printf 'release endpoint publication contract: pass\n'
