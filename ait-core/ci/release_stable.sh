#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' 'usage: release_stable.sh {plan|prepare|start} --request <absolute-request.json>' >&2
  printf '%s\n' '       release_stable.sh {run|status} --input <absolute-accepted-input.json>' >&2
  exit 64
}

mode=${1:-}
release_path="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"
export PATH="${release_path}"
case ${mode} in
  plan|prepare|start)
    shift
    [[ ${1:-} == --request && ${2:-} == /* && $# -eq 2 ]] || usage
    request=$2
    repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
    prepare_mode=run
    [[ ${mode} == plan ]] && prepare_mode=plan
    if [[ ${mode} == start ]]; then
      node "${repo_root}/ci/release_stable_prepare.mjs" run --request "${request}"
      records_root=$(jq -er '.records_root | select(type == "string" and startswith("/"))' "${request}")
      exec "${repo_root}/ci/release_stable.sh" run --input "${records_root}/accepted-input.json"
    fi
    exec node "${repo_root}/ci/release_stable_prepare.mjs" "${prepare_mode}" --request "${request}"
    ;;
  run|status) ;;
  *) usage ;;
esac
shift
[[ ${1:-} == --input && ${2:-} == /* && $# -eq 2 ]] || usage
input=$2
[[ -f ${input} && ! -L ${input} ]] || {
  printf 'stable release input must be a regular file\n' >&2
  exit 66
}
for command_name in jq node; do
  command -v "${command_name}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command_name}" >&2
    exit 69
  }
done
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else printf 'no SHA-256 utility is available\n' >&2; exit 69
  fi
}
repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
records_root=$(jq -er '.records_root | select(type == "string" and startswith("/"))' "${input}")
mkdir -p -- "${records_root}"
records_root=$(CDPATH='' cd -- "${records_root}" && pwd)
spec=${records_root}/release-spec.json
if [[ ! -e ${spec} ]]; then
  node "${repo_root}/ci/release_stable_spec.mjs" --input "${input}" --output "${spec}" >/dev/null
else
  expected=$(sha256_file "${input}")
  jq -e --arg input_path "${input}" --arg expected "${expected}" '
    .values.accepted_input_path == $input_path and .values.accepted_input_sha256 == $expected
  ' "${spec}" >/dev/null || {
    printf 'accepted release input differs from the immutable compiled spec\n' >&2
    exit 65
  }
fi
exec "${repo_root}/ci/release_conductor.sh" "${mode}" \
  --spec "${spec}" --recipe "${repo_root}/release/stable-conductor.recipe.json"
