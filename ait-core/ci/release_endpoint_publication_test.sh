#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
workflow=${repo_root}/.github/workflows/pypi-publish.yml
endpoint_config=${repo_root}/release/endpoint-publication.rc3.json
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
  /- name: Publish the signed apt testing repository/ { apt_step = NR }
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
eval "$(extract_remote_function npm_provenance_policy)"
eval "$(extract_remote_function remove_matching_npm_prerelease_latest_tag)"
eval "$(extract_remote_function validate_npm_dist_tags)"
eval "$(extract_remote_function wait_for_pypi_remote_state)"

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
grep -F 'PyPI RC wheel set did not become fully visible after 12 attempts' \
  "${temporary_root}/pypi-eventual-timeout.stderr" >/dev/null
unset -f validate_pypi_remote_state sleep wait_for_pypi_remote_state

github_asset_fixture=${temporary_root}/github-asset-fixture
mkdir -p "${github_asset_fixture}/assets"
assets=${github_asset_fixture}/assets
stage_receipt=${github_asset_fixture}/ait-release.endpoint-publication.json
: >"${assets}/ait-native_1.0.0~rc.3_amd64.deb"
: >"${assets}/plain.zip"
: >"${stage_receipt}"
github_asset_map=${github_asset_fixture}/asset-map
github_release_asset_map "${github_asset_map}"
test "$(wc -l <"${github_asset_map}" | tr -d '[:space:]')" = 3
grep -F $'ait-native_1.0.0.rc.3_amd64.deb\t'"${assets}/ait-native_1.0.0~rc.3_amd64.deb" \
  "${github_asset_map}" >/dev/null
test "$(github_release_local_path \
  "${github_asset_map}" 'ait-native_1.0.0.rc.3_amd64.deb')" = \
  "${assets}/ait-native_1.0.0~rc.3_amd64.deb"
test "$(github_release_asset_name 'plain~name.zip')" = 'plain~name.zip'
: >"${assets}/ait-native_1.0.0.rc.3_amd64.deb"
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
  '    matching) printf '\''latest: 1.0.0-rc.3\nrc: 1.0.0-rc.3\n'\''' \
  '      ;;' \
  '    stable) printf '\''latest: 0.9.0\nrc: 1.0.0-rc.3\n'\''' \
  '      ;;' \
  '    no-latest) printf '\''rc: 1.0.0-rc.3\n'\''' \
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
  "${npmrc}" matching 1.0.0-rc.3 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" stable 1.0.0-rc.3 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" no-latest 1.0.0-rc.3 rc
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0 latest
PATH="${npm_tag_mock}:${PATH}" \
  remove_matching_npm_prerelease_latest_tag \
  "${npmrc}" matching 1.0.0-rc.3 latest
test "$(wc -l <"${npm_tag_log}" | tr -d '[:space:]')" = 1
grep -Fx 'dist-tag rm matching latest --registry https://registry.npmjs.org' \
  "${npm_tag_log}" >/dev/null

jq -n '{"dist-tags": {rc: "1.0.0-rc.3", latest: "0.9.0"}}' \
  >"${temporary_root}/npm-tags-valid.json"
validate_npm_dist_tags "${temporary_root}/npm-tags-valid.json" \
  ait-native 1.0.0-rc.3 rc
jq -n '{"dist-tags": {rc: "1.0.0-rc.3", latest: "1.0.0-rc.3"}}' \
  >"${temporary_root}/npm-tags-rc-latest.json"
expect_failure npm-rc-remains-latest validate_npm_dist_tags \
  "${temporary_root}/npm-tags-rc-latest.json" ait-native 1.0.0-rc.3 rc
grep -F 'npm prerelease remains the default latest tag: ait-native@1.0.0-rc.3' \
  "${temporary_root}/npm-rc-remains-latest.stderr" >/dev/null
jq -n '{"dist-tags": {rc: "1.0.0-rc.0", latest: "0.9.0"}}' \
  >"${temporary_root}/npm-tags-wrong-rc.json"
expect_failure npm-rc-tag-drift validate_npm_dist_tags \
  "${temporary_root}/npm-tags-wrong-rc.json" ait-native 1.0.0-rc.3 rc
grep -F 'npm RC dist-tag readback failed: ait-native@1.0.0-rc.3' \
  "${temporary_root}/npm-rc-tag-drift.stderr" >/dev/null

export github_repository=weita2026/ait-native
export release_version=1.0.0-rc.3
export source_commit=ba368cf4d0750035345f14a8a91c22fb9e450260
test "$(npm_provenance_policy \
  future-package unused https://github.com/weita2026/ait-native)" = \
  '--provenance'
test "$(npm_provenance_policy \
  future-package unused git+https://github.com/weita2026/ait-native.git)" = \
  '--provenance'
npm_recovery_rows=${temporary_root}/npm-provenance-recovery-rows
printf '%s\t%s\n' \
  ait-native 8862dc3621320fda30e6923c85eee872751bfc92d95f319382b5b690540392f8 \
  ait-native-ait-darwin-arm64 262b2860df61c64dd8c358d0e36c5ae136ae0f98ee1bbc04511ba0608313abd2 \
  ait-native-ait-darwin-x64 42cb08e1651e8d96cd4dfc56cabf63ce0c50629b9c122ce065351cf67747e870 \
  ait-native-ait-linux-arm64 e59c1d29819454d20943dad038e7e3273d114c29033ff22d2430ae427778b221 \
  ait-native-ait-linux-x64 b594c192d921aa2e9c0fd6868d477b76fb431bac6ef0b89c8c0f011ac5cc1843 \
  ait-native-ait-win32-arm64 510ad5a977948c9a71c5434f721370fd2265b7665d3198bac597e563d2d4a8be \
  ait-native-ait-win32-x64 7e09475a008b9a36993c9efe89ee99fd493da0457d06ca349143449e95b3298b \
  >"${npm_recovery_rows}"
while IFS=$'\t' read -r recovery_package recovery_sha256; do
  test "$(npm_provenance_policy \
    "${recovery_package}" "${recovery_sha256}" '')" = \
    '--provenance=false'
done <"${npm_recovery_rows}"
expect_failure npm-provenance-wrong-digest npm_provenance_policy \
  ait-native-ait-darwin-arm64 \
  0000000000000000000000000000000000000000000000000000000000000000 ''
expect_failure npm-provenance-wrong-repository npm_provenance_policy \
  ait-native-ait-darwin-arm64 \
  262b2860df61c64dd8c358d0e36c5ae136ae0f98ee1bbc04511ba0608313abd2 \
  https://github.com/example/other
export source_commit=0000000000000000000000000000000000000000
expect_failure npm-provenance-wrong-source npm_provenance_policy \
  ait-native-ait-darwin-arm64 \
  262b2860df61c64dd8c358d0e36c5ae136ae0f98ee1bbc04511ba0608313abd2 ''
for failure in wrong-digest wrong-repository wrong-source; do
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
