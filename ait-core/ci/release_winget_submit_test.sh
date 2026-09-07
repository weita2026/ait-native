#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-winget.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-winget.*) rm -rf -- "${temporary_root}" ;;
    *) return 1 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
mkdir -p "${temporary_root}/bin" "${temporary_root}/payload/nested"
manifest_root=${temporary_root}/payload/nested
version=1.2.0
arm64_payload='arm64 release asset'
x64_payload='x64 release asset'
arm64_hash=$(printf '%s' "${arm64_payload}" | shasum -a 256 | awk '{print toupper($1)}')
x64_hash=$(printf '%s' "${x64_payload}" | shasum -a 256 | awk '{print toupper($1)}')

cat >"${manifest_root}/Weita.AitNative.yaml" <<EOF
PackageIdentifier: Weita.AitNative
PackageVersion: ${version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
EOF
cat >"${manifest_root}/Weita.AitNative.installer.yaml" <<EOF
PackageIdentifier: Weita.AitNative
PackageVersion: ${version}
InstallerType: zip
Installers:
  - Architecture: arm64
    InstallerUrl: https://github.com/owner/ait-native/releases/download/v${version}/ait-native-${version}-aarch64-pc-windows-msvc.zip
    InstallerSha256: ${arm64_hash}
  - Architecture: x64
    InstallerUrl: https://github.com/owner/ait-native/releases/download/v${version}/ait-native-${version}-x86_64-pc-windows-msvc.zip
    InstallerSha256: ${x64_hash}
ManifestType: installer
ManifestVersion: 1.12.0
EOF
cat >"${manifest_root}/Weita.AitNative.locale.en-US.yaml" <<EOF
PackageIdentifier: Weita.AitNative
PackageVersion: ${version}
PackageLocale: en-US
Publisher: Weita
PackageName: ait-native
License: AGPL-3.0-only AND Apache-2.0
ShortDescription: AIT native CLI
ManifestType: defaultLocale
ManifestVersion: 1.12.0
EOF

cat >"${temporary_root}/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == api ]]
endpoint=${2:-}
case "${endpoint}" in
  repos/microsoft/winget-pkgs/pulls\?*)
    printf '%s\n' '[{"number":42,"html_url":"https://example.invalid/pull/42","state":"open","merged_at":null}]'
    ;;
  repos/microsoft/winget-pkgs/pulls/42)
    printf '%s\n' '{"number":42,"html_url":"https://example.invalid/pull/42","state":"open","merged_at":null}'
    ;;
  *)
    printf 'unexpected fixture endpoint: %s\n' "${endpoint}" >&2
    exit 64
    ;;
esac
GH
chmod +x "${temporary_root}/bin/gh"

prefix="manifests/w/Weita/AitNative/${version}"
jq -n --arg version "${version}" --arg prefix "${prefix}" \
  --arg v "$(shasum -a 256 "${manifest_root}/Weita.AitNative.yaml" | awk '{print $1}')" \
  --arg i "$(shasum -a 256 "${manifest_root}/Weita.AitNative.installer.yaml" | awk '{print $1}')" \
  --arg l "$(shasum -a 256 "${manifest_root}/Weita.AitNative.locale.en-US.yaml" | awk '{print $1}')" '
  {
    contract:"ait.release.winget-submission/v1",version:$version,identity:"Weita.AitNative",
    release_repository:"owner/ait-native",fork:"owner/winget-pkgs",upstream:"microsoft/winget-pkgs",
    branch:("new-package-Weita.AitNative-"+$version),
    manifests:[
      {path:($prefix+"/Weita.AitNative.yaml"),sha256:$v},
      {path:($prefix+"/Weita.AitNative.installer.yaml"),sha256:$i},
      {path:($prefix+"/Weita.AitNative.locale.en-US.yaml"),sha256:$l}
    ],
    pull_request:{number:42,url:"https://example.invalid/pull/42",state:"open"},
    status:"submitted",submitted_at:"2026-09-07T00:00:00Z"
  }
' >"${temporary_root}/receipt.json"

common=(--version "${version}" --manifests "${temporary_root}/payload" \
  --output "${temporary_root}/receipt.json" --release-repository owner/ait-native \
  --fork owner/winget-pkgs --upstream microsoft/winget-pkgs)
PATH="${temporary_root}/bin:${PATH}" node "${repo_root}/ci/release_winget_submit.mjs" probe "${common[@]}"
PATH="${temporary_root}/bin:${PATH}" node "${repo_root}/ci/release_winget_submit.mjs" status "${common[@]}" \
  >"${temporary_root}/status.json"
jq -e '.contract == "ait.release.winget-status/v1" and .status == "submitted" and .pull_request.number == 42' \
  "${temporary_root}/status.json" >/dev/null

rm "${temporary_root}/receipt.json"
cat >"${temporary_root}/fetch.cjs" <<'JS'
globalThis.fetch = async (url) => {
  const value = String(url);
  const payload = value.includes("aarch64-pc-windows-msvc.zip")
    ? process.env.TEST_ARM64_PAYLOAD
    : value.includes("x86_64-pc-windows-msvc.zip")
      ? process.env.TEST_X64_PAYLOAD
      : null;
  if (payload === null) return { ok: false, arrayBuffer: async () => Buffer.alloc(0) };
  return { ok: true, arrayBuffer: async () => Buffer.from(payload) };
};
JS
cat >"${temporary_root}/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == api ]]
shift
method=GET
if [[ ${1:-} == -X ]]; then
  method=$2
  shift 2
fi
endpoint=${1:-}
shift || true
body=''
if [[ ${1:-} == --input ]]; then
  body=$(cat)
fi
printf '%s\t%s\t%s\n' "${method}" "${endpoint}" "${body}" >>"${TEST_GH_LOG}"
case "${method} ${endpoint}" in
  "GET repos/owner/ait-native/releases/tags/v${TEST_VERSION}")
    jq -n --arg version "${TEST_VERSION}" '{assets:[
      {name:("ait-native-"+$version+"-aarch64-pc-windows-msvc.zip"),browser_download_url:("https://github.com/owner/ait-native/releases/download/v"+$version+"/ait-native-"+$version+"-aarch64-pc-windows-msvc.zip")},
      {name:("ait-native-"+$version+"-x86_64-pc-windows-msvc.zip"),browser_download_url:("https://github.com/owner/ait-native/releases/download/v"+$version+"/ait-native-"+$version+"-x86_64-pc-windows-msvc.zip")}
    ]}'
    ;;
  "GET repos/microsoft/winget-pkgs/pulls?"*)
    printf '%s\n' '[]'
    ;;
  "GET repos/owner/winget-pkgs/git/ref/heads/new-package-Weita.AitNative-${TEST_VERSION}")
    exit 1
    ;;
  "GET repos/owner/winget-pkgs/git/ref/heads/master")
    printf '%s\n' '{"object":{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}'
    ;;
  "GET repos/microsoft/winget-pkgs/git/ref/heads/master")
    printf '%s\n' '{"object":{"sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}'
    ;;
  "GET repos/microsoft/winget-pkgs/compare/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    printf '%s\n' '{"status":"ahead"}'
    ;;
  "GET repos/owner/winget-pkgs/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    printf '%s\n' '{"tree":{"sha":"cccccccccccccccccccccccccccccccccccccccc"}}'
    ;;
  "POST repos/owner/winget-pkgs/git/blobs")
    printf '%s\n' '{"sha":"dddddddddddddddddddddddddddddddddddddddd"}'
    ;;
  "POST repos/owner/winget-pkgs/git/trees")
    printf '%s\n' '{"sha":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}'
    ;;
  "POST repos/owner/winget-pkgs/git/commits")
    printf '%s\n' '{"sha":"ffffffffffffffffffffffffffffffffffffffff"}'
    ;;
  "POST repos/owner/winget-pkgs/git/refs")
    printf '%s\n' '{"ref":"refs/heads/new-package-Weita.AitNative-1.2.0"}'
    ;;
  "GET repos/owner/winget-pkgs/contents/"*)
    path=${endpoint%%\?*}
    name=${path##*/}
    encoded=$(base64 <"${TEST_MANIFEST_ROOT}/${name}" | tr -d '\n')
    jq -n --arg content "${encoded}" '{content:$content}'
    ;;
  "POST repos/microsoft/winget-pkgs/pulls")
    printf '%s\n' '{"number":84,"html_url":"https://example.invalid/pull/84","state":"open","merged_at":null}'
    ;;
  *)
    printf 'unexpected fixture request: %s %s\n' "${method}" "${endpoint}" >&2
    exit 64
    ;;
esac
GH
chmod +x "${temporary_root}/bin/gh"

export TEST_ARM64_PAYLOAD=${arm64_payload}
export TEST_X64_PAYLOAD=${x64_payload}
export TEST_GH_LOG=${temporary_root}/gh.log
export TEST_MANIFEST_ROOT=${manifest_root}
export TEST_VERSION=${version}
NODE_OPTIONS="--require=${temporary_root}/fetch.cjs" PATH="${temporary_root}/bin:${PATH}" \
  node "${repo_root}/ci/release_winget_submit.mjs" submit "${common[@]}" \
  >"${temporary_root}/submit.out"
jq -e '.contract == "ait.release.winget-submission/v1" and .status == "submitted" and .pull_request.number == 84' \
  "${temporary_root}/receipt.json" >/dev/null
if rg -q 'merge-upstream' "${temporary_root}/gh.log"; then
  printf 'WinGet submission unexpectedly synchronized the fork\n' >&2
  exit 1
fi
rg -q $'GET\trepos/microsoft/winget-pkgs/compare/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\.\.\.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\t' \
  "${temporary_root}/gh.log"
jq -e '.parents == ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]' \
  < <(awk -F '\t' '$1 == "POST" && $2 == "repos/owner/winget-pkgs/git/commits" {print $3}' "${temporary_root}/gh.log") \
  >/dev/null

printf 'release WinGet submission tests passed\n'
