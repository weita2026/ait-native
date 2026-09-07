#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  printf 'usage: release_export_source.sh <prior|final> <canonical-core-root> <family-manifest> <coordinator-json> <absolute-output-dir>\n' >&2
  exit 64
fi
kind=$1
core_root=$2
family=$3
coordinator=$4
output=$5
[[ ${kind} == prior || ${kind} == final ]]
for input in "${core_root}" "${family}" "${coordinator}" "${output}"; do
  [[ ${input} == /* ]] || {
    printf 'release source-export paths must be absolute\n' >&2
    exit 64
  }
done
[[ -d ${core_root} && ! -L ${core_root} && -f ${family} && ! -L ${family} &&
  -f ${coordinator} && ! -L ${coordinator} ]] || {
  printf 'release source-export input is unavailable\n' >&2
  exit 66
}
[[ ! -e ${output} && ! -L ${output} ]] || {
  printf 'release source-export output already exists\n' >&2
  exit 73
}
parent=$(dirname -- "${output}")
[[ -d ${parent} && ! -L ${parent} ]] || {
  printf 'release source-export output parent must be a real directory\n' >&2
  exit 66
}
for command in diff jq; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done
temporary=$(mktemp -d "${parent}/.$(basename -- "${output}").partial.XXXXXX")
cleanup() {
  if [[ -n ${temporary:-} && -d ${temporary} ]]; then
    case "${temporary}" in
      "${parent}"/.*.partial.*) rm -rf -- "${temporary}" ;;
      *) printf 'refusing unexpected source-export cleanup path: %s\n' "${temporary}" >&2 ;;
    esac
  fi
}
trap cleanup EXIT HUP INT TERM

IFS=$'\t' read -r snapshot manifest_hash created_at < <(
  jq -er '[.snapshot_id, .manifest_hash, .created_at] | @tsv' "${coordinator}"
)
[[ ${snapshot} =~ ^SNP-[0-9A-F]{12}$ && ${manifest_hash} =~ ^[0-9a-f]{64}$ &&
  ${created_at} =~ ^(0|[1-9][0-9]*)$ ]] || {
  printf 'release source-export coordinator metadata is invalid\n' >&2
  exit 65
}

if [[ ${kind} == prior ]]; then
  "${core_root}/ci/release_authority_preflight.sh" "${core_root}" \
    "${temporary}/authority.json" "${family}" >"${temporary}/authority.log" 2>&1
  "${core_root}/ci/release_source_bundles.sh" "${core_root}" \
    "${temporary}/source-bundles" "${family}" >"${temporary}/source-bundles.log" 2>&1
else
  cmp "${core_root}/ait-release-family.json" "${family}" >/dev/null || {
    printf 'final source-export family must equal canonical authority\n' >&2
    exit 65
  }
  "${core_root}/ci/release_authority_preflight.sh" "${core_root}" \
    "${temporary}/authority.json" >"${temporary}/authority.log" 2>&1
  "${core_root}/ci/release_source_bundles.sh" "${core_root}" \
    "${temporary}/source-bundles" >"${temporary}/source-bundles.log" 2>&1
fi
for label in a b; do
  AIT_RELEASE_COORDINATOR_SNAPSHOT=${snapshot} \
  AIT_RELEASE_COORDINATOR_MANIFEST_HASH=${manifest_hash} \
  AIT_RELEASE_COORDINATOR_CREATED_AT=${created_at} \
    "${core_root}/ci/release_monorepo_export.sh" \
      "${family}" "${temporary}/source-bundles" \
      "${temporary}/export-${label}" "${temporary}/export-${label}.evidence.json" \
      >"${temporary}/export-${label}.log" 2>&1
done
diff -qr "${temporary}/export-a" "${temporary}/export-b" >"${temporary}/repeat.diff"
(cd "${temporary}/export-a/ait-core" && ./ci/release_clean_host_test.sh) \
  >"${temporary}/clean-host.log" 2>&1
jq -S -n --arg kind "${kind}" --arg snapshot "${snapshot}" \
  --arg manifest_hash "${manifest_hash}" --arg created_at "${created_at}" \
  --slurpfile family "${family}" '
  {
    contract: "ait.release.source-export/v1",
    status: "pass",
    kind: $kind,
    version: $family[0].family.version,
    tag: $family[0].family.tag,
    coordinator: {
      snapshot_id: $snapshot,
      manifest_hash: $manifest_hash,
      created_at: $created_at
    },
    repeated_export_equal: true,
    exported_clean_host_contract: "pass"
  }
' >"${temporary}/receipt.json"
mv "${temporary}" "${output}"
temporary=
printf '%s\n' "${output}"
