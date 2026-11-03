#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-artifact-cache.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-artifact-cache.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
mkdir "${temporary_root}/bin" "${temporary_root}/records"

cat >"${temporary_root}/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_ARTIFACT_CACHE_FIXTURE:?}
if [[ $1 == api && ${*: -1} == */artifacts\?per_page=100 ]]; then
  printf '{"artifacts":[{"id":21,"name":"ait-family-dossier-REL-FAM-ABCDEF0123456789","expired":false},{"id":22,"name":"ait-family-dossier-REL-FAM-ABCDEF0123456789","expired":false}]}\n'
elif [[ $1 == api && ${*: -1} == */actions/artifacts/22/zip ]]; then
  archive=${state}/artifact.zip
  work=${state}/payload
  mkdir -p "${work}"
  printf 'frozen bytes\n' >"${work}/dossier.bin"
  (cd "${work}" && zip -q "${archive}" dossier.bin)
  cat "${archive}"
  count=0
  [[ ! -f ${state}/download-count ]] || count=$(cat "${state}/download-count")
  printf '%s\n' "$((count + 1))" >"${state}/download-count"
elif [[ $1 == api ]]; then
  printf '{"id":1234,"status":"completed","conclusion":"success","head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}\n'
else
  exit 64
fi
GH
chmod +x "${temporary_root}/bin/gh"
export AIT_ARTIFACT_CACHE_FIXTURE=${temporary_root}/fixture
mkdir "${AIT_ARTIFACT_CACHE_FIXTURE}"

PATH="${temporary_root}/bin:${PATH}" "${repo_root}/ci/release_artifact_cache.sh" \
  owner/repository 1234 ait-family-dossier- \
  "${temporary_root}/records/dossier" >/dev/null
jq -e '.status == "complete" and .workflow_run_id == 1234 and .artifact_id == 22' \
  "${temporary_root}/records/dossier/cache.json" >/dev/null
test -f "${temporary_root}/records/dossier/payload/dossier.bin"
test "$(cat "${AIT_ARTIFACT_CACHE_FIXTURE}/download-count")" = 1
if PATH="${temporary_root}/bin:${PATH}" "${repo_root}/ci/release_artifact_cache.sh" \
  owner/repository 1234 ait-family-dossier- \
  "${temporary_root}/records/dossier" >/dev/null 2>"${temporary_root}/repeat.stderr"; then
  printf 'artifact cache overwrote an immutable output\n' >&2
  exit 65
fi
grep -F 'release artifact-cache output already exists' "${temporary_root}/repeat.stderr" >/dev/null
test "$(cat "${AIT_ARTIFACT_CACHE_FIXTURE}/download-count")" = 1

printf 'release artifact-cache tests passed\n'
