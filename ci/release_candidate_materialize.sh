#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' 'usage: release_candidate_materialize.sh {materialize|probe} <version> <candidate-cache-root> <absolute-output-dir>' >&2
  exit 64
}
mode=${1:-}
version=${2:-}
cache_root=${3:-}
output=${4:-}
[[ (${mode} == materialize || ${mode} == probe) && ${version} =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && \
  ${cache_root} == /* && ${output} == /* && $# -eq 4 ]] || usage
for command_name in find awk uname cmp jq; do
  command -v "${command_name}" >/dev/null 2>&1 || exit 69
done
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else printf 'no SHA-256 utility is available\n' >&2; exit 69
  fi
}

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) triple=aarch64-apple-darwin ;;
  Darwin:x86_64) triple=x86_64-apple-darwin ;;
  Linux:aarch64 | Linux:arm64) triple=aarch64-unknown-linux-gnu ;;
  Linux:x86_64 | Linux:amd64) triple=x86_64-unknown-linux-gnu ;;
  *) printf 'release candidate host is unsupported\n' >&2; exit 65 ;;
esac

find_one() {
  local name=$1
  local matches=()
  while IFS= read -r candidate_path; do matches+=("${candidate_path}"); done < <(
    find "${cache_root}" -type f -name "${name}" -print | LC_ALL=C sort
  )
  [[ ${#matches[@]} -eq 1 ]] || {
    printf 'candidate file is missing or ambiguous: %s\n' "${name}" >&2
    exit 66
  }
  printf '%s\n' "${matches[0]}"
}

ait_source=$(find_one "ait-${version}-${triple}")
server_source=$(find_one "ait-server-${version}-${triple}")
ait_sha=$(sha256_file "${ait_source}")
server_sha=$(sha256_file "${server_source}")

valid_output() {
  [[ -d ${output} && ! -L ${output} && -x ${output}/ait && -x ${output}/ait-server && \
    -f ${output}/receipt.json && ! -L ${output}/receipt.json ]] || return 1
  cmp "${ait_source}" "${output}/ait" >/dev/null && cmp "${server_source}" "${output}/ait-server" >/dev/null &&
    jq -e --arg version "${version}" --arg triple "${triple}" --arg ait_sha "${ait_sha}" --arg server_sha "${server_sha}" '
      .contract == "ait.release.candidate-materialization/v1" and .status == "complete" and
      .version == $version and .target_triple == $triple and
      .executables.ait.sha256 == $ait_sha and .executables.server.sha256 == $server_sha
    ' "${output}/receipt.json" >/dev/null
}

if [[ ${mode} == probe ]]; then
  valid_output
  exit
fi
[[ ! -e ${output} && ! -L ${output} ]] || {
  printf 'candidate materialization output already exists\n' >&2
  exit 73
}
temporary=$(mktemp -d "$(dirname -- "${output}")/.$(basename -- "${output}").partial.XXXXXX")
cleanup() {
  [[ ! -d ${temporary:-} ]] || rm -rf -- "${temporary}"
}
trap cleanup EXIT HUP INT TERM
cp "${ait_source}" "${temporary}/ait"
cp "${server_source}" "${temporary}/ait-server"
chmod 700 "${temporary}/ait" "${temporary}/ait-server"
jq -S -n --arg version "${version}" --arg triple "${triple}" \
  --arg ait_sha "${ait_sha}" --arg server_sha "${server_sha}" '
  {contract:"ait.release.candidate-materialization/v1",status:"complete",version:$version,
    target_triple:$triple,executables:{ait:{path:"ait",sha256:$ait_sha},server:{path:"ait-server",sha256:$server_sha}}}
' >"${temporary}/receipt.json"
mv "${temporary}" "${output}"
temporary=''
valid_output
printf '%s\n' "${output}"
