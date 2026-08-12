#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
workflow=${repo_root}/.github/workflows/pypi-publish.yml
endpoint_config=${repo_root}/release/endpoint-publication.rc2.json
preparer=${repo_root}/ci/release_endpoint_publication.sh
remote=${repo_root}/ci/release_endpoint_remote.sh
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

for path in "${workflow}" "${endpoint_config}" "${preparer}" "${remote}" \
  "${repo_root}/release/oci/ait-server.Dockerfile" \
  "${repo_root}/release/oci/ait-runner.Dockerfile"; do
  if [[ ! -f ${path} || -L ${path} ]]; then
    printf 'endpoint-publication input is unavailable: %s\n' "${path}" >&2
    exit 66
  fi
done

bash -n "${preparer}"
bash -n "${remote}"
jq -e '
  .contract == "ait.release.family.endpoints/v1" and
  .release == {
    id: "REL-FAM-0B6EDBCCA2EFE26B",
    version: "1.0.0-rc.2",
    python_version: "1.0.0rc2",
    tag: "v1.0.0-rc.2",
    source_commit: "3dfd9dde5a9867cfe265352f48540fa8241f8e66",
    coordinator_snapshot: "SNP-152CBCB22EAC",
    frozen_manifest_sha256: "38cf7635d398294e2a433caf4d54444fdf97ffdfaa8307a0782247939841ac56",
    frozen_checksums_sha256: "51cadec032ca6bcab2a27fc60538886a90288e1c9eff97a82e7a15be13fd3896"
  } and
  .publisher == {
    repository: "weita2026/ait-native",
    workflow: "pypi-publish.yml",
    environment: "pypi"
  } and
  .endpoints.npm.packages == [
    "ait-native",
    "ait-native-ait-darwin-arm64",
    "ait-native-ait-darwin-x64",
    "ait-native-ait-linux-arm64",
    "ait-native-ait-linux-x64",
    "ait-native-ait-win32-arm64",
    "ait-native-ait-win32-x64"
  ] and
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
  'control/ci/release_endpoint_publication.sh' \
  'control/ci/release_endpoint_remote.sh preflight' \
  'secrets.AIT_NPM_TOKEN' \
  'docker logout ghcr.io'; do
  grep -F -- "${required}" "${workflow}" >/dev/null
done
if [[ $(grep -c '^        continue-on-error: true$' "${workflow}") != 1 ]] ||
  ! awk '
    /- name: Publish npm implementation payloads and command envelope/ {
      npm_step = 1
      next
    }
    npm_step && /- name: / { exit }
    npm_step && /continue-on-error: true/ { isolated = 1 }
    END { exit isolated ? 0 : 1 }
  ' "${workflow}"; then
  printf 'only the npm publication step may yield to independent endpoints\n' >&2
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
if awk '/^[[:space:]]*uses:/ && $0 !~ /@[0-9a-f]{40}([[:space:]]|$)/ {print; bad=1} END {exit bad}' \
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
eval "$(extract_remote_function github_release_asset_name)"
eval "$(extract_remote_function github_release_asset_map)"
eval "$(extract_remote_function github_release_local_path)"
eval "$(extract_remote_function remove_matching_npm_prerelease_latest_tag)"
eval "$(extract_remote_function validate_npm_dist_tags)"
github_asset_fixture=${temporary_root}/github-asset-fixture
mkdir -p "${github_asset_fixture}/assets"
assets=${github_asset_fixture}/assets
stage_receipt=${github_asset_fixture}/ait-release.endpoint-publication.json
: >"${assets}/ait-native_1.0.0~rc.2_amd64.deb"
: >"${assets}/plain.zip"
: >"${stage_receipt}"
github_asset_map=${github_asset_fixture}/asset-map
github_release_asset_map "${github_asset_map}"
test "$(wc -l <"${github_asset_map}" | tr -d '[:space:]')" = 3
grep -F $'ait-native_1.0.0.rc.2_amd64.deb\t'"${assets}/ait-native_1.0.0~rc.2_amd64.deb" \
  "${github_asset_map}" >/dev/null
test "$(github_release_local_path \
  "${github_asset_map}" 'ait-native_1.0.0.rc.2_amd64.deb')" = \
  "${assets}/ait-native_1.0.0~rc.2_amd64.deb"
test "$(github_release_asset_name 'plain~name.zip')" = 'plain~name.zip'
: >"${assets}/ait-native_1.0.0.rc.2_amd64.deb"
expect_failure github-asset-name-collision \
  github_release_asset_map "${github_asset_fixture}/colliding-map"

npm_tag_mock=${temporary_root}/npm-tag-mock
mkdir "${npm_tag_mock}"
npm_tag_log=${temporary_root}/npm-tag.log
# shellcheck disable=SC2016 # These are literal lines for the mock executable.
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'if [[ $1 == dist-tag && $2 == ls ]]; then' \
  '  case "$3" in' \
  '    matching) printf '\''latest: 1.0.0-rc.2\nrc: 1.0.0-rc.2\n'\''' \
  '      ;;' \
  '    stable) printf '\''latest: 0.9.0\nrc: 1.0.0-rc.2\n'\''' \
  '      ;;' \
  '    no-latest) printf '\''rc: 1.0.0-rc.2\n'\''' \
  '      ;;' \
  '    *) exit 64 ;;' \
  '  esac' \
  '  exit 0' \
  'fi' \
  'if [[ $1 == dist-tag && $2 == rm ]]; then' \
  '  printf '\''%s\n'\'' "$*" >>"${AIT_NPM_TAG_TEST_LOG}"' \
  '  exit 0' \
  'fi' \
  'exit 64' \
  >"${npm_tag_mock}/npm"
chmod 0700 "${npm_tag_mock}/npm"
: >"${npm_tag_log}"
export npm_registry=https://registry.npmjs.org
npmrc=${temporary_root}/npm-tag-test.npmrc
: >"${npmrc}"
export AIT_NPM_TAG_TEST_LOG=${npm_tag_log}
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0-rc.2 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" stable 1.0.0-rc.2 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" no-latest 1.0.0-rc.2 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0 latest
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0-rc.2 latest
test "$(wc -l <"${npm_tag_log}" | tr -d '[:space:]')" = 1
grep -Fx 'dist-tag rm matching latest --registry https://registry.npmjs.org' \
  "${npm_tag_log}" >/dev/null

jq -n '{"dist-tags": {rc: "1.0.0-rc.2", latest: "0.9.0"}}' \
  >"${temporary_root}/npm-tags-valid.json"
validate_npm_dist_tags "${temporary_root}/npm-tags-valid.json" \
  ait-native 1.0.0-rc.2 rc
jq -n '{"dist-tags": {rc: "1.0.0-rc.2", latest: "1.0.0-rc.2"}}' \
  >"${temporary_root}/npm-tags-rc-latest.json"
expect_failure npm-rc-remains-latest validate_npm_dist_tags \
  "${temporary_root}/npm-tags-rc-latest.json" ait-native 1.0.0-rc.2 rc
grep -F 'npm prerelease remains the default latest tag: ait-native@1.0.0-rc.2' \
  "${temporary_root}/npm-rc-remains-latest.stderr" >/dev/null
jq -n '{"dist-tags": {rc: "1.0.0-rc.0", latest: "0.9.0"}}' \
  >"${temporary_root}/npm-tags-wrong-rc.json"
expect_failure npm-rc-tag-drift validate_npm_dist_tags \
  "${temporary_root}/npm-tags-wrong-rc.json" ait-native 1.0.0-rc.2 rc
grep -F 'npm RC dist-tag readback failed: ait-native@1.0.0-rc.2' \
  "${temporary_root}/npm-rc-tag-drift.stderr" >/dev/null

for component in ait-server ait-runner; do
  dockerfile=${repo_root}/release/oci/${component}.Dockerfile
  grep -F '# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e' \
    "${dockerfile}" >/dev/null
  grep -F 'FROM docker.io/library/debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241' \
    "${dockerfile}" >/dev/null
  grep -F "COPY --chmod=0755 bin/\${TARGETARCH}/${component} /usr/local/bin/${component}" \
    "${dockerfile}" >/dev/null
  if [[ ${component} == ait-server ]]; then
    grep -F 'AITSERVER_LISTEN=0.0.0.0:8088' "${dockerfile}" >/dev/null
    grep -F 'AITSERVER_LISTEN=0.0.0.0:8088' "${remote}" >/dev/null
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
