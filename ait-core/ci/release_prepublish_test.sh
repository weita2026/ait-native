#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
verifier=${repo_root}/ci/release_prepublish_verify.mjs
stage_control=${repo_root}/ci/release_prepublish_stage.sh
oci_control=${repo_root}/ci/release_prepublish_oci.sh
prepublish_workflow=${repo_root}/.github/workflows/ait-release-prepublish-clean-host.yml
publication_workflow=${repo_root}/.github/workflows/pypi-publish.yml
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-prepublish-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-prepublish-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected prepublish test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

write_inventory() {
  local root=$1
  local name=$2
  find "${root}" -type f ! -name "${name}" -print | LC_ALL=C sort |
    while IFS= read -r file; do
      printf '%s  %s\n' "$(sha256_file "${file}")" "${file#"${root}/"}"
    done >"${root}/${name}"
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected prepublish failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

require_direct_exact_artifact_downloads() {
  local workflow=$1
  local line
  local block
  while IFS=: read -r line _; do
    block=$(sed -n "${line},$((line + 5))p" "${workflow}")
    if ! grep -F 'merge-multiple: true' <<<"${block}" >/dev/null; then
      printf 'exact artifact download does not extract into its consumer root: %s:%s\n' \
        "${workflow}" "${line}" >&2
      return 1
    fi
  done < <(grep -n '^[[:space:]]*artifact-ids:' "${workflow}")
}

node --check "${verifier}"
bash -n "${stage_control}"
bash -n "${oci_control}"
for required in \
  'permissions:' \
  'actions: read' \
  'contents: read' \
  'reuse_frozen_candidate:' \
  'Download the previously frozen candidate for control-only retry' \
  'cmp "${comparison_root}/ait-release.clean-host-matrix.json" "${matrix}"' \
  '"failure", "cancelled", "timed_out", "startup_failure",' \
  '"stale", "action_required"' \
  'AIT_NEW_ARTIFACT_DIGEST: ${{ steps.upload.outputs.artifact-digest }}' \
  'if [[ ${candidate_artifact_digest} =~ ^[0-9a-f]{64}$ ]]; then' \
  'candidate_artifact_digest=sha256:${candidate_artifact_digest}' \
  '[[ "${candidate_artifact_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]' \
  'candidate_run_id: ${{ steps.select.outputs.candidate_run_id }}' \
  'run-id: ${{ needs.stage.outputs.candidate_run_id }}' \
  'release_prepublish_verify.mjs qualify' \
  'ait-prepublish-clean-host-${{ inputs.release_id }}'; do
  grep -F -- "${required}" "${prepublish_workflow}" >/dev/null
done
grep -F 'needs: prepublish' "${publication_workflow}" >/dev/null
grep -F 'environment:' "${publication_workflow}" >/dev/null
for required in \
  'reuse_frozen_candidate:' \
  'candidate_run_id:' \
  'candidate_artifact_id:' \
  'candidate_artifact_digest:' \
  'candidate_status_sha256:' \
  'run-id: ${{ needs.prepublish.outputs.candidate_run_id }}'; do
  grep -F -- "${required}" "${publication_workflow}" >/dev/null
done
require_direct_exact_artifact_downloads "${prepublish_workflow}"
require_direct_exact_artifact_downloads "${publication_workflow}"
if grep -E '(^|[[:space:]])(gh release|npm publish|docker push)([[:space:]]|$)' \
  "${prepublish_workflow}" "${stage_control}" >/dev/null; then
  printf 'prepublish controls contain a public endpoint write\n' >&2
  exit 65
fi

candidate=${temporary_root}/candidate
mkdir -p "${candidate}/assets" "${candidate}/oci-archives"
config=${candidate}/ait-release.endpoints.authority.json
jq -S -n '{
  contract: "ait.release.family.endpoints/v1",
  release: {
    id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.6",
    python_version: "1.2.3rc6",
    channel: "rc",
    tag: "v1.2.3-rc.6",
    source_commit: "1111111111111111111111111111111111111111"
  }
}' >"${config}"
printf 'endpoint-stage\n' >"${candidate}/ait-release.endpoint-publication.json"
debian_asset=${candidate}/assets/ait-native_1.2.3~rc.6_amd64.deb
printf 'debian-rc-package\n' >"${debian_asset}"
printf '%s  %s\n' "$(sha256_file "${debian_asset}")" \
  "$(basename "${debian_asset}")" >"${candidate}/assets/SHA256SUMS"
for component in ait-server ait-runner; do
  for architecture in amd64 arm64; do
    printf '%s-%s\n' "${component}" "${architecture}" \
      >"${candidate}/oci-archives/${component}-${architecture}.docker.tar"
  done
done

oci=${temporary_root}/oci.json
jq -S -n --arg root "${candidate}" '
  reduce ["ait-server", "ait-runner"][] as $component ({};
    reduce ["amd64", "arm64"][] as $architecture (.;
      ($component + "-" + $architecture + ".docker.tar") as $archive |
      .[$component][$architecture] = {
        archive: $archive,
        sha256: null,
        reference: ("ait-prepublish/" + $component + ":1.2.3-rc.6-" + $architecture),
        image_id: ("sha256:" + (if $component == "ait-server" then
          (if $architecture == "amd64" then "2" else "3" end)
        else
          (if $architecture == "amd64" then "4" else "5" end)
        end) * 64)
      }
    ))
' >"${oci}"
for component in ait-server ait-runner; do
  for architecture in amd64 arm64; do
    archive=${component}-${architecture}.docker.tar
    jq --arg component "${component}" --arg architecture "${architecture}" \
      --arg digest "$(sha256_file "${candidate}/oci-archives/${archive}")" \
      '.[$component][$architecture].sha256 = $digest' "${oci}" >"${oci}.new"
    mv "${oci}.new" "${oci}"
  done
done

receipt=${candidate}/ait-release.prepublish-stage.json
jq -S -n --slurpfile oci "${oci}" \
  --arg config_sha "$(sha256_file "${config}")" \
  --arg endpoint_sha "$(sha256_file "${candidate}/ait-release.endpoint-publication.json")" \
  --arg assets_sha "$(sha256_file "${candidate}/assets/SHA256SUMS")" '{
  contract: "ait.release.prepublish.stage/v1",
  status: "frozen_candidate_staged",
  release: {
    id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.6",
    tag: "v1.2.3-rc.6",
    source_commit: "1111111111111111111111111111111111111111"
  },
  authority: {
    endpoint_config_sha256: $config_sha,
    endpoint_stage_receipt_sha256: $endpoint_sha,
    assets_checksum_sha256: $assets_sha
  },
  oci: $oci[0],
  mutation: {
    artifact_rebuild: false,
    component_rebuild: false,
    registry_write: false,
    endpoint_write: false,
    github_release_write: false,
    tag_write: false,
    service_start: false
  }
}' >"${receipt}"
status=${candidate}/ait-release.prepublish-candidate.json
jq -S -n --slurpfile oci "${oci}" \
  --arg receipt_sha "$(sha256_file "${receipt}")" '{
  contract: "ait.release.prepublish.candidate/v1",
  status: "frozen_candidate_pending_clean_host",
  release: {
    id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.6",
    tag: "v1.2.3-rc.6",
    source_commit: "1111111111111111111111111111111111111111"
  },
  candidate: {stage_receipt_sha256: $receipt_sha, oci: $oci[0]},
  public_endpoint_writes: false
}' >"${status}"
write_inventory "${candidate}" PREPUBLISH_SHA256SUMS
config_sha=$(sha256_file "${config}")
status_sha=$(sha256_file "${status}")
node "${verifier}" stage --root "${candidate}" \
  --config-sha256 "${config_sha}" --status-sha256 "${status_sha}" >/dev/null

unsafe_inventory=${temporary_root}/unsafe-inventory
cp -R "${candidate}" "${unsafe_inventory}"
{
  printf '%064d  assets/../escape\n' 0
  cat "${candidate}/PREPUBLISH_SHA256SUMS"
} >"${unsafe_inventory}/PREPUBLISH_SHA256SUMS"
expect_failure unsafe-inventory node "${verifier}" stage \
  --root "${unsafe_inventory}" \
  --config-sha256 "${config_sha}" \
  --status-sha256 "${status_sha}"
grep -F 'prepublish checksum inventory contains an unsafe or duplicate row' \
  "${temporary_root}/unsafe-inventory.stderr" >/dev/null

incomplete_mutation=${temporary_root}/incomplete-mutation
cp -R "${candidate}" "${incomplete_mutation}"
jq 'del(.mutation.tag_write)' "${incomplete_mutation}/ait-release.prepublish-stage.json" \
  >"${incomplete_mutation}/ait-release.prepublish-stage.json.new"
mv "${incomplete_mutation}/ait-release.prepublish-stage.json.new" \
  "${incomplete_mutation}/ait-release.prepublish-stage.json"
jq --arg receipt_sha \
  "$(sha256_file "${incomplete_mutation}/ait-release.prepublish-stage.json")" \
  '.candidate.stage_receipt_sha256 = $receipt_sha' \
  "${incomplete_mutation}/ait-release.prepublish-candidate.json" \
  >"${incomplete_mutation}/ait-release.prepublish-candidate.json.new"
mv "${incomplete_mutation}/ait-release.prepublish-candidate.json.new" \
  "${incomplete_mutation}/ait-release.prepublish-candidate.json"
write_inventory "${incomplete_mutation}" PREPUBLISH_SHA256SUMS
expect_failure incomplete-mutation node "${verifier}" stage \
  --root "${incomplete_mutation}" \
  --config-sha256 "${config_sha}" \
  --status-sha256 \
    "$(sha256_file "${incomplete_mutation}/ait-release.prepublish-candidate.json")"

printf 'tamper\n' >>"${candidate}/oci-archives/ait-server-amd64.docker.tar"
expect_failure stage-drift node "${verifier}" stage --root "${candidate}" \
  --config-sha256 "${config_sha}" --status-sha256 "${status_sha}"
printf 'ait-server-amd64\n' >"${candidate}/oci-archives/ait-server-amd64.docker.tar"

qualification=${temporary_root}/qualification
mkdir -p "${qualification}/rows"
for ordinal in $(seq 1 32); do
  printf '{"row":%s}\n' "${ordinal}" \
    >"${qualification}/rows/row-$(printf '%02d' "${ordinal}").json"
done
cp "${status}" "${qualification}/ait-release.prepublish-candidate.json"
artifact_digest=sha256:9999999999999999999999999999999999999999999999999999999999999999
jq -S -n \
  --arg artifact_digest "${artifact_digest}" \
  --arg status_sha "${status_sha}" \
  --arg receipt_sha "$(sha256_file "${receipt}")" '{
  contract: "ait.release.clean-host.aggregate/v1",
  status: "qualified",
  release: {
    verification_stage: "prepublication",
    candidate_artifact_digest: $artifact_digest,
    candidate_stage_receipt_sha256: $receipt_sha,
    operator_status_sha256: $status_sha
  },
  matrix: {expected_rows: 32, admitted_rows: 32, evidence_files: 32},
  failures: [],
  promotion: {allowed: true, retry_same_candidate: false, terminal_for_release: false}
}' >"${qualification}/ait-release.clean-host-status.json"
write_inventory "${qualification}" SHA256SUMS
aggregate_sha=$(sha256_file "${qualification}/ait-release.clean-host-status.json")
node "${verifier}" qualify --root "${qualification}" --candidate-root "${candidate}" \
  --candidate-artifact-digest "${artifact_digest}" \
  --aggregate-sha256 "${aggregate_sha}" >/dev/null

jq '.matrix.admitted_rows = 31' "${qualification}/ait-release.clean-host-status.json" \
  >"${qualification}/ait-release.clean-host-status.json.new"
mv "${qualification}/ait-release.clean-host-status.json.new" \
  "${qualification}/ait-release.clean-host-status.json"
write_inventory "${qualification}" SHA256SUMS
aggregate_sha=$(sha256_file "${qualification}/ait-release.clean-host-status.json")
expect_failure incomplete-qualification node "${verifier}" qualify \
  --root "${qualification}" --candidate-root "${candidate}" \
  --candidate-artifact-digest "${artifact_digest}" \
  --aggregate-sha256 "${aggregate_sha}"

printf 'release prepublish contract: pass\n'
