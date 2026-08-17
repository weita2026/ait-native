#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 || $1 != publish ]]; then
  printf '%s\n' \
    'usage: release_prepublish_oci.sh publish <endpoint-config> <candidate-root> <ait-server|ait-runner> <existing-digest|absent>' >&2
  exit 64
fi

endpoint_config=$2
candidate_root=$3
component=$4
existing=$5

case "${component}" in
  ait-server | ait-runner) ;;
  *) printf 'unsupported prepublish OCI component: %s\n' "${component}" >&2; exit 64 ;;
esac

for command in awk docker jq; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required prepublish OCI command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

[[ -f ${endpoint_config} && ! -L ${endpoint_config} ]] || exit 66
[[ ${candidate_root} == /* && -d ${candidate_root} && ! -L ${candidate_root} ]] || exit 66
status=${candidate_root}/ait-release.prepublish-candidate.json
[[ -f ${status} && ! -L ${status} ]] || exit 66

image=$(jq -er --arg component "${component}" '
  .endpoints.oci.images[] | select(endswith("/" + $component))
' "${endpoint_config}")
immutable_tag=$(jq -er '.endpoints.oci.immutable_tag' "${endpoint_config}")
moving_tag=$(jq -er '.endpoints.oci.moving_tag' "${endpoint_config}")

platform_config_digest() {
  local reference=$1
  docker buildx imagetools inspect --raw "${reference}" | jq -er \
    '.config.digest | select(test("^sha256:[0-9a-f]{64}$"))'
}

verify_index() {
  local digest=$1
  local raw architecture manifest_digest expected_config actual_config
  raw=$(docker buildx imagetools inspect --raw "${image}@${digest}")
  for architecture in amd64 arm64; do
    manifest_digest=$(jq -er --arg architecture "${architecture}" '
      [.manifests[] |
        select(.platform.os == "linux" and .platform.architecture == $architecture) |
        .digest] |
      if length == 1 then .[0] else error("platform manifest is not unique") end
    ' <<<"${raw}")
    expected_config=$(jq -er \
      --arg component "${component}" --arg architecture "${architecture}" \
      '.candidate.oci[$component][$architecture].image_id' "${status}")
    actual_config=$(platform_config_digest "${image}@${manifest_digest}")
    [[ ${actual_config} == "${expected_config}" ]] || {
      printf 'published OCI config differs from prepublish image: %s/%s\n' \
        "${component}" "${architecture}" >&2
      return 65
    }
  done
}

if [[ ${existing} == absent ]]; then
  platform_references=()
  for architecture in amd64 arm64; do
    archive_name=$(jq -er \
      --arg component "${component}" --arg architecture "${architecture}" \
      '.candidate.oci[$component][$architecture].archive' "${status}")
    expected_archive_sha=$(jq -er \
      --arg component "${component}" --arg architecture "${architecture}" \
      '.candidate.oci[$component][$architecture].sha256' "${status}")
    local_reference=$(jq -er \
      --arg component "${component}" --arg architecture "${architecture}" \
      '.candidate.oci[$component][$architecture].reference' "${status}")
    expected_config=$(jq -er \
      --arg component "${component}" --arg architecture "${architecture}" \
      '.candidate.oci[$component][$architecture].image_id' "${status}")
    archive=${candidate_root}/oci-archives/${archive_name}
    [[ -f ${archive} && ! -L ${archive} &&
      $(sha256_file "${archive}") == "${expected_archive_sha}" ]] || {
      printf 'prepublish OCI archive drifted: %s/%s\n' "${component}" "${architecture}" >&2
      exit 65
    }
    docker load --input "${archive}" >/dev/null
    test "$(docker image inspect --format '{{.Id}}' "${local_reference}")" = \
      "${expected_config}"
    platform_reference="${image}:${immutable_tag}-${architecture}"
    docker tag "${local_reference}" "${platform_reference}"
    docker push "${platform_reference}" >/dev/null
    test "$(platform_config_digest "${platform_reference}")" = "${expected_config}"
    platform_references+=("${platform_reference}")
  done
  docker buildx imagetools create \
    --tag "${image}:${immutable_tag}" \
    "${platform_references[@]}" >/dev/null
  digest=$(docker buildx imagetools inspect \
    --format '{{json .Manifest.Digest}}' "${image}:${immutable_tag}" | jq -er .)
else
  [[ ${existing} =~ ^sha256:[0-9a-f]{64}$ ]] || {
    printf 'existing OCI digest is invalid: %s\n' "${existing}" >&2
    exit 64
  }
  digest=${existing}
fi

[[ ${digest} =~ ^sha256:[0-9a-f]{64}$ ]]
verify_index "${digest}"
moving=$(docker buildx imagetools inspect \
  --format '{{json .Manifest.Digest}}' "${image}:${moving_tag}" 2>/dev/null | jq -r . || true)
if [[ ${moving} != "${digest}" ]]; then
  docker buildx imagetools create --tag "${image}:${moving_tag}" "${image}@${digest}" \
    >/dev/null
fi
test "$(docker buildx imagetools inspect \
  --format '{{json .Manifest.Digest}}' "${image}:${moving_tag}" | jq -er .)" = "${digest}"
printf '%s\n' "${digest}"
