#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
config=${repo_root}/release/npm-namespace-supplement.rc3.json
preparer=${repo_root}/ci/release_npm_namespace_supplement.mjs
remote=${repo_root}/ci/release_npm_namespace_remote.sh
workflow=${repo_root}/.github/workflows/npm-namespace-supplement.yml
node_root=${AIT_NODE_ROOT:-${repo_root}/../ait-node}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-npm-supplement-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-npm-supplement-test.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected npm supplement test path: %s\n' \
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
    printf 'expected npm namespace supplement failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

for file in "${config}" "${preparer}" "${remote}" "${workflow}"; do
  if [[ ! -f ${file} || -L ${file} ]]; then
    printf 'npm namespace supplement input is unavailable: %s\n' "${file}" >&2
    exit 66
  fi
done
if [[ ! -d ${node_root} || -L ${node_root} ]]; then
  printf 'ait-node source root is unavailable: %s\n' "${node_root}" >&2
  exit 66
fi

node --check "${preparer}"
bash -n "${remote}"
jq empty "${config}"

jq -e '
  .contract == "ait.release.npm-namespace-supplement/v1" and
  .release == {
    id: "REL-FAM-600EFDC327FE7860",
    version: "1.0.0-rc.3",
    tag: "v1.0.0-rc.3",
    tag_object: "810265c705ffececba3d74924f60ed2d0453ef7d",
    source_commit: "ba368cf4d0750035345f14a8a91c22fb9e450260",
    github_release_id: 369674917,
    source_dossier_run_id: 31664713921,
    source_dossier_artifact_id: 9167933771,
    source_dossier_artifact_digest: "sha256:08afc391688c902f3c2259392286b51612e6b6eb0aa51c388e8e513329705823",
    protected_authorization_run_id: 31666479359,
    protected_authorization_artifact_id: 9168120753,
    protected_authorization_artifact_digest: "sha256:54079d53bc3e115f314d99591228cc80dbab4c56c5ae361530f9c490c0764be9",
    protected_authorization_evidence_sha256: "cc18cf39db59147d5ee94359f0c00813be6841bf317686451eafd1152f870b32",
    failed_endpoint_run_id: 31668411148,
    failed_endpoint_workflow_commit: "30672445b7321226f81db280f3e2531ad6fc2a5d"
  } and
  .node_source.snapshot == "SNP-22993C1FEF52" and
  .node_source.binding_snapshot == "SNP-158C9C5BB3D7" and
  .registry.packages == [
    "@wa120/ait-native",
    "@wa120/ait-native-darwin-arm64",
    "@wa120/ait-native-darwin-x64",
    "@wa120/ait-native-linux-arm64",
    "@wa120/ait-native-linux-x64",
    "@wa120/ait-native-win32-arm64",
    "@wa120/ait-native-win32-x64"
  ] and
  ([.addons[].source_github_release_asset_id] | length) == 6 and
  ([.addons[].native_sha256] | unique | length) == 6 and
  .mutation.native_addon_rebuild == false and
  .mutation.release_family_rebuild == false and
  .mutation.tag_write == false and
  .mutation.github_release_write == false and
  .mutation.existing_unscoped_package_write == false
' "${config}" >/dev/null

# shellcheck disable=SC2016 # These are literal workflow-contract fragments.
for required in \
  'name: ait RC3 scoped npm namespace supplement' \
  'actions: read' \
  'attestations: write' \
  'contents: read' \
  'id-token: write' \
  '      name: pypi' \
  "node-version: '22.17.1'" \
  'test "$(npm --version)" = '\''10.9.2'\''' \
  'release_npm_namespace_supplement.mjs prepare' \
  'release_npm_namespace_remote.sh preflight' \
  'release_npm_namespace_remote.sh publish' \
  'release_npm_namespace_remote.sh readback' \
  'secrets.AIT_NPM_TOKEN' \
  'actions/attest-build-provenance@977bb373ede98d70efdf65b84cb5f73e068dcc2a' \
  'actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02'; do
  if ! grep -F -- "${required}" "${workflow}" >/dev/null; then
    printf 'npm supplement workflow is missing: %s\n' "${required}" >&2
    exit 65
  fi
done
if awk '/^[[:space:]]*uses:/ && $0 !~ /@[0-9a-f]{40}([[:space:]]|$)/ {print; bad=1} END {exit bad}' \
  "${workflow}"; then
  :
else
  printf 'npm supplement workflow contains an unpinned action\n' >&2
  exit 65
fi
for forbidden in \
  'contents: write' \
  'packages: write' \
  'continue-on-error:' \
  'cargo build' \
  'npm run build' \
  'gh release upload' \
  'gh release create' \
  'git tag' \
  'npm unpublish' \
  'npm deprecate' \
  'ait-native-ait-win32-x64@'; do
  if grep -F -- "${forbidden}" "${workflow}" >/dev/null; then
    printf 'npm supplement workflow contains forbidden behavior: %s\n' \
      "${forbidden}" >&2
    exit 65
  fi
done
if ! awk '
  /release_npm_namespace_remote.sh preflight/ { preflight = NR }
  /Attest every renamed npm archive before publication/ { attest = NR }
  /release_npm_namespace_remote.sh publish/ { publish = NR }
  /release_npm_namespace_remote.sh readback/ { readback = NR }
  END { exit !(preflight > 0 && attest > preflight && publish > attest && readback > publish) }
' "${workflow}"; then
  printf 'npm supplement protected publication ordering drifted\n' >&2
  exit 65
fi
for forbidden in \
  'npm unpublish' \
  'npm deprecate' \
  'gh release' \
  'git tag' \
  'task publish' \
  'AIT_RELEASE_SERVER_URL'; do
  if grep -F -- "${forbidden}" "${remote}" "${preparer}" >/dev/null; then
    printf 'npm supplement implementation contains forbidden behavior: %s\n' \
      "${forbidden}" >&2
    exit 65
  fi
done
# shellcheck disable=SC2016 # These are literal remote-contract fragments.
for required_remote in \
  'for addon_index in 0 1 2 3 4 5; do' \
  'while IFS=$'\''\t'\'' read -r package_name filename expected_sha1 expected_integrity; do' \
  'AIT_STAGE_ATTESTATION_VERIFIED' \
  '--access public' \
  '--ignore-scripts' \
  '--provenance' \
  '--tag "${dist_tag}"' \
  'npm dist-tag rm' \
  'wait_for_exact_registry false'; do
  if ! grep -F -- "${required_remote}" "${remote}" >/dev/null; then
    printf 'npm supplement remote contract is missing: %s\n' \
      "${required_remote}" >&2
    exit 65
  fi
done

fixture_assets=${temporary_root}/fixture-assets
fixture_config=${temporary_root}/fixture-config.json
fixture_source=${temporary_root}/fixture-source
mkdir "${fixture_assets}" "${fixture_source}"
cp "${config}" "${fixture_config}"

index=0
while IFS=$'\t' read -r target os cpu source_package source_filename native_size; do
  package_root=${temporary_root}/package-${index}/package
  mkdir -p "${package_root}/native"
  cp "${node_root}/LICENSE" "${package_root}/LICENSE"
  cp "${node_root}/NOTICE" "${package_root}/NOTICE"
  awk -v size="${native_size}" 'BEGIN {
    for (i = 0; i < size; i += 1) printf "%c", ((i * 31 + 17) % 251) + 1
  }' >"${package_root}/native/ait_napi.node"
  native_sha=$(sha256_file "${package_root}/native/ait_napi.node")
  jq -n \
    --arg name "${source_package}" \
    --arg version '1.0.0-rc.3' \
    --arg target "${target}" \
    --arg os "${os}" \
    --arg cpu "${cpu}" '
      {
        name: $name,
        version: $version,
        description: ("Implementation-only AIT Node-API addon for " + $target),
        license: "Apache-2.0",
        os: [$os],
        cpu: [$cpu],
        main: "native/ait_napi.node",
        files: ["native", "provenance.json", "LICENSE", "NOTICE"],
        aitNativeAddon: {
          schema: "ait.node.napi-platform-addon/v1",
          component: "ait-node",
          target: $target,
          addon: "native/ait_napi.node",
          binding_repository: "ait-core",
          binding_snapshot: "SNP-158C9C5BB3D7"
        }
      }
    ' >"${package_root}/package.json"
  jq -n \
    --arg package "${source_package}" \
    --arg target "${target}" \
    --arg os "${os}" \
    --arg cpu "${cpu}" \
    --arg native_sha "${native_sha}" \
    --argjson native_size "${native_size}" \
    --arg license_sha "$(sha256_file "${node_root}/LICENSE")" \
    --argjson license_size "$(wc -c <"${node_root}/LICENSE" | tr -d '[:space:]')" \
    --arg notice_sha "$(sha256_file "${node_root}/NOTICE")" \
    --argjson notice_size "$(wc -c <"${node_root}/NOTICE" | tr -d '[:space:]')" '
      {
        schema: "ait.node.napi-platform-addon-provenance/v1",
        family_version: "1.0.0-rc.3",
        package: $package,
        target: $target,
        os: $os,
        cpu: $cpu,
        component: "ait-node",
        package_source_repository: "ait-node",
        binding_repository: "ait-core",
        binding_snapshot: "SNP-158C9C5BB3D7",
        license: "Apache-2.0",
        license_file: {path: "LICENSE", sha256: $license_sha, size_bytes: $license_size},
        notice_file: {path: "NOTICE", sha256: $notice_sha, size_bytes: $notice_size},
        source_artifact: {sha256: $native_sha, size_bytes: $native_size},
        installed_path: "native/ait_napi.node"
      }
    ' >"${package_root}/provenance.json"
  npm pack --ignore-scripts --json --pack-destination "${fixture_assets}" \
    "${package_root}" >"${temporary_root}/pack-${index}.json"
  packed=$(jq -er '.[0].filename' "${temporary_root}/pack-${index}.json")
  test "${packed}" = "${source_filename}"
  source_sha=$(sha256_file "${fixture_assets}/${source_filename}")
  source_size=$(wc -c <"${fixture_assets}/${source_filename}" | tr -d '[:space:]')
  jq \
    --argjson index "${index}" \
    --arg native_sha "${native_sha}" \
    --argjson native_size "${native_size}" \
    --arg source_sha "${source_sha}" \
    --argjson source_size "${source_size}" '
      .addons[$index].native_sha256 = $native_sha |
      .addons[$index].native_size_bytes = $native_size |
      .addons[$index].source_sha256 = $source_sha |
      .addons[$index].source_size_bytes = $source_size
    ' "${fixture_config}" >"${fixture_config}.next"
  mv "${fixture_config}.next" "${fixture_config}"
  index=$((index + 1))
done < <(jq -r '.addons[] | [.target, .os, .cpu, .source_package, .source_filename, (2048 + (.source_github_release_asset_id % 257) | tostring)] | @tsv' "${config}")

test "${index}" = 6
fixture_output=${temporary_root}/fixture-output
node "${preparer}" prepare-fixture \
  "${fixture_config}" "${node_root}" "${fixture_assets}" "${fixture_output}" \
  >"${temporary_root}/fixture-prepare.json"
jq -e '
  .contract == "ait.release.npm-namespace-supplement.fixture-stage/v1" and
  .status == "test_fixture_only" and
  (.packages | length) == 7 and
  ([.packages[].order] == [1, 2, 3, 4, 5, 6, 7]) and
  ([.addon_mappings[].native_bytes_identical] | all(. == true)) and
  .mutation.native_addon_rebuild == false
' "${fixture_output}/ait-release.npm-namespace-supplement.json" >/dev/null
test "$(find "${fixture_output}/packages" -mindepth 1 -maxdepth 1 -type f | wc -l | tr -d '[:space:]')" = 7

index=0
while IFS=$'\t' read -r source_filename scoped_filename native_sha; do
  tar -xOf "${fixture_assets}/${source_filename}" package/native/ait_napi.node \
    >"${temporary_root}/source-${index}.node"
  tar -xOf "${fixture_output}/packages/${scoped_filename}" package/native/ait_napi.node \
    >"${temporary_root}/scoped-${index}.node"
  cmp "${temporary_root}/source-${index}.node" "${temporary_root}/scoped-${index}.node"
  test "$(sha256_file "${temporary_root}/scoped-${index}.node")" = "${native_sha}"
  index=$((index + 1))
done < <(jq -r '.addon_mappings[] | [.source_filename, .scoped_filename, .native_sha256] | @tsv' "${fixture_output}/ait-release.npm-namespace-supplement.json")
test "${index}" = 6

tampered_assets=${temporary_root}/tampered-assets
cp -R "${fixture_assets}" "${tampered_assets}"
tampered_name=$(jq -er '.addons[0].source_filename' "${fixture_config}")
printf 'tamper\n' >>"${tampered_assets}/${tampered_name}"
expect_failure archive-tamper node "${preparer}" prepare-fixture \
  "${fixture_config}" "${node_root}" "${tampered_assets}" \
  "${temporary_root}/tampered-output"
grep -E '(size|digest) drifted' \
  "${temporary_root}/archive-tamper.stderr" >/dev/null

metadata_assets=${temporary_root}/metadata-assets
metadata_config=${temporary_root}/metadata-config.json
cp -R "${fixture_assets}" "${metadata_assets}"
cp "${fixture_config}" "${metadata_config}"
metadata_name=$(jq -er '.addons[1].source_filename' "${metadata_config}")
metadata_root=${temporary_root}/metadata-repack
mkdir "${metadata_root}"
tar -xzf "${metadata_assets}/${metadata_name}" -C "${metadata_root}"
jq '.license = "MIT"' "${metadata_root}/package/package.json" \
  >"${metadata_root}/package/package.json.next"
mv "${metadata_root}/package/package.json.next" "${metadata_root}/package/package.json"
rm "${metadata_assets}/${metadata_name}"
npm pack --ignore-scripts --json --pack-destination "${metadata_assets}" \
  "${metadata_root}/package" >"${temporary_root}/metadata-pack.json"
metadata_sha=$(sha256_file "${metadata_assets}/${metadata_name}")
metadata_size=$(wc -c <"${metadata_assets}/${metadata_name}" | tr -d '[:space:]')
jq \
  --arg digest "${metadata_sha}" \
  --argjson size "${metadata_size}" \
  '.addons[1].source_sha256 = $digest | .addons[1].source_size_bytes = $size' \
  "${metadata_config}" >"${metadata_config}.next"
mv "${metadata_config}.next" "${metadata_config}"
expect_failure metadata-tamper node "${preparer}" prepare-fixture \
  "${metadata_config}" "${node_root}" "${metadata_assets}" \
  "${temporary_root}/metadata-output"
grep -F 'original addon metadata drifted' \
  "${temporary_root}/metadata-tamper.stderr" >/dev/null

printf '%s\n' \
  '{"contract":"ait.release.npm-namespace-supplement.tests/v1","status":"pass"}'
