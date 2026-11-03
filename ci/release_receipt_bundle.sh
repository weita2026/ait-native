#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 8 ]]; then
  printf '%s\n' \
    'usage: release_receipt_bundle.sh <repo-root> <receipt-json> <output-dir> <repo-name> <snapshot> <version> <target-or-portable> <component-artifact-count>' >&2
  exit 64
fi

repo_root=$1
receipt_json=$2
output_dir=$3
expected_repo=$4
expected_snapshot=$5
expected_version=$6
expected_target=$7
expected_component_artifact_count=$8

if [[ ! -d ${repo_root} || -L ${repo_root} ]]; then
  printf 'receipt source root must be a real directory\n' >&2
  exit 66
fi
if [[ ! -f ${receipt_json} || -L ${receipt_json} ]]; then
  printf 'release receipt must be a real file\n' >&2
  exit 66
fi
if [[ -e ${output_dir} || -L ${output_dir} ]]; then
  printf 'receipt bundle output must not already exist\n' >&2
  exit 73
fi
if ! [[ ${expected_component_artifact_count} =~ ^[1-9][0-9]*$ ]]; then
  printf 'expected component artifact count must be positive\n' >&2
  exit 64
fi

if ! jq -e \
  --arg repo_name "${expected_repo}" \
  --arg snapshot "${expected_snapshot}" \
  --arg version "${expected_version}" \
  --arg target "${expected_target}" \
  --argjson artifact_count "${expected_component_artifact_count}" '
    .contract == "ait.release.adapter.receipt/v1" and
    .status == "built" and
    .authority.source == "selected_snapshot" and
    .authority.local_release_authority == "not_activated" and
    .authority.remote_publish_supported == false and
    .check_summary.decision == "pass" and
    .repo_name == $repo_name and
    .snapshot_id == $snapshot and
    .version == $version and
    (.artifacts | type) == "array" and
    all(
      .artifacts[];
      (.path | type) == "string" and
      (.path | length) > 0 and
      .path != "ait-release.receipt.json" and
      .path != "ci-run.evidence.json" and
      (.sha256 | type) == "string" and
      (.sha256 | test("^[0-9a-f]{64}$")) and
      (.size_bytes | type) == "number" and
      (.size_bytes | floor) == .size_bytes and
      .size_bytes >= 0
    ) and
    ([.artifacts[].path] | unique | length) == (.artifacts | length) and
    ([.artifacts[] | select(.role == "component-artifact")] | length)
      == $artifact_count and
    (
      if $target == "portable" then
        .artifact_selection == "portable" and
        ([.artifacts[] | select(.role == "component-artifact") | .target]
          | all(. == null))
      else
        .target == $target and
        ([.artifacts[] | select(.role == "component-artifact") | .target]
          | all(. == $target))
      end
    )
  ' "${receipt_json}" >/dev/null; then
  printf 'release receipt does not satisfy the component bundle contract\n' >&2
  exit 65
fi

artifact_rows=$(mktemp "${TMPDIR:-/tmp}/ait-release-artifacts.XXXXXX")
cleanup() {
  case "${artifact_rows}" in
    "${TMPDIR:-/tmp}"/ait-release-artifacts.*)
      rm -f -- "${artifact_rows}"
      ;;
    *)
      printf 'refusing to remove unexpected artifact inventory: %s\n' \
        "${artifact_rows}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

jq -r '.artifacts[] | [.path, .sha256, (.size_bytes | tostring)] | @tsv' \
  "${receipt_json}" >"${artifact_rows}"
artifact_count=$(wc -l <"${artifact_rows}" | tr -d '[:space:]')
if [[ ${artifact_count} -lt ${expected_component_artifact_count} ]]; then
  printf 'release receipt artifact inventory is incomplete\n' >&2
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

mkdir -p "${output_dir}"
while IFS=$'\t' read -r relative_path expected_sha256 expected_size; do
  if [[ -z ${relative_path} \
    || ${relative_path} == /* \
    || ${relative_path} == *\\* \
    || ! ${relative_path} =~ ^[A-Za-z0-9._+/-]+$ ]]; then
    printf 'release artifact path is not portable: %s\n' "${relative_path}" >&2
    exit 65
  fi
  case "/${relative_path}/" in
    */../* | */./* | *//*)
      printf 'release artifact path is not normalized: %s\n' "${relative_path}" >&2
      exit 65
      ;;
  esac
  source_path="${repo_root}/${relative_path}"
  destination_path="${output_dir}/${relative_path}"
  if [[ ! -f ${source_path} || -L ${source_path} ]]; then
    printf 'release artifact is not a real source file: %s\n' "${relative_path}" >&2
    exit 66
  fi
  actual_size=$(wc -c <"${source_path}" | tr -d '[:space:]')
  actual_sha256=$(sha256_file "${source_path}")
  if [[ ${actual_size} != "${expected_size}" || ${actual_sha256} != "${expected_sha256}" ]]; then
    printf 'release artifact evidence differs from source bytes: %s\n' \
      "${relative_path}" >&2
    exit 65
  fi
  mkdir -p "$(dirname -- "${destination_path}")"
  cp "${source_path}" "${destination_path}"
done <"${artifact_rows}"

cp "${receipt_json}" "${output_dir}/ait-release.receipt.json"
receipt_sha256=$(sha256_file "${output_dir}/ait-release.receipt.json")

jq -n \
  --arg contract 'ait.release.component-ci-evidence/v1' \
  --arg repo_name "${expected_repo}" \
  --arg snapshot "${expected_snapshot}" \
  --arg version "${expected_version}" \
  --arg target "${expected_target}" \
  --arg runner_label "${AIT_RELEASE_RUNNER_LABEL:-unknown}" \
  --arg runner_os "${AIT_RELEASE_RUNNER_OS:-unknown}" \
  --arg runner_arch "${AIT_RELEASE_RUNNER_ARCH:-unknown}" \
  --arg runner_image "${AIT_RELEASE_RUNNER_IMAGE:-unknown}" \
  --arg platform_floor_kind "${AIT_RELEASE_PLATFORM_FLOOR_KIND:-unknown}" \
  --arg platform_floor "${AIT_RELEASE_PLATFORM_FLOOR:-unknown}" \
  --arg bootstrap_git_sha "${AIT_RELEASE_BOOTSTRAP_GIT_SHA:-unknown}" \
  --arg receipt_sha256 "${receipt_sha256}" \
  --argjson component_artifact_count "${expected_component_artifact_count}" \
  --argjson recorded_artifact_count "${artifact_count}" '
  {
    contract: $contract,
    status: "pass",
    repo_name: $repo_name,
    source_snapshot: $snapshot,
    version: $version,
    target: $target,
    runner: {
      label: $runner_label,
      os: $runner_os,
      architecture: $runner_arch,
      image: $runner_image
    },
    platform_floor: {
      kind: $platform_floor_kind,
      value: $platform_floor
    },
    bootstrap_git_sha: $bootstrap_git_sha,
    receipt_sha256: $receipt_sha256,
    component_artifact_count: $component_artifact_count,
    recorded_artifact_count: $recorded_artifact_count,
    source_authority: "selected_snapshot_store",
    registry_publish: false,
    public_publish: false
  }
' >"${output_dir}/ci-run.evidence.json"

printf '%s\n' "${output_dir}"
