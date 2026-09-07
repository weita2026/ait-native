#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'usage: release_conductor.sh {run|status} --spec <absolute-json> --recipe <absolute-json>\n' >&2
  exit 64
}

mode=${1:-}
[[ ${mode} == run || ${mode} == status ]] || usage
shift
spec=
recipe=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --spec) [[ $# -ge 2 && -z ${spec} ]] || usage; spec=$2; shift 2 ;;
    --recipe) [[ $# -ge 2 && -z ${recipe} ]] || usage; recipe=$2; shift 2 ;;
    *) usage ;;
  esac
done
[[ ${spec} == /* && ${recipe} == /* ]] || usage
for command in jq node shasum; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done
[[ -f ${spec} && ! -L ${spec} && -f ${recipe} && ! -L ${recipe} ]] || {
  printf 'release spec and recipe must be regular files\n' >&2
  exit 66
}

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
records_root=$(jq -er '.records_root | select(type == "string" and startswith("/"))' "${spec}")
records_root=$(mkdir -p "${records_root}" && CDPATH='' cd -- "${records_root}" && pwd)
plan=${records_root}/conductor-plan.json
state=${records_root}/conductor-state.json

if [[ ! -e ${plan} ]]; then
  node "${repo_root}/ci/release_conductor_init.mjs" \
    --spec "${spec}" --recipe "${recipe}" --plan "${plan}" >/dev/null
else
  [[ -f ${plan} && ! -L ${plan} ]] || {
    printf 'release conductor plan must be a regular file\n' >&2
    exit 66
  }
  spec_sha=$(shasum -a 256 "${spec}" | awk '{print $1}')
  recipe_sha=$(shasum -a 256 "${recipe}" | awk '{print $1}')
  jq -e --arg spec_sha "${spec_sha}" --arg recipe_sha "${recipe_sha}" '
    .source.spec_sha256 == $spec_sha and .source.recipe_sha256 == $recipe_sha
  ' "${plan}" >/dev/null || {
    printf 'release spec or recipe differs from the immutable compiled plan\n' >&2
    exit 65
  }
fi

exec node "${repo_root}/ci/release_conductor.mjs" "${mode}" \
  --plan "${plan}" --state "${state}"
