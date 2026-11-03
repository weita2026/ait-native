#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  printf 'usage: release_artifact_cache.sh <owner/repository> <run-id> <artifact-prefix> <absolute-output-dir>\n' >&2
  exit 64
fi
repository=$1
run_id=$2
artifact_prefix=$3
output=$4
[[ ${repository} =~ ^[^/]+/[^/]+$ && ${run_id} =~ ^[1-9][0-9]*$ &&
  -n ${artifact_prefix} && ${output} == /* ]] || {
  printf 'release artifact-cache input is invalid\n' >&2
  exit 64
}
for command in gh jq unzip; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done
[[ ! -e ${output} && ! -L ${output} ]] || {
  printf 'release artifact-cache output already exists\n' >&2
  exit 73
}
parent=$(dirname -- "${output}")
[[ -d ${parent} && ! -L ${parent} ]] || {
  printf 'release artifact-cache output parent must be a real directory\n' >&2
  exit 66
}
temporary=$(mktemp -d "${parent}/.$(basename -- "${output}").partial.XXXXXX")
cleanup() {
  if [[ -n ${temporary:-} && -d ${temporary} ]]; then
    case "${temporary}" in
      "${parent}"/.*.partial.*) rm -rf -- "${temporary}" ;;
      *) printf 'refusing unexpected artifact-cache cleanup path: %s\n' "${temporary}" >&2 ;;
    esac
  fi
}
trap cleanup EXIT HUP INT TERM

gh api "repos/${repository}/actions/runs/${run_id}" >"${temporary}/run.json"
jq -e --argjson run_id "${run_id}" '
  .id == $run_id and .status == "completed" and .conclusion == "success" and
  (.head_sha | test("^[0-9a-f]{40}$"))
' "${temporary}/run.json" >/dev/null || {
  printf 'release artifact-cache workflow run is not successful\n' >&2
  exit 65
}
gh api --paginate "repos/${repository}/actions/runs/${run_id}/artifacts?per_page=100" |
  jq -s '{artifacts: [.[].artifacts[]]}' >"${temporary}/artifacts.json"
jq -e --arg prefix "${artifact_prefix}" '
  [.artifacts[] | select(.expired == false and (.name | startswith($prefix)))] as $matches |
  ($matches | map(.name) | unique) as $names |
  if ($names | length) == 1 then ($matches | max_by(.id))
  else error("artifact prefix is unavailable or ambiguous") end
' "${temporary}/artifacts.json" >"${temporary}/artifact.json"
artifact_name=$(jq -er '.name' "${temporary}/artifact.json")
artifact_id=$(jq -er '.id' "${temporary}/artifact.json")
mkdir "${temporary}/payload"
gh api -H 'Accept: application/vnd.github+json' \
  "repos/${repository}/actions/artifacts/${artifact_id}/zip" \
  >"${temporary}/artifact.zip"
unzip -q "${temporary}/artifact.zip" -d "${temporary}/payload"
jq -S -n --arg repository "${repository}" --argjson run_id "${run_id}" \
  --arg artifact_name "${artifact_name}" --argjson artifact_id "${artifact_id}" '
  {
    contract: "ait.release.artifact-cache/v1",
    repository: $repository,
    workflow_run_id: $run_id,
    artifact_name: $artifact_name,
    artifact_id: $artifact_id,
    status: "complete"
  }
' >"${temporary}/cache.json"
mv "${temporary}" "${output}"
temporary=
printf '%s\n' "${output}"
