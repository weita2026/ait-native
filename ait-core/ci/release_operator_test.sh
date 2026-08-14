#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
operator=${repo_root}/ci/release_operator.sh
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-operator-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-operator-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected release-operator test path: %s\n' \
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

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected release-operator failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

test -x "${operator}"
bash -n "${operator}"

source_root=${temporary_root}/public-source
mkdir -p "${source_root}/ci"
jq -n '
  {
    schema: "ait.release.family/v3",
    family: {
      name: "ait-native",
      version: "1.2.3-rc.5",
      channel: "rc",
      tag: "v1.2.3-rc.5"
    },
    public_source: {
      model: "release-monorepo",
      identity: "weita2026/ait-native",
      product_document: "docs/distribution.md",
      subtrees: [
        {source_repository: "ait-core"},
        {source_repository: "ait-server"},
        {source_repository: "ait-runner"},
        {source_repository: "ait-python"},
        {source_repository: "ait-node"}
      ]
    },
    components: [
      {id: "ait", source_repository: "ait-core", version: "1.2.3-rc.5"},
      {id: "ait-server", source_repository: "ait-server", version: "1.2.3-rc.5"},
      {id: "ait-runner", source_repository: "ait-runner", version: "1.2.3-rc.5"},
      {id: "ait-python", source_repository: "ait-python", version: "1.2.3rc5"},
      {id: "ait-node", source_repository: "ait-node", version: "1.2.3-rc.5"}
    ]
  }
' >"${source_root}/ait-release-family.json"
family_sha=$(sha256_file "${source_root}/ait-release-family.json")
jq -n --arg family_sha "${family_sha}" '
  {
    schema: "ait.release.monorepo-source/v1",
    public_source_identity: "weita2026/ait-native",
    coordinator_snapshot: "SNP-ABCDEF123456",
    family_version: "1.2.3-rc.5",
    family_tag: "v1.2.3-rc.5",
    family_manifest_sha256: $family_sha,
    subtrees: [1, 2, 3, 4, 5],
    git_commit_created: false,
    public_publish: false
  }
' >"${source_root}/ait-monorepo-source.json"
jq -n '{family_version: "1.2.3-rc.5", public_publish: false}' \
  >"${source_root}/ci/release_repository_authorities.json"
jq -n '{version: "1.2.3-rc.5", public_publish: false}' \
  >"${source_root}/ci/native_bootstrap_matrix.json"
printf '%s\n' \
  '#!/usr/bin/env node' \
  'if (!process.argv.includes("--validate-only")) process.exit(64);' \
  >"${source_root}/build-release.mjs"
chmod 0755 "${source_root}/build-release.mjs"
git -C "${source_root}" init -q
git -C "${source_root}" config user.name 'AIT release operator test'
git -C "${source_root}" config user.email 'release-operator@localhost'
git -C "${source_root}" add -A
git -C "${source_root}" commit -qm 'fixture release source'
git -C "${source_root}" tag -a v1.2.3-rc.5 -m 'fixture release tag'
source_commit=$(git -C "${source_root}" rev-parse HEAD)

prepare=${temporary_root}/prepare.json
"${operator}" prepare --source-root "${source_root}" --output "${prepare}" >/dev/null
jq -e --arg commit "${source_commit}" '
  .contract == "ait.release.operator.prepare/v1" and
  .status == "ready_for_component_receipts" and
  .release.version == "1.2.3-rc.5" and .release.channel == "rc" and
  .release.python_version == "1.2.3rc5" and .release.source_commit == $commit and
  .receipt_dispatch.inputs.coordinator_snapshot == "SNP-ABCDEF123456" and
  .receipt_dispatch.requested == false and
  ([.mutation[]] | all(. == false))
' "${prepare}" >/dev/null

version_drift=${temporary_root}/version-drift
git clone -q "${source_root}" "${version_drift}"
jq '.family.version = "1.2.3-rc.6"' \
  "${version_drift}/ait-release-family.json" >"${temporary_root}/version-drift.json"
mv "${temporary_root}/version-drift.json" "${version_drift}/ait-release-family.json"
expect_failure version-drift "${operator}" prepare \
  --source-root "${version_drift}" --output "${temporary_root}/version-drift-output.json"
grep -F 'family identity is inconsistent' "${temporary_root}/version-drift.stderr" >/dev/null
expect_failure relative-output "${operator}" prepare \
  --source-root "${source_root}" --output relative.json

lightweight_tag=${temporary_root}/lightweight-tag
git clone -q "${source_root}" "${lightweight_tag}"
git -C "${lightweight_tag}" tag -d v1.2.3-rc.5 >/dev/null
git -C "${lightweight_tag}" tag v1.2.3-rc.5
expect_failure lightweight-tag "${operator}" prepare \
  --source-root "${lightweight_tag}" \
  --output "${temporary_root}/lightweight-tag-output.json"
grep -F 'public release tag must be an annotated tag object' \
  "${temporary_root}/lightweight-tag.stderr" >/dev/null

dossier=${temporary_root}/dossier
mkdir -p "${dossier}/frozen"
cp "${source_root}/ait-monorepo-source.json" "${dossier}/ait-monorepo-source.json"
control_commit=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
jq -n --arg commit "${source_commit}" --arg control "${control_commit}" \
  --arg mapping_sha "$(sha256_file "${dossier}/ait-monorepo-source.json")" '
  {
    contract: "ait.release.public-git-source/v1",
    status: "ready",
    public_source_identity: "weita2026/ait-native",
    git_commit: $commit,
    workflow_control_commit: $control,
    coordinator_snapshot: "SNP-ABCDEF123456",
    mapping_sha256: $mapping_sha,
    registry_write: false,
    public_publish: false
  }
' >"${dossier}/ait-public-git-source.evidence.json"
jq -n --arg family_sha "${family_sha}" '
  {
    contract: "ait.release.family.candidate/v1",
    release_id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.5",
    channel: "rc",
    tag: "v1.2.3-rc.5",
    snapshot_id: "SNP-ABCDEF123456",
    family_manifest_sha256: $family_sha
  }
' >"${dossier}/ait-release.candidate.json"
jq -n '
  {
    contract: "ait.release.family.promotion/v1",
    release_id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.5",
    channel: "rc",
    tag: "v1.2.3-rc.5",
    status: "ready_for_protected_ci",
    authorization: {required: true, granted: false},
    mutation: {performed: false, registry_write: false}
  }
' >"${dossier}/ait-release.promotion.json"
jq -n '
  {
    contract: "ait.release.family.frozen/v1",
    release_id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.5",
    channel: "rc",
    tag: "v1.2.3-rc.5",
    snapshot_id: "SNP-ABCDEF123456",
    promotion: {authorized: false, registry_write: false}
  }
' >"${dossier}/frozen/ait-release-family.manifest.json"
printf '%064d  fixture.bin\n' 0 >"${dossier}/frozen/SHA256SUMS"

receipt_run=${temporary_root}/receipt-run.json
receipt_artifact=${temporary_root}/receipt-artifact.json
jq -n --arg head "${control_commit}" '
  {
    id: 101,
    run_attempt: 1,
    name: "ait release component receipts",
    path: ".github/workflows/ait-release-component-receipts.yml",
    event: "workflow_dispatch",
    status: "completed",
    conclusion: "success",
    head_sha: $head
  }
' >"${receipt_run}"
jq -n '
  {
    id: 201,
    name: "ait-family-dossier-REL-FAM-0123456789ABCDEF",
    digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    expired: false,
    workflow_run: {id: 101}
  }
' >"${receipt_artifact}"

receipts=${temporary_root}/receipts.json
"${operator}" bind-receipts \
  --prepare "${prepare}" \
  --run-record "${receipt_run}" \
  --artifact-record "${receipt_artifact}" \
  --dossier-root "${dossier}" \
  --output "${receipts}" >/dev/null
jq -e '
  .contract == "ait.release.operator.receipt-binding/v1" and
  .status == "ready_for_protected_authorization" and
  .release.id == "REL-FAM-0123456789ABCDEF" and
  .source_dossier.workflow_run_id == 101 and
  .source_dossier.artifact_id == 201 and
  .protected_dispatch.inputs.channel == "rc" and
  .protected_dispatch.requested == false and
  ([.mutation[]] | all(. == false))
' "${receipts}" >/dev/null

jq '.id = 999' "${receipt_run}" >"${temporary_root}/wrong-run.json"
expect_failure wrong-run "${operator}" bind-receipts \
  --prepare "${prepare}" \
  --run-record "${temporary_root}/wrong-run.json" \
  --artifact-record "${receipt_artifact}" \
  --dossier-root "${dossier}" \
  --output "${temporary_root}/wrong-run-output.json"
jq '.workflow_run.id = 999' "${receipt_artifact}" \
  >"${temporary_root}/wrong-artifact.json"
expect_failure wrong-artifact "${operator}" bind-receipts \
  --prepare "${prepare}" \
  --run-record "${receipt_run}" \
  --artifact-record "${temporary_root}/wrong-artifact.json" \
  --dossier-root "${dossier}" \
  --output "${temporary_root}/wrong-artifact-output.json"
jq '.mutation.registry_write = true' "${prepare}" \
  >"${temporary_root}/mutating-prepare.json"
expect_failure mutating-prepare "${operator}" bind-receipts \
  --prepare "${temporary_root}/mutating-prepare.json" \
  --run-record "${receipt_run}" \
  --artifact-record "${receipt_artifact}" \
  --dossier-root "${dossier}" \
  --output "${temporary_root}/mutating-prepare-output.json"

protected_run=${temporary_root}/protected-run.json
protected_artifact=${temporary_root}/protected-artifact.json
protected_evidence=${temporary_root}/protected-evidence.json
protected_control=cccccccccccccccccccccccccccccccccccccccc
jq -n --arg head "${protected_control}" '
  {
    id: 102,
    run_attempt: 2,
    name: "ait release protected promotion",
    path: ".github/workflows/ait-release-protected-promotion.yml",
    event: "workflow_dispatch",
    status: "completed",
    conclusion: "success",
    head_sha: $head
  }
' >"${protected_run}"
jq -n '
  {
    id: 202,
    name: "ait-protected-promotion-REL-FAM-0123456789ABCDEF",
    digest: "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    expired: false,
    workflow_run: {id: 102}
  }
' >"${protected_artifact}"
jq -n \
  --slurpfile receipts "${receipts}" \
  --arg protected_control "${protected_control}" '
  ($receipts[0]) as $r |
  {
    contract: "ait.release.family.protected-promotion/v1",
    status: "authorized_for_explicit_endpoint_promotion",
    release_id: $r.release.id,
    version: $r.release.version,
    channel: $r.release.channel,
    tag: $r.release.tag,
    snapshot_id: $r.release.coordinator_snapshot,
    public_source: {
      repository: $r.release.repository,
      git_commit: $r.release.source_commit,
      status: "verified"
    },
    dossier: {
      source_run_id: ($r.source_dossier.workflow_run_id | tostring),
      source_run_attempt: ($r.source_dossier.workflow_run_attempt | tostring),
      source_workflow_sha: $r.source_dossier.workflow_control_commit,
      artifact_id: ($r.source_dossier.artifact_id | tostring),
      artifact_digest: $r.source_dossier.artifact_digest,
      frozen_manifest_sha256: $r.release.frozen_manifest_sha256,
      checksum_sha256: $r.release.frozen_checksums_sha256,
      native_promotion_readback_equal: true,
      admission_replay: {
        model: "immutable-tag-native-admission/v1",
        rust_toolchain: "1.96.0",
        cargo_lock_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
        family_packages_sha256: "2222222222222222222222222222222222222222222222222222222222222222",
        family_release_sha256: "3333333333333333333333333333333333333333333333333333333333333333"
      }
    },
    authorization: {
      required: true,
      granted: true,
      exact_digest_approval: true,
      boundary: "github_protected_environment",
      protected_environment: "rc-promotion",
      workflow_run_id: "102",
      workflow_run_attempt: "2",
      workflow_sha: $protected_control
    },
    mutation: {
      artifact_rebuild: false,
      component_rebuild: false,
      registry_credentials_loaded: false,
      registry_write: false,
      github_release_write: false,
      tag_write: false,
      ait_remote_release_activation: false,
      service_mutation: false
    }
  }
' >"${protected_evidence}"

endpoint_config=${temporary_root}/endpoint-config.json
"${operator}" bind-authorization \
  --receipts "${receipts}" \
  --run-record "${protected_run}" \
  --artifact-record "${protected_artifact}" \
  --protected-evidence "${protected_evidence}" \
  --output "${endpoint_config}" >/dev/null
"${operator}" validate-config --config "${endpoint_config}" \
  --expected-release-id REL-FAM-0123456789ABCDEF >/dev/null
jq -e '
  .release.channel == "rc" and .release.python_version == "1.2.3rc5" and
  .protected_authorization.workflow_run_id == 102 and
  .protected_authorization.artifact_id == 202 and
  .endpoints.github.prerelease == false and
  .endpoints.npm.dist_tag == "rc" and
  .endpoints.homebrew.formula_path == "Formula/ait-native-rc.rb" and
  .endpoints.apt.suite == "testing" and
  .endpoints.winget == {
    identity: "Weita.AitNative",
    route: "validation",
    community_manifest_submission: false
  } and
  .endpoints.oci.immutable_tag == "1.2.3-rc.5" and
  .endpoints.oci.moving_tag == "rc"
' "${endpoint_config}" >/dev/null

jq '.release_id = "REL-FAM-FFFFFFFFFFFFFFFF"' "${protected_evidence}" \
  >"${temporary_root}/altered-evidence.json"
expect_failure altered-evidence "${operator}" bind-authorization \
  --receipts "${receipts}" \
  --run-record "${protected_run}" \
  --artifact-record "${protected_artifact}" \
  --protected-evidence "${temporary_root}/altered-evidence.json" \
  --output "${temporary_root}/altered-evidence-output.json"

stable_receipts=${temporary_root}/stable-receipts.json
jq '
  .release.version = "1.2.3" |
  .release.channel = "stable" |
  .release.python_version = "1.2.3" |
  .release.tag = "v1.2.3" |
  .protected_dispatch.inputs.channel = "stable" |
  .protected_dispatch.inputs.tag = "v1.2.3"
' "${receipts}" >"${stable_receipts}"
stable_evidence=${temporary_root}/stable-evidence.json
jq '
  .version = "1.2.3" |
  .channel = "stable" |
  .tag = "v1.2.3" |
  .authorization.protected_environment = "stable-promotion"
' "${protected_evidence}" >"${stable_evidence}"
stable_config=${temporary_root}/stable-config.json
"${operator}" bind-authorization \
  --receipts "${stable_receipts}" \
  --run-record "${protected_run}" \
  --artifact-record "${protected_artifact}" \
  --protected-evidence "${stable_evidence}" \
  --output "${stable_config}" >/dev/null
jq -e '
  .release.channel == "stable" and .release.python_version == "1.2.3" and
  .endpoints.npm.dist_tag == "latest" and
  .endpoints.homebrew.formula_path == "Formula/ait-native.rb" and
  .endpoints.apt.suite == "stable" and
  .endpoints.winget == {
    identity: "Weita.AitNative",
    route: "community",
    community_manifest_submission: true
  } and
  .endpoints.oci.moving_tag == "latest"
' "${stable_config}" >/dev/null

evidence_root=${temporary_root}/endpoint-evidence
mkdir "${evidence_root}"
server_digest=sha256:4444444444444444444444444444444444444444444444444444444444444444
runner_digest=sha256:5555555555555555555555555555555555555555555555555555555555555555
jq -n --arg server "${server_digest}" --arg runner "${runner_digest}" '
  {
    contract: "ait.release.family.endpoint-readback/v1",
    status: "published_pending_clean_host_smoke",
    release_id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.5",
    tag: "v1.2.3-rc.5",
    endpoints: {
      github: "published_and_read_back",
      pypi: "published_and_read_back",
      npm: "published_and_read_back",
      homebrew: "published_and_read_back",
      apt: "published_signed_and_read_back",
      winget: "validation_assets_published_no_community_submission",
      oci: {
        server: $server,
        runner: $runner,
        immutable_tag: "1.2.3-rc.5",
        moving_tag: "rc"
      }
    },
    next_action: "run_all_declared_clean_host_install_upgrade_uninstall_smoke"
  }
' >"${evidence_root}/ait-release.endpoint-readback.json"
for name in github pypi npm homebrew; do
  jq -n --arg name "${name}" '{contract: $name}' >"${evidence_root}/${name}.json"
done
jq -n '
  {
    contract: "ait.release.endpoint.apt/v1",
    release_id: "REL-FAM-0123456789ABCDEF",
    status: "published_signed_and_read_back",
    signature_readback: true,
    package_digest_readback: true,
    apt_cache_search: true
  }
' >"${evidence_root}/apt.json"
jq -n --arg server "${server_digest}" --arg runner "${runner_digest}" '
  {
    contract: "ait.release.endpoint.oci-state/v1",
    images: {
      "ait-server": {digest: $server},
      "ait-runner": {digest: $runner}
    }
  }
' >"${evidence_root}/oci-state.json"
endpoint_run=${temporary_root}/endpoint-run.json
endpoint_artifact=${temporary_root}/endpoint-artifact.json
jq -n '
  {
    id: 103,
    run_attempt: 1,
    name: "ait release endpoint publication",
    path: ".github/workflows/pypi-publish.yml",
    event: "workflow_dispatch",
    status: "completed",
    conclusion: "success",
    head_sha: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
  }
' >"${endpoint_run}"
jq -n '
  {
    id: 203,
    name: "ait-endpoint-publication-REL-FAM-0123456789ABCDEF",
    digest: "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    expired: false,
    workflow_run: {id: 103}
  }
' >"${endpoint_artifact}"
status_output=${temporary_root}/status.json
"${operator}" status \
  --config "${endpoint_config}" \
  --run-record "${endpoint_run}" \
  --artifact-record "${endpoint_artifact}" \
  --evidence-root "${evidence_root}" \
  --output "${status_output}" >/dev/null
jq -e '
  .contract == "ait.release.operator.status/v1" and
  .status == "published_pending_clean_host_smoke" and
  .release.id == "REL-FAM-0123456789ABCDEF" and
  .publication_workflow.run_id == 103 and
  .platforms.apt == "published_signed_and_read_back" and
  .platforms.winget == "validation_assets_published_no_community_submission"
' "${status_output}" >/dev/null

printf 'release operator contract: pass\n'
