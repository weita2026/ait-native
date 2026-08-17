#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 9 ]]; then
  printf '%s\n' \
    'usage: release_source_cache.sh <ait-bin> <repo-name> <repository-index> <namespace> <snapshot> <version> <license> <bootstrap-ci-manifest> <destination>' >&2
  exit 64
fi

ait_bin=$1
repo_name=$2
repository_index=$3
namespace=$4
source_snapshot=$5
version=$6
license=$7
bootstrap_ci_manifest=$8
destination=$9
server_url=${AIT_RELEASE_SERVER_URL:?AIT_RELEASE_SERVER_URL is required}
source_line=${AIT_RELEASE_SOURCE_LINE:-main}
bootstrap_line=${AIT_RELEASE_BOOTSTRAP_LINE:-release-bootstrap}
evidence_path=${AIT_RELEASE_SOURCE_EVIDENCE_PATH:-${destination}.evidence.json}
reserved_materialization=.ait-release-bootstrap-source

require_match() {
  local value=$1
  local expression=$2
  local label=$3
  if ! [[ ${value} =~ ${expression} ]]; then
    printf 'invalid %s\n' "${label}" >&2
    exit 64
  fi
}

require_match "${repo_name}" '^[a-z0-9-]+$' 'repository name'
require_match "${repository_index}" '^[0-9]+$' 'Repository index'
require_match "${namespace}" '^[A-Za-z0-9_-]{1,2}$' 'Repository namespace'
require_match "${source_snapshot}" '^SNP-[0-9A-F]{12}$' 'source Snapshot'
require_match "${version}" '^[0-9A-Za-z.+-]+$' 'component version'
require_match "${license}" '^[0-9A-Za-z.+-]+$' 'component license'
require_match "${source_line}" '^[A-Za-z0-9._/-]+$' 'source Line'
require_match "${bootstrap_line}" '^[A-Za-z0-9._/-]+$' 'bootstrap Line'

if [[ ${source_line} == "${bootstrap_line}" ]]; then
  printf 'source Line and bootstrap Line must differ\n' >&2
  exit 64
fi
if [[ ! -x ${ait_bin} || -L ${ait_bin} ]]; then
  printf 'ait bootstrap must be a real executable file\n' >&2
  exit 66
fi
if [[ ! -f ${bootstrap_ci_manifest} || -L ${bootstrap_ci_manifest} ]]; then
  printf 'bootstrap Patchset CI manifest must be a real file\n' >&2
  exit 66
fi
case "${destination}" in
  /*) ;;
  *)
    printf 'source-cache destination must be absolute\n' >&2
    exit 64
    ;;
esac
if [[ -e ${destination} || -L ${destination} ]]; then
  printf 'source-cache destination must not already exist\n' >&2
  exit 73
fi
if [[ ${destination##*/} != "${repo_name}" ]]; then
  printf 'source-cache destination basename must equal the repository name\n' >&2
  exit 64
fi
if [[ -e ${evidence_path} || -L ${evidence_path} ]]; then
  printf 'source-cache evidence path must not already exist\n' >&2
  exit 73
fi

mkdir -p "${destination}/ci"
cp "${bootstrap_ci_manifest}" "${destination}/ci/patch_ci.json"
{
  printf '[[external]]\n'
  printf 'name = "release-source"\n'
  printf 'repo_name = "%s"\n' "${repo_name}"
  printf 'repository_index = %s\n' "${repository_index}"
  printf 'remote = "origin"\n'
  printf 'line = "%s"\n' "${source_line}"
  printf 'snapshot = "%s"\n' "${source_snapshot}"
  printf 'materialize_to = "%s"\n' "${reserved_materialization}"
  printf 'license = "%s"\n' "${license}"
  printf 'version = "%s"\n' "${version}"
} >"${destination}/ait-external.toml"

evidence_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-source-evidence.XXXXXX")
cleanup() {
  case "${evidence_root}" in
    "${TMPDIR:-/tmp}"/ait-release-source-evidence.*)
      rm -rf -- "${evidence_root}"
      ;;
    *)
      printf 'refusing to remove unexpected evidence path: %s\n' "${evidence_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

(
  cd "${destination}"
  "${ait_bin}" init --json >"${evidence_root}/init.json"
  if [[ ${bootstrap_line} != main ]]; then
    "${ait_bin}" line rename main "${bootstrap_line}" \
      --json >"${evidence_root}/bootstrap-line.json"
  fi
  "${ait_bin}" snapshot create \
    --message "Release source-cache bootstrap policy" \
    --json >"${evidence_root}/bootstrap-snapshot.json"
  "${ait_bin}" config set \
    --repository-index "${repository_index}" \
    --id-namespace-prefix "${namespace}" \
    --json >"${evidence_root}/config.json"
  "${ait_bin}" remote add origin "${server_url}" \
    --default \
    --json >"${evidence_root}/remote.json"
  "${ait_bin}" external update release-source \
    --to "${source_snapshot}" \
    --no-recursive \
    --json >"${evidence_root}/hydrate.json"
  "${ait_bin}" line create "${source_line}" \
    --from-snapshot "${source_snapshot}" \
    --json >"${evidence_root}/line.json"
  "${ait_bin}" line switch "${source_line}" \
    --restore \
    --force \
    --json >"${evidence_root}/switch.json"
)

generated_root="${destination}/${reserved_materialization}"
if [[ ! -f ${generated_root}/.ait-external-marker.json \
  || -L ${generated_root} \
  || -L ${generated_root}/.ait-external-marker.json ]]; then
  printf 'generated bootstrap source is missing its AIT materialization marker\n' >&2
  exit 65
fi
rm -rf -- "${generated_root}"

workspace_external_materialized=false
if [[ -f ${destination}/ait-external.lock ]]; then
  (
    cd "${destination}"
    "${ait_bin}" external update \
      --locked \
      --json >"${evidence_root}/target-external.json"
  )
  workspace_external_materialized=true
fi

config_path="${destination}/.ait/config.json"
sanitized_config="${destination}/.ait/config.release-cache.json"
jq '
  del(
    .default_remote,
    .remotes,
    .task_worktree,
    .agent_runtime,
    .web_inbox_defaults
  )
' "${config_path}" >"${sanitized_config}"
mv "${sanitized_config}" "${config_path}"

(
  cd "${destination}"
  "${ait_bin}" status --json >"${evidence_root}/status.json"
  "${ait_bin}" snapshot show "${source_snapshot}" --json \
    >"${evidence_root}/snapshot.json"
)

jq -e \
  --arg repo_name "${repo_name}" \
  --arg source_line "${source_line}" \
  --arg source_snapshot "${source_snapshot}" '
    .repo_name == $repo_name and
    .current_line == $source_line and
    .head_snapshot_id == $source_snapshot and
    .workspace_status == "clean" and
    .workspace_changed_count == 0 and
    .remote_count == 0
  ' "${evidence_root}/status.json" >/dev/null
jq -e --arg source_snapshot "${source_snapshot}" '
  .snapshot_id == $source_snapshot and
  (.parent_snapshot_ids | length) == 0
' "${evidence_root}/snapshot.json" >/dev/null

if grep -RIlF -- "${server_url}" "${destination}/.ait" \
  >"${evidence_root}/embedded-remote-paths.txt"; then
  printf 'source cache still contains remote coordinates\n' >&2
  exit 65
fi

sha256_file() {
  local path=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${path}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${path}" | awk '{print $1}'
  else
    printf 'no SHA-256 utility is available\n' >&2
    return 69
  fi
}

adapter_sha256=$(sha256_file "${destination}/ait-release.json")
source_manifest_hash=$(jq -er '.manifest_hash | select(test("^[0-9a-f]{64}$"))' \
  "${evidence_root}/snapshot.json")
source_snapshot_created_at=$(jq -er \
  '.created_at | select(type == "string" and test("^(0|[1-9][0-9]*)$"))' \
  "${evidence_root}/snapshot.json")
family_manifest_sha256=null
if [[ -f ${destination}/ait-release-family.json ]]; then
  family_manifest_sha256=$(sha256_file \
    "${destination}/ait-release-family.json")
fi

jq -n \
  --arg contract 'ait.release.source-cache/v1' \
  --arg repo_name "${repo_name}" \
  --argjson repository_index "${repository_index}" \
  --arg namespace "${namespace}" \
  --arg source_snapshot "${source_snapshot}" \
  --arg source_manifest_hash "${source_manifest_hash}" \
  --arg source_snapshot_created_at "${source_snapshot_created_at}" \
  --arg source_line "${source_line}" \
  --arg version "${version}" \
  --arg license "${license}" \
  --arg adapter_sha256 "${adapter_sha256}" \
  --arg family_manifest_sha256 "${family_manifest_sha256}" \
  --argjson workspace_external_materialized \
  "${workspace_external_materialized}" '
  {
    contract: $contract,
    status: "ready",
    repo_name: $repo_name,
    repository_index: $repository_index,
    namespace: $namespace,
    source_snapshot: $source_snapshot,
    source_manifest_hash: $source_manifest_hash,
    source_snapshot_created_at: $source_snapshot_created_at,
    source_line: $source_line,
    version: $version,
    license: $license,
    source_authority: "ait_remote_snapshot_boundary",
    local_selection_authority: "selected_snapshot_store",
    shallow_history_boundary: true,
    workspace_clean: true,
    workspace_external_materialized: $workspace_external_materialized,
    remote_coordinates_embedded: false,
    adapter_manifest_sha256: $adapter_sha256,
    family_manifest_sha256: (
      if $family_manifest_sha256 == "null"
      then null
      else $family_manifest_sha256
      end
    ),
    public_publish: false
  }
' >"${evidence_path}"

printf '%s\n' "${evidence_path}"
