#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  printf '%s\n' \
    'usage: release_prepublish_stage.sh <endpoint-config> <dossier-root> <protected-evidence> <output-root>' >&2
  exit 64
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
endpoint_config=$1
dossier_root=$2
protected_evidence=$3
output_root=$4

for command in awk docker find jq sort; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required prepublish-stage command is unavailable: %s\n' "${command}" >&2
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
    return 69
  fi
}

require_regular_file() {
  local input=$1
  local label=$2
  [[ -f ${input} && ! -L ${input} ]] || {
    printf '%s must be a regular non-symlink file: %s\n' "${label}" "${input}" >&2
    return 66
  }
}

require_regular_file "${endpoint_config}" 'endpoint configuration'
require_regular_file "${protected_evidence}" 'protected authorization evidence'
[[ ${dossier_root} == /* && -d ${dossier_root} && ! -L ${dossier_root} ]] || {
  printf 'family dossier must be an absolute real directory: %s\n' "${dossier_root}" >&2
  exit 66
}
[[ ${output_root} == /* && ! -e ${output_root} && ! -L ${output_root} ]] || {
  printf 'prepublish stage output must be a new absolute path: %s\n' "${output_root}" >&2
  exit 73
}

bash "${repo_root}/ci/release_endpoint_publication.sh" \
  "${endpoint_config}" "${dossier_root}" "${protected_evidence}" "${output_root}" \
  >/dev/null

release_id=$(jq -er '.release.id' "${endpoint_config}")
release_version=$(jq -er '.release.version' "${endpoint_config}")
release_tag=$(jq -er '.release.tag' "${endpoint_config}")
source_commit=$(jq -er '.release.source_commit' "${endpoint_config}")
endpoint_config_sha256=$(sha256_file "${endpoint_config}")
endpoint_stage_receipt=${output_root}/ait-release.endpoint-publication.json
assets_checksums=${output_root}/assets/SHA256SUMS
require_regular_file "${endpoint_stage_receipt}" 'endpoint stage receipt'
require_regular_file "${assets_checksums}" 'candidate asset checksum inventory'

archive_root=${output_root}/oci-archives
mkdir "${archive_root}"
oci_rows=${output_root}/.oci-rows.jsonl
: >"${oci_rows}"

for component in ait-server ait-runner; do
  context=${output_root}/oci/${component}
  for architecture in amd64 arm64; do
    reference="ait-prepublish/${component}:${release_version}-${architecture}"
    archive_name="${component}-${architecture}.docker.tar"
    archive=${archive_root}/${archive_name}
    docker buildx build \
      --platform "linux/${architecture}" \
      --file "${context}/Dockerfile" \
      --tag "${reference}" \
      --build-arg "AIT_RELEASE_SOURCE_COMMIT=${source_commit}" \
      --build-arg "AIT_RELEASE_VERSION=${release_version}" \
      --provenance=false \
      --sbom=false \
      --output "type=docker,dest=${archive}" \
      "${context}" >/dev/null
    require_regular_file "${archive}" 'frozen OCI Docker archive'
    docker load --input "${archive}" >/dev/null
    image_id=$(docker image inspect --format '{{.Id}}' "${reference}")
    [[ ${image_id} =~ ^sha256:[0-9a-f]{64}$ ]] || {
      printf 'prepublish OCI image ID is invalid: %s\n' "${reference}" >&2
      exit 65
    }
    jq -cn \
      --arg component "${component}" \
      --arg architecture "${architecture}" \
      --arg archive "${archive_name}" \
      --arg sha256 "$(sha256_file "${archive}")" \
      --arg reference "${reference}" \
      --arg image_id "${image_id}" '
        {
          component: $component,
          architecture: $architecture,
          archive: $archive,
          sha256: $sha256,
          reference: $reference,
          image_id: $image_id
        }
      ' >>"${oci_rows}"
  done
done

oci=$(jq -s '
  reduce .[] as $row ({};
    .[$row.component][$row.architecture] = ($row | del(.component, .architecture)))
' "${oci_rows}")
rm "${oci_rows}"

stage_receipt=${output_root}/ait-release.prepublish-stage.json
jq -S -n \
  --arg release_id "${release_id}" \
  --arg version "${release_version}" \
  --arg tag "${release_tag}" \
  --arg source_commit "${source_commit}" \
  --arg endpoint_config_sha256 "${endpoint_config_sha256}" \
  --arg endpoint_stage_receipt_sha256 "$(sha256_file "${endpoint_stage_receipt}")" \
  --arg assets_checksum_sha256 "$(sha256_file "${assets_checksums}")" \
  --argjson oci "${oci}" '
    {
      contract: "ait.release.prepublish.stage/v1",
      status: "frozen_candidate_staged",
      release: {
        id: $release_id,
        version: $version,
        tag: $tag,
        source_commit: $source_commit
      },
      authority: {
        endpoint_config_sha256: $endpoint_config_sha256,
        endpoint_stage_receipt_sha256: $endpoint_stage_receipt_sha256,
        assets_checksum_sha256: $assets_checksum_sha256
      },
      oci: $oci,
      mutation: {
        artifact_rebuild: false,
        component_rebuild: false,
        registry_write: false,
        endpoint_write: false,
        github_release_write: false,
        tag_write: false,
        service_start: false
      },
      next_action: "run_complete_clean_host_matrix_before_publication"
    }
  ' >"${stage_receipt}"

candidate_status=${output_root}/ait-release.prepublish-candidate.json
jq -S -n \
  --arg release_id "${release_id}" \
  --arg version "${release_version}" \
  --arg tag "${release_tag}" \
  --arg source_commit "${source_commit}" \
  --arg stage_receipt_sha256 "$(sha256_file "${stage_receipt}")" \
  --argjson oci "${oci}" '
    {
      contract: "ait.release.prepublish.candidate/v1",
      status: "frozen_candidate_pending_clean_host",
      release: {
        id: $release_id,
        version: $version,
        tag: $tag,
        source_commit: $source_commit
      },
      candidate: {
        stage_receipt_sha256: $stage_receipt_sha256,
        oci: $oci
      },
      public_endpoint_writes: false
    }
  ' >"${candidate_status}"

find "${output_root}" -type f ! -name PREPUBLISH_SHA256SUMS -print |
  LC_ALL=C sort |
  while IFS= read -r file; do
    relative=${file#"${output_root}/"}
    printf '%s  %s\n' "$(sha256_file "${file}")" "${relative}"
  done >"${output_root}/PREPUBLISH_SHA256SUMS"

find "${output_root}" -type f -exec chmod 0644 {} +
printf '%s\n' "${output_root}"
