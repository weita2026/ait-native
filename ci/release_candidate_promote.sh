#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  printf '%s\n' \
    'usage: release_candidate_promote.sh <endpoint-config> <protected-evidence> <aggregate-root> <candidate-root>' >&2
  exit 64
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
endpoint_config=$1
protected_evidence=$2
aggregate_root=$3
candidate_root=$4

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

require_file() {
  [[ -f $1 && ! -L $1 ]] || {
    printf '%s must be a regular non-symlink file: %s\n' "$2" "$1" >&2
    return 66
  }
}

for input in "${endpoint_config}" "${protected_evidence}"; do
  require_file "${input}" 'promotion input'
done
for directory in "${aggregate_root}" "${candidate_root}"; do
  [[ ${directory} == /* && -d ${directory} && ! -L ${directory} ]] || {
    printf 'promotion directory must be absolute and real: %s\n' "${directory}" >&2
    exit 66
  }
done

bash "${repo_root}/ci/release_operator.sh" validate-config \
  --config "${endpoint_config}" >/dev/null
authority=${candidate_root}/ait-release.endpoints.authority.json
candidate_status=${candidate_root}/ait-release.prepublish-candidate.json
candidate_receipt=${candidate_root}/ait-release.endpoint-publication.json
aggregate_status=${aggregate_root}/ait-release.clean-host-status.json
assets=${candidate_root}/assets
for input in "${authority}" "${candidate_status}" "${candidate_receipt}" "${aggregate_status}"; do
  require_file "${input}" 'qualified candidate input'
done
[[ -d ${assets} && ! -L ${assets} ]] || {
  printf 'qualified candidate assets directory is unavailable\n' >&2
  exit 66
}

protected_sha=$(sha256_file "${protected_evidence}")
[[ ${protected_sha} == $(jq -er '.protected_authorization.evidence_sha256' "${endpoint_config}") ]] || {
  printf 'protected evidence differs from endpoint authority\n' >&2
  exit 65
}
jq -e --slurpfile endpoint "${endpoint_config}" '
  .contract == "ait.release.family.pre-tag-candidate-authority/v1" and
  .status == "ready_for_pre_tag_clean_host" and
  .release == $endpoint[0].release and
  .source_dossier == $endpoint[0].source_dossier and
  .tag_authority.required_state == "absent"
' "${authority}" >/dev/null || {
  printf 'qualified candidate authority differs from final endpoint authority\n' >&2
  exit 65
}
jq -e --slurpfile endpoint "${endpoint_config}" '
  .contract == "ait.release.clean-host.aggregate/v1" and
  .status == "qualified" and
  .release.id == $endpoint[0].release.id and
  .release.version == $endpoint[0].release.version and
  .release.source_commit == $endpoint[0].release.source_commit and
  .release.verification_stage == "pre_tag" and
  .release.candidate_artifact_digest ==
    $endpoint[0].pre_tag_qualification.candidate_artifact_digest and
  .matrix.expected_rows == 32 and .matrix.admitted_rows == 32 and
  .matrix.evidence_files == 32 and .failures == [] and
  .promotion.allowed == true
' "${aggregate_status}" >/dev/null || {
  printf 'pre-tag aggregate does not authorize this exact endpoint payload\n' >&2
  exit 65
}
jq -e --slurpfile endpoint "${endpoint_config}" '
  .contract == "ait.release.family.protected-promotion/v1" and
  .status == "authorized_for_explicit_endpoint_promotion" and
  .release_id == $endpoint[0].release.id and
  .version == $endpoint[0].release.version and
  .channel == $endpoint[0].release.channel and
  .tag == $endpoint[0].release.tag and
  .snapshot_id == $endpoint[0].release.coordinator_snapshot and
  .public_source.repository == $endpoint[0].publisher.repository and
  .public_source.git_commit == $endpoint[0].release.source_commit and
  .public_source.status == "verified" and
  .public_source.anonymous_tag_readback == true and
  .public_source.commit_tree_equal == true and
  .public_source.archived_source_equal == true and
  .dossier.source_run_id == ($endpoint[0].source_dossier.workflow_run_id | tostring) and
  .dossier.source_run_attempt == ($endpoint[0].source_dossier.workflow_run_attempt | tostring) and
  .dossier.source_workflow_sha == $endpoint[0].source_dossier.workflow_control_commit and
  .dossier.artifact_id == ($endpoint[0].source_dossier.artifact_id | tostring) and
  .dossier.artifact_digest == $endpoint[0].source_dossier.artifact_digest and
  .dossier.frozen_manifest_sha256 == $endpoint[0].release.frozen_manifest_sha256 and
  .dossier.checksum_sha256 == $endpoint[0].release.frozen_checksums_sha256 and
  .dossier.native_promotion_readback_equal == true and
  .pre_tag_qualification == {
    workflow_run_id: ($endpoint[0].pre_tag_qualification.workflow_run_id | tostring),
    workflow_run_attempt: ($endpoint[0].pre_tag_qualification.workflow_run_attempt | tostring),
    workflow_control_commit: $endpoint[0].pre_tag_qualification.workflow_control_commit,
    candidate_artifact_id: ($endpoint[0].pre_tag_qualification.candidate_artifact_id | tostring),
    candidate_artifact_digest: $endpoint[0].pre_tag_qualification.candidate_artifact_digest,
    candidate_status_sha256: $endpoint[0].pre_tag_qualification.candidate_status_sha256,
    aggregate_artifact_id: ($endpoint[0].pre_tag_qualification.aggregate_artifact_id | tostring),
    aggregate_artifact_digest: $endpoint[0].pre_tag_qualification.aggregate_artifact_digest,
    aggregate_status_sha256: $endpoint[0].pre_tag_qualification.aggregate_status_sha256,
    clean_host_rows: 32,
    tag_state_at_closeout: "absent"
  } and
  .authorization.required == true and .authorization.granted == true and
  .authorization.exact_digest_approval == true and
  .authorization.boundary == "github_protected_environment" and
  .authorization.protected_environment == ($endpoint[0].release.channel + "-promotion") and
  .authorization.workflow_run_id ==
    ($endpoint[0].protected_authorization.workflow_run_id | tostring) and
  .authorization.workflow_run_attempt ==
    ($endpoint[0].protected_authorization.workflow_run_attempt | tostring) and
  .authorization.workflow_sha == $endpoint[0].protected_authorization.workflow_control_commit and
  .mutation == {
    artifact_rebuild: false,
    component_rebuild: false,
    registry_credentials_loaded: false,
    registry_write: false,
    github_release_write: false,
    tag_write: false,
    ait_remote_release_activation: false,
    service_mutation: false
  }
' "${protected_evidence}" >/dev/null || {
  printf 'protected evidence does not bind the pre-tag qualification\n' >&2
  exit 65
}

cp -p "${endpoint_config}" "${assets}/ait-release.endpoints.json"
cp -p "${protected_evidence}" "${assets}/ait-release.protected-promotion.json"
find "${assets}" -mindepth 1 -maxdepth 1 -type f ! -name SHA256SUMS -print |
  LC_ALL=C sort |
  while IFS= read -r asset; do
    printf '%s  %s\n' "$(sha256_file "${asset}")" "$(basename -- "${asset}")"
  done >"${assets}/SHA256SUMS"

release_id=$(jq -er '.release.id' "${endpoint_config}")
release_version=$(jq -er '.release.version' "${endpoint_config}")
release_tag=$(jq -er '.release.tag' "${endpoint_config}")
release_channel=$(jq -er '.release.channel' "${endpoint_config}")
if [[ ${release_channel} == rc ]]; then
  winget_stage_status=forbidden_for_rc
else
  winget_stage_status=requires_external_community_submission
fi
asset_count=$(awk 'NF {count += 1} END {print count + 0}' "${assets}/SHA256SUMS")
jq -S -n \
  --arg release_id "${release_id}" --arg version "${release_version}" \
  --arg tag "${release_tag}" \
  --arg endpoint_config_sha256 "$(sha256_file "${endpoint_config}")" \
  --arg protected_evidence_sha256 "${protected_sha}" \
  --arg release_checksums_sha256 "$(sha256_file "${assets}/SHA256SUMS")" \
  --arg candidate_artifact_digest \
    "$(jq -er '.pre_tag_qualification.candidate_artifact_digest' "${endpoint_config}")" \
  --arg aggregate_artifact_digest \
    "$(jq -er '.pre_tag_qualification.aggregate_artifact_digest' "${endpoint_config}")" \
  --arg aggregate_status_sha256 \
    "$(jq -er '.pre_tag_qualification.aggregate_status_sha256' "${endpoint_config}")" \
  --arg winget_stage_status "${winget_stage_status}" \
  --argjson asset_count "${asset_count}" --slurpfile config "${endpoint_config}" '
    {
      contract: "ait.release.family.endpoint-publication/v1",
      status: "ready_for_authenticated_endpoint_preflight",
      release_id: $release_id,
      version: $version,
      tag: $tag,
      endpoint_config_sha256: $endpoint_config_sha256,
      protected_evidence_sha256: $protected_evidence_sha256,
      release_checksums_sha256: $release_checksums_sha256,
      release_asset_count: $asset_count,
      endpoints: $config[0].endpoints,
      pre_tag_qualification: {
        candidate_artifact_digest: $candidate_artifact_digest,
        aggregate_artifact_digest: $aggregate_artifact_digest,
        aggregate_status_sha256: $aggregate_status_sha256,
        clean_host_rows: 32,
        payload_rebuilt: false
      },
      checks: {
        pre_tag_clean_host: "pass",
        protected_authorization: "pass",
        frozen_checksums: "pass",
        package_receipts: "pass",
        package_checksums: "pass",
        github_asset_staging: "pass",
        oci_context_staging: "pass",
        winget_community_submission: $winget_stage_status
      },
      mutation: {
        artifact_rebuild: false,
        component_rebuild: false,
        credentials_loaded: false,
        registry_write: false,
        github_release_write: false,
        endpoint_repository_write: false,
        tag_write: false,
        ait_remote_release_activation: false,
        service_mutation: false
      },
      next_action: {
        code: "authenticated_all_endpoint_preflight",
        detail: "Prove every configured credential and remote identity before the first package write."
      }
    }
  ' >"${candidate_receipt}"
chmod 0644 "${assets}/ait-release.endpoints.json" \
  "${assets}/ait-release.protected-promotion.json" "${assets}/SHA256SUMS" \
  "${candidate_receipt}"
printf '%s\n' "${candidate_root}"
