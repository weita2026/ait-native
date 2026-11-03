#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf 'usage: release_export_source.sh <ait-native-root> <prior-version> <absolute-output-dir>\n' >&2
  exit 64
fi
native_root=$1
prior_version=$2
output=$3
for input in "${native_root}" "${output}"; do
  [[ ${input} == /* ]] || {
    printf 'release source-export paths must be absolute\n' >&2
    exit 64
  }
done
[[ -d ${native_root} && ! -L ${native_root} ]] || {
  printf 'release source-export root is unavailable\n' >&2
  exit 66
}
[[ ${prior_version} =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc\.[1-9][0-9]*)?$ ]] || {
  printf 'release source-export prior version is invalid\n' >&2
  exit 64
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
for command in diff jq node; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    printf 'no SHA-256 utility is available\n' >&2
    exit 69
  fi
}
template=${native_root}/ait-release-family.json
[[ -f ${template} && ! -L ${template} ]] || {
  printf 'canonical family template is unavailable\n' >&2
  exit 66
}
version=$(jq -er '.family.version' "${template}")
[[ ${prior_version} != "${version}" ]] || {
  printf 'release source-export prior version must differ from the release version\n' >&2
  exit 65
}
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

logged() {
  local step=$1 log=$2
  shift 2
  if ! "$@" >"${log}" 2>&1; then
    printf 'release source-export step failed: %s\n' "${step}" >&2
    cat "${log}" >&2
    exit 65
  fi
}

logged authority-preflight "${temporary}/authority.log" \
  "${native_root}/ci/release_authority_preflight.sh" "${native_root}" \
  "${temporary}/authority.json"
jq '.binding' "${temporary}/authority.json" >"${temporary}/binding.json"
logged bind-final "${temporary}/bind-final.log" \
  node "${native_root}/ci/release_family_bind.mjs" --template "${template}" \
  --binding "${temporary}/binding.json" --output "${temporary}/final-family.json"
logged bind-prior "${temporary}/bind-prior.log" \
  node "${native_root}/ci/release_family_bind.mjs" --template "${template}" \
  --binding "${temporary}/binding.json" --output "${temporary}/prior-family.json" \
  --version "${prior_version}"
logged source-bundles "${temporary}/source-bundles.log" \
  "${native_root}/ci/release_source_bundles.sh" "${native_root}" \
  "${temporary}/source-bundles"

core_evidence=${temporary}/source-bundles/ait-release-source-ait-core/source-cache.evidence.json
[[ -f ${core_evidence} && ! -L ${core_evidence} ]] || {
  printf 'release source-export Core bundle evidence is unavailable\n' >&2
  exit 65
}
IFS=$'\t' read -r snapshot manifest_hash created_at < <(
  jq -er '[.source_snapshot, .source_manifest_hash, .source_snapshot_created_at] | @tsv' \
    "${core_evidence}"
)
[[ ${snapshot} == $(jq -er '.components["ait-core"]' "${temporary}/binding.json") &&
  ${snapshot} =~ ^SNP-[0-9A-F]{12}$ && ${manifest_hash} =~ ^[0-9a-f]{64}$ &&
  ${created_at} =~ ^(0|[1-9][0-9]*)$ ]] || {
  printf 'release source-export Core Snapshot metadata is invalid\n' >&2
  exit 65
}

export_kind() {
  local kind=$1 family=$2
  mkdir -p "${temporary}/${kind}"
  local label
  local -a overlay=()
  if [[ ${kind} == prior ]]; then
    overlay=(
      "AIT_RELEASE_VERSION_OVERLAY_FROM=${version}"
      "AIT_RELEASE_VERSION_OVERLAY_TO=${prior_version}"
    )
  fi
  for label in a b; do
    logged "export-${kind}-${label}" "${temporary}/${kind}/export-${label}.log" \
      env ${overlay[@]+"${overlay[@]}"} \
        "AIT_RELEASE_COORDINATOR_SNAPSHOT=${snapshot}" \
        "AIT_RELEASE_COORDINATOR_MANIFEST_HASH=${manifest_hash}" \
        "AIT_RELEASE_COORDINATOR_CREATED_AT=${created_at}" \
        "${native_root}/ci/release_monorepo_export.sh" \
          "${family}" "${temporary}/source-bundles" \
          "${temporary}/${kind}/export-${label}" \
          "${temporary}/${kind}/export-${label}.evidence.json"
  done
  logged "repeat-${kind}" "${temporary}/${kind}/repeat.diff" \
    diff -qr "${temporary}/${kind}/export-a" "${temporary}/${kind}/export-b"
  (cd "${temporary}/${kind}/export-a" && ./ci/release_clean_host_test.sh) \
    >"${temporary}/${kind}/clean-host.log" 2>&1 || {
    printf 'release source-export step failed: clean-host-%s\n' "${kind}" >&2
    cat "${temporary}/${kind}/clean-host.log" >&2
    exit 65
  }
}

export_kind final "${temporary}/final-family.json"
export_kind prior "${temporary}/prior-family.json"

jq -S -n \
  --arg version "${version}" \
  --arg prior_version "${prior_version}" \
  --arg snapshot "${snapshot}" \
  --arg manifest_hash "${manifest_hash}" \
  --arg created_at "${created_at}" \
  --arg binding_sha256 "$(sha256_file "${temporary}/binding.json")" \
  --arg final_family_sha256 "$(sha256_file "${temporary}/final-family.json")" \
  --arg prior_family_sha256 "$(sha256_file "${temporary}/prior-family.json")" \
  --argjson binding "$(cat "${temporary}/binding.json")" '
  {
    contract: "ait.release.source-export/v2",
    status: "pass",
    version: $version,
    tag: ("v" + $version),
    prior_version: $prior_version,
    coordinator: {
      snapshot_id: $snapshot,
      manifest_hash: $manifest_hash,
      created_at: $created_at
    },
    binding: $binding,
    binding_sha256: $binding_sha256,
    exports: {
      final: {
        family_manifest_sha256: $final_family_sha256,
        version_overlay: null,
        repeated_export_equal: true,
        exported_clean_host_contract: "pass"
      },
      prior: {
        family_manifest_sha256: $prior_family_sha256,
        version_overlay: {from: $version, to: $prior_version},
        repeated_export_equal: true,
        exported_clean_host_contract: "pass"
      }
    }
  }
' >"${temporary}/receipt.json"
mv "${temporary}" "${output}"
temporary=
printf '%s\n' "${output}"
