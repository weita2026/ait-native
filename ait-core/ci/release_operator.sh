#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
defaults=${repo_root}/release/endpoint-publication.defaults.json

usage() {
  cat >&2 <<'USAGE'
usage:
  release_operator.sh prepare --source-root <absolute-dir> --output <absolute-json> [--dispatch]
  release_operator.sh bind-receipts --prepare <json> --run-id <id> --output <absolute-json> [--dispatch]
  release_operator.sh bind-receipts --prepare <json> --run-record <json> --artifact-record <json> --dossier-root <absolute-dir> --output <absolute-json> [--dispatch]
  release_operator.sh bind-authorization --receipts <json> --run-id <id> --output <absolute-json> [--dispatch]
  release_operator.sh bind-authorization --receipts <json> --run-record <json> --artifact-record <json> --protected-evidence <json> --output <absolute-json> [--dispatch]
  release_operator.sh validate-config --config <json> [--expected-release-id <REL-FAM-id>]
  release_operator.sh status --config <json> --run-id <id> --output <absolute-json>
  release_operator.sh status --config <json> --run-record <json> --artifact-record <json> --evidence-root <absolute-dir> --output <absolute-json>
  release_operator.sh clean-host --config <json> --status <json> --prior-version <semver> --prior-python-version <pep440> --output <absolute-json> [--dispatch]
  release_operator.sh validate-clean-host-request --request <json> --config <json> --status <json>
  release_operator.sh clean-host-status --request <json> --config <json> --status <json> --run-id <id> --output <absolute-json>
  release_operator.sh clean-host-status --request <json> --config <json> --status <json> --run-record <json> --artifact-record <json> --evidence-root <absolute-dir> --output <absolute-json>

All preparation and binding modes are non-publishing by default. --dispatch only
starts the next reviewed GitHub Actions workflow; registry writes remain inside
the workflow's protected environment.
USAGE
  exit 64
}

fail() {
  local code=$1
  shift
  printf '%s\n' "$*" >&2
  exit "${code}"
}

for command in awk base64 git jq node; do
  command -v "${command}" >/dev/null 2>&1 ||
    fail 69 "required release-operator command is unavailable: ${command}"
done

sha256_file() {
  local path=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${path}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${path}" | awk '{print $1}'
  else
    fail 69 'no SHA-256 utility is available'
  fi
}

require_regular_file() {
  local path=$1
  local label=$2
  [[ -f ${path} && ! -L ${path} ]] ||
    fail 66 "${label} must be a regular non-symlink file: ${path}"
}

require_real_directory() {
  local path=$1
  local label=$2
  [[ ${path} == /* && -d ${path} && ! -L ${path} ]] ||
    fail 66 "${label} must be an absolute real directory: ${path}"
}

canonical_file() {
  local path=$1
  (cd "$(dirname -- "${path}")" && printf '%s/%s\n' "$(pwd -P)" "$(basename -- "${path}")")
}

canonical_directory() {
  local path=$1
  (cd "${path}" && pwd -P)
}

require_new_output() {
  local path=$1
  local label=$2
  [[ ${path} == /* ]] || fail 64 "${label} must use an absolute path"
  [[ ! -e ${path} && ! -L ${path} ]] || fail 73 "${label} already exists: ${path}"
  require_real_directory "$(dirname -- "${path}")" "${label} parent"
}

validate_defaults() {
  require_regular_file "${defaults}" 'endpoint defaults'
  jq -e '
    .contract == "ait.release.endpoint-defaults/v1" and
    .publisher == {
      repository: "weita2026/ait-native",
      workflow: "pypi-publish.yml",
      environment: "pypi"
    } and
    .endpoints.github == {repository: "weita2026/ait-native"} and
    .endpoints.pypi == {
      base_url: "https://pypi.org",
      identity: "ait-native",
      trusted_publisher: {
        repository: "weita2026/ait-native",
        workflow: "pypi-publish.yml",
        environment: "pypi"
      }
    } and
    .endpoints.npm.registry == "https://registry.npmjs.org" and
    .endpoints.npm.credential_secret == "AIT_NPM_TOKEN" and
    .endpoints.npm.packages == [
      "@wa120/ait-native",
      "@wa120/ait-native-darwin-arm64",
      "@wa120/ait-native-darwin-x64",
      "@wa120/ait-native-linux-arm64",
      "@wa120/ait-native-linux-x64",
      "@wa120/ait-native-win32-arm64",
      "@wa120/ait-native-win32-x64"
    ] and
    .endpoints.homebrew.formula_paths == {
      rc: "Formula/ait-native-rc.rb",
      stable: "Formula/ait-native.rb"
    } and
    .endpoints.apt.suites == {rc: "testing", stable: "stable"} and
    (.endpoints.apt.signing_fingerprint | test("^[0-9A-F]{40}$")) and
    .endpoints.winget.routes == {
      rc: {route: "validation", community_manifest_submission: false},
      stable: {route: "community", community_manifest_submission: true}
    } and
    (.endpoints.oci.dockerfile_frontend | test("^docker/dockerfile:[^@]+@sha256:[0-9a-f]{64}$")) and
    (.endpoints.oci.base_image | test("^docker\\.io/[^@]+@sha256:[0-9a-f]{64}$")) and
    .endpoints.oci.images == [
      "ghcr.io/weita2026/ait-server",
      "ghcr.io/weita2026/ait-runner"
    ]
  ' "${defaults}" >/dev/null || fail 65 'endpoint defaults are not the reviewed static route'
}

release_identity_tsv() {
  local family=$1
  jq -er '
    . as $root |
    .schema == "ait.release.family/v3" and
    .family.name == "ait-native" and
    (.family.channel == "rc" or .family.channel == "stable") and
    .family.tag == ("v" + .family.version) and
    .public_source.model == "release-monorepo" and
    .public_source.identity == "weita2026/ait-native" and
    .public_source.product_document == "docs/distribution.md" and
    ([.public_source.subtrees[].source_repository] | sort) ==
      ["ait-core", "ait-node", "ait-python", "ait-runner", "ait-server"] and
    ([.components[].source_repository] | unique | sort) ==
      ["ait-core", "ait-node", "ait-python", "ait-runner", "ait-server"] and
    ([.components[] | select(.id != "ait-python") | .version] | unique) ==
      [.family.version] and
    ([.components[] | select(.id == "ait-python") | .version] | length) == 1
    | if . then
        [
          $root.family.version,
          $root.family.channel,
          $root.family.tag,
          ($root.components[] | select(.id == "ait-python") | .version)
        ] | @tsv
      else
        error("family identity is inconsistent")
      end
  ' "${family}"
}

validate_version_identity() {
  local version=$1
  local channel=$2
  local tag=$3
  local python_version=$4
  local base ordinal expected_python
  [[ ${tag} == "v${version}" ]] || fail 65 'release tag and version differ'
  case "${channel}" in
    rc)
      [[ ${version} =~ ^([0-9]+\.[0-9]+\.[0-9]+)-rc\.([1-9][0-9]*)$ ]] ||
        fail 65 'RC version is not canonical SemVer'
      base=${BASH_REMATCH[1]}
      ordinal=${BASH_REMATCH[2]}
      expected_python=${base}rc${ordinal}
      ;;
    stable)
      [[ ${version} =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
        fail 65 'stable version is not canonical SemVer'
      expected_python=${version}
      ;;
    *) fail 65 'release channel must be rc or stable' ;;
  esac
  [[ ${python_version} == "${expected_python}" ]] ||
    fail 65 'Python version does not match the family channel and version'
}

validate_endpoint_config() {
  local config=$1
  local expected_release_id=${2:-}
  local version channel tag python_version release_id
  require_regular_file "${config}" 'endpoint configuration'
  validate_defaults
  if ! jq -e --slurpfile defaults "${defaults}" '
    ($defaults[0]) as $d |
    .contract == "ait.release.family.endpoints/v1" and
    (.release | keys | sort) == ([
      "channel", "coordinator_snapshot", "frozen_checksums_sha256",
      "frozen_manifest_sha256", "id", "python_version", "source_commit",
      "tag", "version"
    ] | sort) and
    (.release.id | test("^REL-FAM-[0-9A-F]{16}$")) and
    (.release.source_commit | test("^[0-9a-f]{40}$")) and
    (.release.coordinator_snapshot | test("^SNP-[0-9A-F]{12}$")) and
    (.release.frozen_manifest_sha256 | test("^[0-9a-f]{64}$")) and
    (.release.frozen_checksums_sha256 | test("^[0-9a-f]{64}$")) and
    (.source_dossier.workflow_run_id | type == "number" and . > 0 and floor == .) and
    (.source_dossier.workflow_run_attempt | type == "number" and . > 0 and floor == .) and
    (.source_dossier.workflow_control_commit | test("^[0-9a-f]{40}$")) and
    (.source_dossier.artifact_id | type == "number" and . > 0 and floor == .) and
    (.source_dossier.artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
    (.protected_authorization.workflow_run_id | type == "number" and . > 0 and floor == .) and
    (.protected_authorization.workflow_run_attempt | type == "number" and . > 0 and floor == .) and
    (.protected_authorization.workflow_control_commit | test("^[0-9a-f]{40}$")) and
    (.protected_authorization.artifact_id | type == "number" and . > 0 and floor == .) and
    (.protected_authorization.artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
    (.protected_authorization.evidence_sha256 | test("^[0-9a-f]{64}$")) and
    .publisher == $d.publisher and
    .endpoints == {
      github: ($d.endpoints.github + {prerelease: false}),
      pypi: $d.endpoints.pypi,
      npm: ($d.endpoints.npm + {
        dist_tag: (if .release.channel == "rc" then "rc" else "latest" end)
      }),
      homebrew: (($d.endpoints.homebrew | del(.formula_paths)) + {
        formula_path: $d.endpoints.homebrew.formula_paths[.release.channel]
      }),
      apt: (($d.endpoints.apt | del(.suites)) + {
        suite: $d.endpoints.apt.suites[.release.channel]
      }),
      winget: ({identity: $d.endpoints.winget.identity} +
        $d.endpoints.winget.routes[.release.channel]),
      oci: ($d.endpoints.oci + {
        immutable_tag: .release.version,
        moving_tag: (if .release.channel == "rc" then "rc" else "latest" end)
      })
    }
  ' "${config}" >/dev/null; then
    fail 65 'endpoint configuration differs from the reviewed static and dynamic routes'
  fi
  IFS=$'\t' read -r release_id version channel tag python_version < <(
    jq -er '[.release.id, .release.version, .release.channel, .release.tag, .release.python_version] | @tsv' "${config}"
  )
  validate_version_identity "${version}" "${channel}" "${tag}" "${python_version}"
  if [[ -n ${expected_release_id} && ${release_id} != "${expected_release_id}" ]]; then
    fail 65 'endpoint configuration release ID differs from the requested identity'
  fi
}

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-operator.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-operator.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected release-operator path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

require_gh() {
  command -v gh >/dev/null 2>&1 || fail 69 'gh is required for live workflow binding or dispatch'
  gh auth status >/dev/null 2>&1 || fail 77 'gh is not authenticated'
}

fetch_run_record() {
  local repository=$1
  local run_id=$2
  local output=$3
  require_gh
  gh api "repos/${repository}/actions/runs/${run_id}" >"${output}"
}

fetch_artifact_record() {
  local repository=$1
  local run_id=$2
  local artifact_name=$3
  local output=$4
  local listing=${temporary_root}/artifacts-${run_id}.json
  require_gh
  gh api "repos/${repository}/actions/runs/${run_id}/artifacts?per_page=100" >"${listing}"
  jq -e --arg name "${artifact_name}" '
    [.artifacts[] | select(.name == $name)] |
    if length == 1 then .[0] else error("artifact identity is not unique") end
  ' "${listing}" >"${output}"
}

fetch_single_prefixed_artifact_record() {
  local repository=$1
  local run_id=$2
  local prefix=$3
  local output=$4
  local listing=${temporary_root}/artifacts-${run_id}.json
  require_gh
  gh api "repos/${repository}/actions/runs/${run_id}/artifacts?per_page=100" >"${listing}"
  jq -e --arg prefix "${prefix}" '
    [.artifacts[] | select(.name | startswith($prefix))] |
    if length == 1 then .[0] else error("prefixed artifact identity is not unique") end
  ' "${listing}" >"${output}"
}

download_run_artifact() {
  local repository=$1
  local run_id=$2
  local artifact_name=$3
  local destination=$4
  require_gh
  mkdir "${destination}"
  gh run download "${run_id}" --repo "${repository}" \
    --name "${artifact_name}" --dir "${destination}"
}

validate_workflow_run() {
  local record=$1
  local expected_id=$2
  local expected_name=$3
  local expected_path=$4
  require_regular_file "${record}" 'workflow run record'
  jq -e \
    --argjson id "${expected_id}" \
    --arg name "${expected_name}" \
    --arg path "${expected_path}" '
      .id == $id and
      (.run_attempt | type == "number" and . > 0 and floor == .) and
      .name == $name and .path == $path and
      .event == "workflow_dispatch" and
      .status == "completed" and .conclusion == "success" and
      (.head_sha | test("^[0-9a-f]{40}$"))
    ' "${record}" >/dev/null || fail 65 "workflow run ${expected_id} is not the exact successful ${expected_name} run"
}

validate_artifact_record() {
  local record=$1
  local expected_id=$2
  local expected_name=$3
  local expected_run_id=$4
  require_regular_file "${record}" 'workflow artifact record'
  jq -e \
    --argjson id "${expected_id}" \
    --arg name "${expected_name}" \
    --argjson run_id "${expected_run_id}" '
      .id == $id and .name == $name and .expired == false and
      (.digest | test("^sha256:[0-9a-f]{64}$")) and
      .workflow_run.id == $run_id
    ' "${record}" >/dev/null || fail 65 "workflow artifact ${expected_id} is not the exact ${expected_name} artifact"
}

validate_pending_clean_host_status() {
  local config=$1
  local status=$2
  require_regular_file "${status}" 'operator publication status'
  jq -e --slurpfile config "${config}" '
    ($config[0]) as $c |
    .contract == "ait.release.operator.status/v1" and
    .status == "published_pending_clean_host_smoke" and
    .release == {
      id: $c.release.id,
      version: $c.release.version,
      tag: $c.release.tag
    } and
    .publication_workflow.conclusion == "success" and
    (.publication_workflow.run_id | type == "number" and . > 0 and floor == .) and
    (.publication_workflow.artifact_id | type == "number" and . > 0 and floor == .) and
    (.publication_workflow.artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
    .platforms.github == "published_and_read_back" and
    .platforms.pypi == "published_and_read_back" and
    .platforms.npm == "published_and_read_back" and
    .platforms.homebrew == "published_and_read_back" and
    .platforms.apt == "published_signed_and_read_back" and
    (.platforms.oci.server | test("^sha256:[0-9a-f]{64}$")) and
    (.platforms.oci.runner | test("^sha256:[0-9a-f]{64}$"))
  ' "${status}" >/dev/null ||
    fail 65 'operator status is not the exact publication pending clean-host smoke'
}

validate_prior_release() {
  local candidate=$1
  local channel=$2
  local prior=$3
  local prior_python=$4
  local candidate_base candidate_ordinal prior_base prior_ordinal expected_python
  if [[ ${channel} == rc ]]; then
    [[ ${candidate} =~ ^([0-9]+\.[0-9]+\.[0-9]+)-rc\.([1-9][0-9]*)$ ]] ||
      fail 65 'RC candidate version is not canonical'
    candidate_base=${BASH_REMATCH[1]}
    candidate_ordinal=${BASH_REMATCH[2]}
    [[ ${prior} =~ ^([0-9]+\.[0-9]+\.[0-9]+)-rc\.([1-9][0-9]*)$ ]] ||
      fail 65 'prior RC version is not canonical'
    prior_base=${BASH_REMATCH[1]}
    prior_ordinal=${BASH_REMATCH[2]}
    [[ ${prior_base} == "${candidate_base}" && ${prior_ordinal} -lt ${candidate_ordinal} ]] ||
      fail 65 'prior RC must use the same base and a lower positive ordinal'
    expected_python=${prior_base}rc${prior_ordinal}
  else
    [[ ${prior} =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[1-9][0-9]*)?$ ]] ||
      fail 65 'prior stable-line version is not canonical'
    [[ ${prior} != "${candidate}" ]] || fail 65 'prior version must differ from candidate'
    expected_python=${prior/-rc./rc}
  fi
  [[ ${prior_python} == "${expected_python}" ]] ||
    fail 65 'prior Python version does not match the exact prior release'
}

generate_clean_host_matrix() {
  local output=$1
  local family=${repo_root}/ait-release-family.json
  local platforms=${repo_root}/ci/native_bootstrap_matrix.json
  local tool=${repo_root}/ci/release_clean_host.mjs
  require_regular_file "${family}" 'clean-host family authority'
  require_regular_file "${platforms}" 'clean-host platform authority'
  require_regular_file "${tool}" 'clean-host matrix tool'
  node "${tool}" matrix --family "${family}" --platforms "${platforms}" \
    --output "${output}" >/dev/null
}

validate_clean_host_request() {
  local request=$1
  local config=$2
  local status=$3
  local config_sha status_sha matrix matrix_sha release_id version channel python_version tag source_commit
  require_regular_file "${request}" 'clean-host request'
  validate_endpoint_config "${config}"
  validate_pending_clean_host_status "${config}" "${status}"
  config_sha=$(sha256_file "${config}")
  status_sha=$(sha256_file "${status}")
  matrix=${temporary_root}/request-matrix.json
  generate_clean_host_matrix "${matrix}"
  matrix_sha=$(sha256_file "${matrix}")
  IFS=$'\t' read -r release_id version channel python_version tag source_commit < <(
    jq -er '[.release.id, .release.version, .release.channel, .release.python_version, .release.tag, .release.source_commit] | @tsv' "${config}"
  )
  if ! jq -e \
    --arg release_id "${release_id}" --arg version "${version}" \
    --arg channel "${channel}" --arg python_version "${python_version}" \
    --arg tag "${tag}" --arg source_commit "${source_commit}" \
    --arg config_sha "${config_sha}" --arg status_sha "${status_sha}" \
    --arg matrix_sha "${matrix_sha}" '
      .contract == "ait.release.operator.clean-host-request/v1" and
      .status == "ready_for_clean_host_matrix" and
      .release == {
        id: $release_id,
        version: $version,
        channel: $channel,
        python_version: $python_version,
        tag: $tag,
        source_commit: $source_commit
      } and
      .evidence == {
        endpoint_config_sha256: $config_sha,
        operator_status_sha256: $status_sha
      } and
      .prior.selector == "exact_immutable_version" and
      (.prior.version | type == "string") and
      .prior.tag == ("v" + .prior.version) and
      (.prior.python_version | type == "string") and
      .matrix == {
        contract: "ait.release.clean-host.matrix/v1",
        revision: "distribution-target-32-2026-08-17.1",
        row_count: 32,
        sha256: $matrix_sha,
        install_host_model: "fresh_github_hosted_vm_per_row",
        upgrade_host_model: "separate_fresh_github_hosted_vm_per_row"
      } and
      .workflow == {
        repository: "weita2026/ait-native",
        workflow: "ait-release-clean-host.yml",
        ref: "main",
        requested: .workflow.requested
      } and
      (.workflow.requested | type == "boolean") and
      .mutation == {
        artifact_rebuild: false,
        registry_write: false,
        endpoint_write: false,
        tag_write: false,
        ait_remote_release_activation: false
      }
    ' "${request}" >/dev/null; then
    fail 65 'clean-host request does not bind the exact endpoint status and 32-row matrix'
  fi
  validate_prior_release "${version}" "${channel}" \
    "$(jq -er '.prior.version' "${request}")" \
    "$(jq -er '.prior.python_version' "${request}")"
}

validate_clean_host_workflow_run() {
  local record=$1
  local expected_id=$2
  require_regular_file "${record}" 'clean-host workflow run record'
  jq -e --argjson id "${expected_id}" '
    .id == $id and
    (.run_attempt | type == "number" and . > 0 and floor == .) and
    .name == "ait release clean host" and
    .path == ".github/workflows/ait-release-clean-host.yml" and
    .event == "workflow_dispatch" and
    .status == "completed" and
    (.conclusion == "success" or .conclusion == "failure") and
    (.head_sha | test("^[0-9a-f]{40}$"))
  ' "${record}" >/dev/null ||
    fail 65 "workflow run ${expected_id} is not the exact terminal clean-host run"
}

mode=${1:-}
[[ -n ${mode} ]] || usage
shift

case "${mode}" in
  prepare)
    source_root=
    output=
    dispatch=false
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --source-root) [[ $# -ge 2 ]] || usage; source_root=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        --dispatch) dispatch=true; shift ;;
        *) usage ;;
      esac
    done
    [[ -n ${source_root} && -n ${output} ]] || usage
    require_real_directory "${source_root}" 'public source root'
    require_new_output "${output}" 'prepare output'
    source_root=$(canonical_directory "${source_root}")
    output=$(canonical_file "${output}")
    family=${source_root}/ait-release-family.json
    mapping=${source_root}/ait-monorepo-source.json
    build_entrypoint=${source_root}/build-release.mjs
    authorities=${source_root}/ci/release_repository_authorities.json
    platforms=${source_root}/ci/native_bootstrap_matrix.json
    for input in "${family}" "${mapping}" "${build_entrypoint}" "${authorities}" "${platforms}"; do
      require_regular_file "${input}" 'public release source input'
    done
    IFS=$'\t' read -r version channel tag python_version < <(release_identity_tsv "${family}")
    validate_version_identity "${version}" "${channel}" "${tag}" "${python_version}"
    source_commit=$(git -C "${source_root}" rev-parse HEAD)
    [[ ${source_commit} =~ ^[0-9a-f]{40}$ ]] || fail 65 'public source HEAD is not a full Git commit'
    [[ -z $(git -C "${source_root}" status --porcelain --untracked-files=all) ]] ||
      fail 65 'public source checkout is not clean'
    [[ $(git -C "${source_root}" cat-file -t "refs/tags/${tag}" 2>/dev/null) == tag ]] ||
      fail 65 'public release tag must be an annotated tag object'
    [[ $(git -C "${source_root}" rev-list -n 1 "refs/tags/${tag}" 2>/dev/null) == "${source_commit}" ]] ||
      fail 65 'public release tag does not resolve to the checked-out commit'
    node "${build_entrypoint}" --validate-only --git-commit "${source_commit}" >/dev/null
    family_sha=$(sha256_file "${family}")
    mapping_sha=$(sha256_file "${mapping}")
    coordinator_snapshot=$(jq -er '.coordinator_snapshot | select(test("^SNP-[0-9A-F]{12}$"))' "${mapping}")
    jq -e \
      --arg version "${version}" \
      --arg tag "${tag}" \
      --arg snapshot "${coordinator_snapshot}" \
      --arg family_sha "${family_sha}" '
        .schema == "ait.release.monorepo-source/v1" and
        .public_source_identity == "weita2026/ait-native" and
        .family_version == $version and .family_tag == $tag and
        .coordinator_snapshot == $snapshot and
        .family_manifest_sha256 == $family_sha and
        (.subtrees | length) == 5 and
        .git_commit_created == false and .public_publish == false
      ' "${mapping}" >/dev/null || fail 65 'public source mapping differs from the family identity'
    jq -e --arg version "${version}" '.family_version == $version and .public_publish == false' \
      "${authorities}" >/dev/null || fail 65 'release authority version differs from the public family'
    jq -e --arg version "${version}" '.version == $version and .public_publish == false' \
      "${platforms}" >/dev/null || fail 65 'native platform version differs from the public family'
    validate_defaults
    repository=$(jq -er '.publisher.repository' "${defaults}")
    jq -S -n \
      --arg version "${version}" \
      --arg channel "${channel}" \
      --arg python_version "${python_version}" \
      --arg tag "${tag}" \
      --arg repository "${repository}" \
      --arg source_commit "${source_commit}" \
      --arg coordinator_snapshot "${coordinator_snapshot}" \
      --arg family_sha "${family_sha}" \
      --arg mapping_sha "${mapping_sha}" \
      --argjson dispatch "${dispatch}" '
        {
          contract: "ait.release.operator.prepare/v1",
          status: "ready_for_component_receipts",
          release: {
            version: $version,
            channel: $channel,
            python_version: $python_version,
            tag: $tag,
            repository: $repository,
            source_commit: $source_commit,
            coordinator_snapshot: $coordinator_snapshot,
            family_manifest_sha256: $family_sha,
            source_mapping_sha256: $mapping_sha
          },
          receipt_dispatch: {
            workflow: "ait-release-component-receipts.yml",
            ref: "main",
            inputs: {
              coordinator_snapshot: $coordinator_snapshot,
              source_commit: $source_commit
            },
            requested: $dispatch
          },
          mutation: {
            registry_write: false,
            endpoint_repository_write: false,
            tag_write: false,
            artifact_rebuild: false,
            ait_remote_release_activation: false
          }
        }
      ' >"${output}"
    if [[ ${dispatch} == true ]]; then
      require_gh
      gh workflow run ait-release-component-receipts.yml \
        --repo "${repository}" --ref main \
        -f "coordinator_snapshot=${coordinator_snapshot}" \
        -f "source_commit=${source_commit}"
    fi
    printf '%s\n' "${output}"
    ;;

  bind-receipts)
    prepare=
    run_id=
    run_record=
    artifact_record=
    dossier_root=
    output=
    dispatch=false
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --prepare) [[ $# -ge 2 ]] || usage; prepare=$2; shift 2 ;;
        --run-id) [[ $# -ge 2 ]] || usage; run_id=$2; shift 2 ;;
        --run-record) [[ $# -ge 2 ]] || usage; run_record=$2; shift 2 ;;
        --artifact-record) [[ $# -ge 2 ]] || usage; artifact_record=$2; shift 2 ;;
        --dossier-root) [[ $# -ge 2 ]] || usage; dossier_root=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        --dispatch) dispatch=true; shift ;;
        *) usage ;;
      esac
    done
    [[ -n ${prepare} && -n ${output} ]] || usage
    require_regular_file "${prepare}" 'prepare record'
    require_new_output "${output}" 'receipt binding output'
    prepare=$(canonical_file "${prepare}")
    output=$(canonical_file "${output}")
    jq -e '
      .contract == "ait.release.operator.prepare/v1" and
      .status == "ready_for_component_receipts" and
      (.release.source_commit | test("^[0-9a-f]{40}$")) and
      (.release.coordinator_snapshot | test("^SNP-[0-9A-F]{12}$")) and
      ([.mutation[]] | all(. == false))
    ' "${prepare}" >/dev/null || fail 65 'prepare record is not an immutable release preparation'
    repository=$(jq -er '.release.repository' "${prepare}")
    if [[ -n ${run_id} ]]; then
      [[ ${run_id} =~ ^[1-9][0-9]*$ && -z ${run_record}${artifact_record}${dossier_root} ]] || usage
      run_record=${temporary_root}/receipt-run.json
      artifact_record=${temporary_root}/receipt-artifact.json
      fetch_run_record "${repository}" "${run_id}" "${run_record}"
      fetch_single_prefixed_artifact_record "${repository}" "${run_id}" \
        'ait-family-dossier-REL-FAM-' "${artifact_record}"
      artifact_name=$(jq -er '.name' "${artifact_record}")
      dossier_root=${temporary_root}/dossier
      download_run_artifact "${repository}" "${run_id}" "${artifact_name}" "${dossier_root}"
    else
      [[ -n ${run_record} && -n ${artifact_record} && -n ${dossier_root} ]] || usage
      run_id=$(jq -er '.id | select(type == "number" and . > 0 and floor == .)' "${run_record}")
    fi
    require_regular_file "${run_record}" 'component-receipt run record'
    require_regular_file "${artifact_record}" 'family dossier artifact record'
    require_real_directory "${dossier_root}" 'family dossier root'
    dossier_root=$(canonical_directory "${dossier_root}")
    validate_workflow_run "${run_record}" "${run_id}" \
      'ait release component receipts' '.github/workflows/ait-release-component-receipts.yml'
    candidate=${dossier_root}/ait-release.candidate.json
    promotion=${dossier_root}/ait-release.promotion.json
    source_mapping=${dossier_root}/ait-monorepo-source.json
    source_evidence=${dossier_root}/ait-public-git-source.evidence.json
    frozen_manifest=${dossier_root}/frozen/ait-release-family.manifest.json
    frozen_checksums=${dossier_root}/frozen/SHA256SUMS
    for input in "${candidate}" "${promotion}" "${source_mapping}" "${source_evidence}" \
      "${frozen_manifest}" "${frozen_checksums}"; do
      require_regular_file "${input}" 'family dossier binding input'
    done
    release_id=$(jq -er '.release_id | select(test("^REL-FAM-[0-9A-F]{16}$"))' "${candidate}")
    version=$(jq -er '.release.version' "${prepare}")
    channel=$(jq -er '.release.channel' "${prepare}")
    python_version=$(jq -er '.release.python_version' "${prepare}")
    tag=$(jq -er '.release.tag' "${prepare}")
    source_commit=$(jq -er '.release.source_commit' "${prepare}")
    coordinator_snapshot=$(jq -er '.release.coordinator_snapshot' "${prepare}")
    family_sha=$(jq -er '.release.family_manifest_sha256' "${prepare}")
    mapping_sha=$(jq -er '.release.source_mapping_sha256' "${prepare}")
    control_commit=$(jq -er '.head_sha' "${run_record}")
    run_attempt=$(jq -er '.run_attempt' "${run_record}")
    artifact_id=$(jq -er '.id' "${artifact_record}")
    artifact_digest=$(jq -er '.digest' "${artifact_record}")
    validate_artifact_record "${artifact_record}" "${artifact_id}" \
      "ait-family-dossier-${release_id}" "${run_id}"
    jq -e \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg tag "${tag}" \
      --arg snapshot "${coordinator_snapshot}" --arg family_sha "${family_sha}" '
        .contract == "ait.release.family.candidate/v1" and
        .release_id == $release_id and .version == $version and
        .channel == $channel and .tag == $tag and .snapshot_id == $snapshot and
        .family_manifest_sha256 == $family_sha
      ' "${candidate}" >/dev/null || fail 65 'family dossier candidate differs from preparation'
    jq -e \
      --arg version "${version}" --arg tag "${tag}" \
      --arg snapshot "${coordinator_snapshot}" --arg family_sha "${family_sha}" '
        .schema == "ait.release.monorepo-source/v1" and
        .family_version == $version and .family_tag == $tag and
        .coordinator_snapshot == $snapshot and .family_manifest_sha256 == $family_sha
      ' "${source_mapping}" >/dev/null || fail 65 'family dossier source mapping differs from preparation'
    [[ $(sha256_file "${source_mapping}") == "${mapping_sha}" ]] ||
      fail 65 'family dossier source mapping digest differs from preparation'
    jq -e \
      --arg repository "${repository}" --arg commit "${source_commit}" \
      --arg control_commit "${control_commit}" --arg snapshot "${coordinator_snapshot}" \
      --arg mapping_sha "${mapping_sha}" '
        .contract == "ait.release.public-git-source/v1" and .status == "ready" and
        .public_source_identity == $repository and .git_commit == $commit and
        .workflow_control_commit == $control_commit and
        .coordinator_snapshot == $snapshot and .mapping_sha256 == $mapping_sha and
        .registry_write == false and .public_publish == false
      ' "${source_evidence}" >/dev/null || fail 65 'public source evidence differs from the selected workflow run'
    jq -e \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg tag "${tag}" --arg snapshot "${coordinator_snapshot}" '
        .contract == "ait.release.family.frozen/v1" and
        .release_id == $release_id and .version == $version and
        .channel == $channel and .tag == $tag and .snapshot_id == $snapshot and
        .promotion.authorized == false and .promotion.registry_write == false
      ' "${frozen_manifest}" >/dev/null || fail 65 'frozen family manifest differs from preparation'
    jq -e \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg tag "${tag}" '
        .contract == "ait.release.family.promotion/v1" and
        .release_id == $release_id and .version == $version and
        .channel == $channel and .tag == $tag and
        .status == "ready_for_protected_ci" and
        .authorization.required == true and .authorization.granted == false and
        .mutation.performed == false and .mutation.registry_write == false
      ' "${promotion}" >/dev/null || fail 65 'promotion handoff is not awaiting exact protected approval'
    frozen_manifest_sha=$(sha256_file "${frozen_manifest}")
    frozen_checksums_sha=$(sha256_file "${frozen_checksums}")
    jq -S -n \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg python_version "${python_version}" \
      --arg tag "${tag}" --arg repository "${repository}" \
      --arg source_commit "${source_commit}" --arg snapshot "${coordinator_snapshot}" \
      --arg frozen_manifest_sha "${frozen_manifest_sha}" \
      --arg frozen_checksums_sha "${frozen_checksums_sha}" \
      --argjson run_id "${run_id}" --argjson run_attempt "${run_attempt}" \
      --arg control_commit "${control_commit}" --argjson artifact_id "${artifact_id}" \
      --arg artifact_digest "${artifact_digest}" --argjson dispatch "${dispatch}" '
        {
          contract: "ait.release.operator.receipt-binding/v1",
          status: "ready_for_protected_authorization",
          release: {
            id: $release_id,
            version: $version,
            channel: $channel,
            python_version: $python_version,
            tag: $tag,
            repository: $repository,
            source_commit: $source_commit,
            coordinator_snapshot: $snapshot,
            frozen_manifest_sha256: $frozen_manifest_sha,
            frozen_checksums_sha256: $frozen_checksums_sha
          },
          source_dossier: {
            workflow_run_id: $run_id,
            workflow_run_attempt: $run_attempt,
            workflow_control_commit: $control_commit,
            artifact_id: $artifact_id,
            artifact_digest: $artifact_digest
          },
          protected_dispatch: {
            workflow: "ait-release-protected-promotion.yml",
            ref: "main",
            inputs: {
              source_run_id: ($run_id | tostring),
              source_run_attempt: ($run_attempt | tostring),
              dossier_artifact_id: ($artifact_id | tostring),
              dossier_artifact_digest: $artifact_digest,
              release_id: $release_id,
              channel: $channel,
              tag: $tag,
              source_commit: $source_commit,
              source_control_commit: $control_commit,
              coordinator_snapshot: $snapshot,
              frozen_manifest_sha256: $frozen_manifest_sha,
              checksum_sha256: $frozen_checksums_sha
            },
            requested: $dispatch
          },
          mutation: {
            registry_write: false,
            endpoint_repository_write: false,
            tag_write: false,
            artifact_rebuild: false,
            ait_remote_release_activation: false
          }
        }
      ' >"${output}"
    if [[ ${dispatch} == true ]]; then
      require_gh
      gh workflow run ait-release-protected-promotion.yml \
        --repo "${repository}" --ref main \
        -f "source_run_id=${run_id}" \
        -f "source_run_attempt=${run_attempt}" \
        -f "dossier_artifact_id=${artifact_id}" \
        -f "dossier_artifact_digest=${artifact_digest}" \
        -f "release_id=${release_id}" \
        -f "channel=${channel}" \
        -f "tag=${tag}" \
        -f "source_commit=${source_commit}" \
        -f "source_control_commit=${control_commit}" \
        -f "coordinator_snapshot=${coordinator_snapshot}" \
        -f "frozen_manifest_sha256=${frozen_manifest_sha}" \
        -f "checksum_sha256=${frozen_checksums_sha}"
    fi
    printf '%s\n' "${output}"
    ;;

  bind-authorization)
    receipts=
    run_id=
    run_record=
    artifact_record=
    protected_evidence=
    output=
    dispatch=false
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --receipts) [[ $# -ge 2 ]] || usage; receipts=$2; shift 2 ;;
        --run-id) [[ $# -ge 2 ]] || usage; run_id=$2; shift 2 ;;
        --run-record) [[ $# -ge 2 ]] || usage; run_record=$2; shift 2 ;;
        --artifact-record) [[ $# -ge 2 ]] || usage; artifact_record=$2; shift 2 ;;
        --protected-evidence) [[ $# -ge 2 ]] || usage; protected_evidence=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        --dispatch) dispatch=true; shift ;;
        *) usage ;;
      esac
    done
    [[ -n ${receipts} && -n ${output} ]] || usage
    require_regular_file "${receipts}" 'receipt binding'
    require_new_output "${output}" 'endpoint configuration output'
    receipts=$(canonical_file "${receipts}")
    output=$(canonical_file "${output}")
    jq -e '
      .contract == "ait.release.operator.receipt-binding/v1" and
      .status == "ready_for_protected_authorization" and
      (.release.id | test("^REL-FAM-[0-9A-F]{16}$")) and
      ([.mutation[]] | all(. == false))
    ' "${receipts}" >/dev/null || fail 65 'receipt binding is not ready for protected authorization'
    repository=$(jq -er '.release.repository' "${receipts}")
    release_id=$(jq -er '.release.id' "${receipts}")
    if [[ -n ${run_id} ]]; then
      [[ ${run_id} =~ ^[1-9][0-9]*$ && -z ${run_record}${artifact_record}${protected_evidence} ]] || usage
      run_record=${temporary_root}/protected-run.json
      artifact_record=${temporary_root}/protected-artifact.json
      fetch_run_record "${repository}" "${run_id}" "${run_record}"
      fetch_artifact_record "${repository}" "${run_id}" \
        "ait-protected-promotion-${release_id}" "${artifact_record}"
      protected_root=${temporary_root}/protected
      download_run_artifact "${repository}" "${run_id}" \
        "ait-protected-promotion-${release_id}" "${protected_root}"
      protected_evidence=${protected_root}/ait-release.protected-promotion.json
    else
      [[ -n ${run_record} && -n ${artifact_record} && -n ${protected_evidence} ]] || usage
      run_id=$(jq -er '.id | select(type == "number" and . > 0 and floor == .)' "${run_record}")
    fi
    require_regular_file "${protected_evidence}" 'protected authorization evidence'
    validate_workflow_run "${run_record}" "${run_id}" \
      'ait release protected promotion' '.github/workflows/ait-release-protected-promotion.yml'
    artifact_id=$(jq -er '.id' "${artifact_record}")
    artifact_digest=$(jq -er '.digest' "${artifact_record}")
    validate_artifact_record "${artifact_record}" "${artifact_id}" \
      "ait-protected-promotion-${release_id}" "${run_id}"
    run_attempt=$(jq -er '.run_attempt' "${run_record}")
    control_commit=$(jq -er '.head_sha' "${run_record}")
    evidence_sha=$(sha256_file "${protected_evidence}")
    channel=$(jq -er '.release.channel' "${receipts}")
    expected_environment=${channel}-promotion
    if ! jq -e \
      --slurpfile receipts "${receipts}" \
      --argjson run_id "${run_id}" --argjson run_attempt "${run_attempt}" \
      --arg control_commit "${control_commit}" --arg environment "${expected_environment}" '
        ($receipts[0]) as $r |
        .contract == "ait.release.family.protected-promotion/v1" and
        .status == "authorized_for_explicit_endpoint_promotion" and
        .release_id == $r.release.id and .version == $r.release.version and
        .channel == $r.release.channel and .tag == $r.release.tag and
        .snapshot_id == $r.release.coordinator_snapshot and
        .public_source.repository == $r.release.repository and
        .public_source.git_commit == $r.release.source_commit and
        .public_source.status == "verified" and
        .dossier.source_run_id == ($r.source_dossier.workflow_run_id | tostring) and
        .dossier.source_run_attempt == ($r.source_dossier.workflow_run_attempt | tostring) and
        .dossier.source_workflow_sha == $r.source_dossier.workflow_control_commit and
        .dossier.artifact_id == ($r.source_dossier.artifact_id | tostring) and
        .dossier.artifact_digest == $r.source_dossier.artifact_digest and
        .dossier.frozen_manifest_sha256 == $r.release.frozen_manifest_sha256 and
        .dossier.checksum_sha256 == $r.release.frozen_checksums_sha256 and
        .dossier.native_promotion_readback_equal == true and
        .dossier.admission_replay.model == "immutable-tag-native-admission/v1" and
        (.dossier.admission_replay.rust_toolchain | test("^[0-9]+\\.[0-9]+\\.[0-9]+$")) and
        (.dossier.admission_replay.cargo_lock_sha256 | test("^[0-9a-f]{64}$")) and
        (.dossier.admission_replay.family_packages_sha256 | test("^[0-9a-f]{64}$")) and
        (.dossier.admission_replay.family_release_sha256 | test("^[0-9a-f]{64}$")) and
        .authorization.required == true and .authorization.granted == true and
        .authorization.exact_digest_approval == true and
        .authorization.boundary == "github_protected_environment" and
        .authorization.protected_environment == $environment and
        .authorization.workflow_run_id == ($run_id | tostring) and
        .authorization.workflow_run_attempt == ($run_attempt | tostring) and
        .authorization.workflow_sha == $control_commit and
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
      ' "${protected_evidence}" >/dev/null; then
      fail 65 'protected authorization evidence does not bind the exact receipt family and run'
    fi
    validate_defaults
    jq -S -n \
      --slurpfile receipts "${receipts}" --slurpfile defaults "${defaults}" \
      --argjson protected_run_id "${run_id}" \
      --argjson protected_run_attempt "${run_attempt}" \
      --arg protected_control_commit "${control_commit}" \
      --argjson protected_artifact_id "${artifact_id}" \
      --arg protected_artifact_digest "${artifact_digest}" \
      --arg protected_evidence_sha "${evidence_sha}" '
        ($receipts[0]) as $r |
        ($defaults[0]) as $d |
        {
          contract: "ait.release.family.endpoints/v1",
          release: {
            id: $r.release.id,
            version: $r.release.version,
            channel: $r.release.channel,
            python_version: $r.release.python_version,
            tag: $r.release.tag,
            source_commit: $r.release.source_commit,
            coordinator_snapshot: $r.release.coordinator_snapshot,
            frozen_manifest_sha256: $r.release.frozen_manifest_sha256,
            frozen_checksums_sha256: $r.release.frozen_checksums_sha256
          },
          source_dossier: $r.source_dossier,
          protected_authorization: {
            workflow_run_id: $protected_run_id,
            workflow_run_attempt: $protected_run_attempt,
            workflow_control_commit: $protected_control_commit,
            artifact_id: $protected_artifact_id,
            artifact_digest: $protected_artifact_digest,
            evidence_sha256: $protected_evidence_sha
          },
          publisher: $d.publisher,
          endpoints: {
            github: ($d.endpoints.github + {prerelease: false}),
            pypi: $d.endpoints.pypi,
            npm: ($d.endpoints.npm + {
              dist_tag: (if $r.release.channel == "rc" then "rc" else "latest" end)
            }),
            homebrew: (($d.endpoints.homebrew | del(.formula_paths)) + {
              formula_path: $d.endpoints.homebrew.formula_paths[$r.release.channel]
            }),
            apt: (($d.endpoints.apt | del(.suites)) + {
              suite: $d.endpoints.apt.suites[$r.release.channel]
            }),
            winget: ({identity: $d.endpoints.winget.identity} +
              $d.endpoints.winget.routes[$r.release.channel]),
            oci: ($d.endpoints.oci + {
              immutable_tag: $r.release.version,
              moving_tag: (if $r.release.channel == "rc" then "rc" else "latest" end)
            })
          }
        }
      ' >"${output}"
    validate_endpoint_config "${output}" "${release_id}"
    if [[ ${dispatch} == true ]]; then
      require_gh
      config_sha=$(sha256_file "${output}")
      config_b64=$(base64 <"${output}" | tr -d '\n')
      gh workflow run pypi-publish.yml \
        --repo "${repository}" --ref main \
        -f "release_id=${release_id}" \
        -f "protected_run_id=${run_id}" \
        -f "endpoint_config_sha256=${config_sha}" \
        -f "endpoint_config_b64=${config_b64}" \
        -f 'publish_exact_frozen_bytes=true'
    fi
    printf '%s\n' "${output}"
    ;;

  validate-config)
    config=
    expected_release_id=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --config) [[ $# -ge 2 ]] || usage; config=$2; shift 2 ;;
        --expected-release-id) [[ $# -ge 2 ]] || usage; expected_release_id=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${config} ]] || usage
    validate_endpoint_config "${config}" "${expected_release_id}"
    printf '%s\n' "$(canonical_file "${config}")"
    ;;

  status)
    config=
    run_id=
    run_record=
    artifact_record=
    evidence_root=
    output=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --config) [[ $# -ge 2 ]] || usage; config=$2; shift 2 ;;
        --run-id) [[ $# -ge 2 ]] || usage; run_id=$2; shift 2 ;;
        --run-record) [[ $# -ge 2 ]] || usage; run_record=$2; shift 2 ;;
        --artifact-record) [[ $# -ge 2 ]] || usage; artifact_record=$2; shift 2 ;;
        --evidence-root) [[ $# -ge 2 ]] || usage; evidence_root=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${config} && -n ${output} ]] || usage
    validate_endpoint_config "${config}"
    require_new_output "${output}" 'status output'
    output=$(canonical_file "${output}")
    repository=$(jq -er '.publisher.repository' "${config}")
    release_id=$(jq -er '.release.id' "${config}")
    if [[ -n ${run_id} ]]; then
      [[ ${run_id} =~ ^[1-9][0-9]*$ && -z ${run_record}${artifact_record}${evidence_root} ]] || usage
      run_record=${temporary_root}/endpoint-run.json
      artifact_record=${temporary_root}/endpoint-artifact.json
      fetch_run_record "${repository}" "${run_id}" "${run_record}"
      fetch_artifact_record "${repository}" "${run_id}" \
        "ait-endpoint-publication-${release_id}" "${artifact_record}"
      evidence_root=${temporary_root}/endpoint-evidence
      download_run_artifact "${repository}" "${run_id}" \
        "ait-endpoint-publication-${release_id}" "${evidence_root}"
    else
      [[ -n ${run_record} && -n ${artifact_record} && -n ${evidence_root} ]] || usage
      run_id=$(jq -er '.id | select(type == "number" and . > 0 and floor == .)' "${run_record}")
    fi
    require_real_directory "${evidence_root}" 'endpoint evidence root'
    evidence_root=$(canonical_directory "${evidence_root}")
    validate_workflow_run "${run_record}" "${run_id}" \
      'ait release endpoint publication' '.github/workflows/pypi-publish.yml'
    artifact_id=$(jq -er '.id' "${artifact_record}")
    validate_artifact_record "${artifact_record}" "${artifact_id}" \
      "ait-endpoint-publication-${release_id}" "${run_id}"
    readback=${evidence_root}/ait-release.endpoint-readback.json
    for receipt in "${readback}" "${evidence_root}/github.json" "${evidence_root}/pypi.json" \
      "${evidence_root}/npm.json" "${evidence_root}/homebrew.json" \
      "${evidence_root}/apt.json" "${evidence_root}/oci-state.json"; do
      require_regular_file "${receipt}" 'endpoint status evidence'
    done
    channel=$(jq -er '.release.channel' "${config}")
    if [[ ${channel} == rc ]]; then
      winget_status=validation_assets_published_no_community_submission
    else
      winget_status=community_manifest_assets_published_submission_required
    fi
    jq -e --slurpfile config "${config}" --arg winget "${winget_status}" '
      ($config[0]) as $c |
      .contract == "ait.release.family.endpoint-readback/v1" and
      .status == "published_pending_clean_host_smoke" and
      .release_id == $c.release.id and .version == $c.release.version and
      .tag == $c.release.tag and
      .endpoints.github == "published_and_read_back" and
      .endpoints.pypi == "published_and_read_back" and
      .endpoints.npm == "published_and_read_back" and
      .endpoints.homebrew == "published_and_read_back" and
      .endpoints.apt == "published_signed_and_read_back" and
      .endpoints.winget == $winget and
      (.endpoints.oci.server | test("^sha256:[0-9a-f]{64}$")) and
      (.endpoints.oci.runner | test("^sha256:[0-9a-f]{64}$")) and
      .endpoints.oci.immutable_tag == $c.endpoints.oci.immutable_tag and
      .endpoints.oci.moving_tag == $c.endpoints.oci.moving_tag
    ' "${readback}" >/dev/null || fail 65 'endpoint readback evidence is incomplete or belongs to another release'
    jq -e --arg release_id "${release_id}" '
      .contract == "ait.release.endpoint.apt/v1" and .release_id == $release_id and
      .status == "published_signed_and_read_back" and .signature_readback == true and
      .package_digest_readback == true and .apt_cache_search == true
    ' "${evidence_root}/apt.json" >/dev/null || fail 65 'APT status does not prove signed apt-cache searchability'
    jq -S -n \
      --slurpfile readback "${readback}" \
      --argjson run_id "${run_id}" --argjson artifact_id "${artifact_id}" \
      --arg artifact_digest "$(jq -er '.digest' "${artifact_record}")" '
        {
          contract: "ait.release.operator.status/v1",
          status: $readback[0].status,
          release: {
            id: $readback[0].release_id,
            version: $readback[0].version,
            tag: $readback[0].tag
          },
          publication_workflow: {
            run_id: $run_id,
            artifact_id: $artifact_id,
            artifact_digest: $artifact_digest,
            conclusion: "success"
          },
          platforms: $readback[0].endpoints,
          next_action: $readback[0].next_action
        }
      ' >"${output}"
    printf '%s\n' "${output}"
    ;;

  clean-host)
    config=
    status=
    prior_version=
    prior_python_version=
    output=
    dispatch=false
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --config) [[ $# -ge 2 ]] || usage; config=$2; shift 2 ;;
        --status) [[ $# -ge 2 ]] || usage; status=$2; shift 2 ;;
        --prior-version) [[ $# -ge 2 ]] || usage; prior_version=$2; shift 2 ;;
        --prior-python-version) [[ $# -ge 2 ]] || usage; prior_python_version=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        --dispatch) dispatch=true; shift ;;
        *) usage ;;
      esac
    done
    [[ -n ${config} && -n ${status} && -n ${prior_version} &&
      -n ${prior_python_version} && -n ${output} ]] || usage
    validate_endpoint_config "${config}"
    validate_pending_clean_host_status "${config}" "${status}"
    require_new_output "${output}" 'clean-host request output'
    config=$(canonical_file "${config}")
    status=$(canonical_file "${status}")
    output=$(canonical_file "${output}")
    IFS=$'\t' read -r release_id version channel python_version tag source_commit < <(
      jq -er '[.release.id, .release.version, .release.channel, .release.python_version, .release.tag, .release.source_commit] | @tsv' "${config}"
    )
    validate_prior_release "${version}" "${channel}" \
      "${prior_version}" "${prior_python_version}"
    matrix=${temporary_root}/clean-host-matrix.json
    generate_clean_host_matrix "${matrix}"
    matrix_sha=$(sha256_file "${matrix}")
    jq -S -n \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg python_version "${python_version}" \
      --arg tag "${tag}" --arg source_commit "${source_commit}" \
      --arg config_sha "$(sha256_file "${config}")" \
      --arg status_sha "$(sha256_file "${status}")" \
      --arg prior_version "${prior_version}" \
      --arg prior_python_version "${prior_python_version}" \
      --arg matrix_sha "${matrix_sha}" --argjson dispatch "${dispatch}" '
        {
          contract: "ait.release.operator.clean-host-request/v1",
          status: "ready_for_clean_host_matrix",
          release: {
            id: $release_id,
            version: $version,
            channel: $channel,
            python_version: $python_version,
            tag: $tag,
            source_commit: $source_commit
          },
          evidence: {
            endpoint_config_sha256: $config_sha,
            operator_status_sha256: $status_sha
          },
          prior: {
            version: $prior_version,
            python_version: $prior_python_version,
            tag: ("v" + $prior_version),
            selector: "exact_immutable_version"
          },
          matrix: {
            contract: "ait.release.clean-host.matrix/v1",
            revision: "distribution-target-32-2026-08-17.1",
            row_count: 32,
            sha256: $matrix_sha,
            install_host_model: "fresh_github_hosted_vm_per_row",
            upgrade_host_model: "separate_fresh_github_hosted_vm_per_row"
          },
          workflow: {
            repository: "weita2026/ait-native",
            workflow: "ait-release-clean-host.yml",
            ref: "main",
            requested: $dispatch
          },
          mutation: {
            artifact_rebuild: false,
            registry_write: false,
            endpoint_write: false,
            tag_write: false,
            ait_remote_release_activation: false
          }
        }
      ' >"${output}"
    validate_clean_host_request "${output}" "${config}" "${status}"
    if [[ ${dispatch} == true ]]; then
      require_gh
      request_sha=$(sha256_file "${output}")
      request_b64=$(base64 <"${output}" | tr -d '\n')
      config_b64=$(base64 <"${config}" | tr -d '\n')
      status_b64=$(base64 <"${status}" | tr -d '\n')
      gh workflow run ait-release-clean-host.yml \
        --repo 'weita2026/ait-native' --ref main \
        -f "release_id=${release_id}" \
        -f "request_sha256=${request_sha}" \
        -f "request_b64=${request_b64}" \
        -f "endpoint_config_sha256=$(sha256_file "${config}")" \
        -f "endpoint_config_b64=${config_b64}" \
        -f "operator_status_sha256=$(sha256_file "${status}")" \
        -f "operator_status_b64=${status_b64}" \
        -f "source_commit=${source_commit}" \
        -f "tag=${tag}" \
        -f "prior_version=${prior_version}" \
        -f "prior_python_version=${prior_python_version}" \
        -f "matrix_sha256=${matrix_sha}"
    fi
    printf '%s\n' "${output}"
    ;;

  validate-clean-host-request)
    request=
    config=
    status=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --request) [[ $# -ge 2 ]] || usage; request=$2; shift 2 ;;
        --config) [[ $# -ge 2 ]] || usage; config=$2; shift 2 ;;
        --status) [[ $# -ge 2 ]] || usage; status=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${request} && -n ${config} && -n ${status} ]] || usage
    validate_clean_host_request "${request}" "${config}" "${status}"
    printf '%s\n' "$(canonical_file "${request}")"
    ;;

  clean-host-status)
    request=
    config=
    status=
    run_id=
    run_record=
    artifact_record=
    evidence_root=
    output=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --request) [[ $# -ge 2 ]] || usage; request=$2; shift 2 ;;
        --config) [[ $# -ge 2 ]] || usage; config=$2; shift 2 ;;
        --status) [[ $# -ge 2 ]] || usage; status=$2; shift 2 ;;
        --run-id) [[ $# -ge 2 ]] || usage; run_id=$2; shift 2 ;;
        --run-record) [[ $# -ge 2 ]] || usage; run_record=$2; shift 2 ;;
        --artifact-record) [[ $# -ge 2 ]] || usage; artifact_record=$2; shift 2 ;;
        --evidence-root) [[ $# -ge 2 ]] || usage; evidence_root=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${request} && -n ${config} && -n ${status} && -n ${output} ]] || usage
    validate_clean_host_request "${request}" "${config}" "${status}"
    require_new_output "${output}" 'clean-host status output'
    output=$(canonical_file "${output}")
    repository=$(jq -er '.workflow.repository' "${request}")
    release_id=$(jq -er '.release.id' "${request}")
    if [[ -n ${run_id} ]]; then
      [[ ${run_id} =~ ^[1-9][0-9]*$ && -z ${run_record}${artifact_record}${evidence_root} ]] || usage
      run_record=${temporary_root}/clean-host-run.json
      artifact_record=${temporary_root}/clean-host-artifact.json
      fetch_run_record "${repository}" "${run_id}" "${run_record}"
      fetch_artifact_record "${repository}" "${run_id}" \
        "ait-clean-host-${release_id}" "${artifact_record}"
      evidence_root=${temporary_root}/clean-host-evidence
      download_run_artifact "${repository}" "${run_id}" \
        "ait-clean-host-${release_id}" "${evidence_root}"
    else
      [[ -n ${run_record} && -n ${artifact_record} && -n ${evidence_root} ]] || usage
      run_id=$(jq -er '.id | select(type == "number" and . > 0 and floor == .)' "${run_record}")
    fi
    require_real_directory "${evidence_root}" 'clean-host evidence root'
    evidence_root=$(canonical_directory "${evidence_root}")
    validate_clean_host_workflow_run "${run_record}" "${run_id}"
    artifact_id=$(jq -er '.id' "${artifact_record}")
    validate_artifact_record "${artifact_record}" "${artifact_id}" \
      "ait-clean-host-${release_id}" "${run_id}"
    aggregate=${evidence_root}/ait-release.clean-host-status.json
    checksums=${evidence_root}/SHA256SUMS
    rows_root=${evidence_root}/rows
    require_regular_file "${aggregate}" 'clean-host aggregate status'
    require_regular_file "${checksums}" 'clean-host checksum inventory'
    require_real_directory "${rows_root}" 'clean-host row evidence directory'
    if find "${evidence_root}" -type l -print -quit | grep -q .; then
      fail 65 'clean-host evidence contains a symbolic link'
    fi
    closeout_matrix=${temporary_root}/clean-host-closeout-matrix.json
    generate_clean_host_matrix "${closeout_matrix}"
    [[ $(sha256_file "${closeout_matrix}") == $(jq -er '.matrix.sha256' "${request}") ]] ||
      fail 65 'clean-host closeout matrix differs from the dispatched request'
    verified_root=${temporary_root}/clean-host-verified
    if node "${repo_root}/ci/release_clean_host.mjs" aggregate \
      --matrix "${closeout_matrix}" --config "${config}" --status "${status}" \
      --evidence-root "${rows_root}" --output-root "${verified_root}" >/dev/null 2>&1; then
      verified_status=published
    else
      verified_status=blocked
    fi
    cmp "${aggregate}" "${verified_root}/ait-release.clean-host-status.json" >/dev/null ||
      fail 65 'clean-host aggregate differs from deterministic row reaggregation'
    cmp "${checksums}" "${verified_root}/SHA256SUMS" >/dev/null ||
      fail 65 'clean-host checksum inventory differs from deterministic reaggregation'
    inventory_count=$(jq -er '.evidence_inventory | length' "${aggregate}")
    checksum_count=0
    while IFS= read -r line || [[ -n ${line} ]]; do
      [[ ${line} =~ ^([0-9a-f]{64})\ \ (.+)$ ]] ||
        fail 65 'clean-host checksum inventory contains a malformed row'
      digest=${BASH_REMATCH[1]}
      relative=${BASH_REMATCH[2]}
      case "${relative}" in
        ait-release.clean-host-status.json | rows/*.json) ;;
        *) fail 65 "clean-host checksum path is unsafe or unexpected: ${relative}" ;;
      esac
      require_regular_file "${evidence_root}/${relative}" 'clean-host checksummed evidence'
      [[ $(sha256_file "${evidence_root}/${relative}") == "${digest}" ]] ||
        fail 65 "clean-host evidence checksum drifted: ${relative}"
      checksum_count=$((checksum_count + 1))
    done <"${checksums}"
    [[ ${checksum_count} -eq $((inventory_count + 1)) ]] ||
      fail 65 'clean-host checksum inventory cardinality differs from the aggregate'
    actual_file_count=$(find "${evidence_root}" -type f | wc -l | tr -d '[:space:]')
    [[ ${actual_file_count} -eq $((inventory_count + 2)) ]] ||
      fail 65 'clean-host artifact contains unlisted or missing files'
    request_sha=$(sha256_file "${request}")
    config_sha=$(sha256_file "${config}")
    status_sha=$(sha256_file "${status}")
    aggregate_status=$(jq -er '.status' "${aggregate}")
    [[ ${aggregate_status} == "${verified_status}" ]] ||
      fail 65 'clean-host aggregate status differs from deterministic verification'
    conclusion=$(jq -er '.conclusion' "${run_record}")
    jq -e \
      --slurpfile request "${request}" \
      --arg config_sha "${config_sha}" --arg status_sha "${status_sha}" '
        ($request[0]) as $r |
        .contract == "ait.release.clean-host.aggregate/v1" and
        (.status == "published" or .status == "blocked") and
        .release == {
          id: $r.release.id,
          version: $r.release.version,
          python_version: $r.release.python_version,
          channel: $r.release.channel,
          tag: $r.release.tag,
          source_commit: $r.release.source_commit,
          endpoint_config_sha256: $config_sha,
          operator_status_sha256: $status_sha
        } and
        .matrix.revision == $r.matrix.revision and
        .matrix.expected_rows == 32 and
        .matrix.evidence_files == (.evidence_inventory | length) and
        ([.evidence_inventory[].path] | unique | length) == (.evidence_inventory | length) and
        all(.evidence_inventory[];
          (.path | test("^rows/[a-z0-9_@.-]+\\.json$")) and
          (.sha256 | test("^[0-9a-f]{64}$"))) and
        if .status == "published" then
          .matrix.admitted_rows == 32 and .matrix.evidence_files == 32 and
          .failures == [] and
          .promotion == {allowed: true, terminal_for_release: false}
        else
          (.failures | type == "array" and length > 0) and
          .promotion == {allowed: false, terminal_for_release: true}
        end
      ' "${aggregate}" >/dev/null ||
      fail 65 'clean-host aggregate does not bind the exact release or terminal decision'
    if [[ ${aggregate_status} == published ]]; then
      [[ ${conclusion} == success ]] ||
        fail 65 'published clean-host aggregate came from a non-successful workflow'
      next_action=release_complete
    else
      [[ ${conclusion} == failure ]] ||
        fail 65 'blocked clean-host aggregate must preserve the failed workflow result'
      next_action=freeze_new_release_after_repair
    fi
    jq -S -n \
      --slurpfile request "${request}" --slurpfile aggregate "${aggregate}" \
      --slurpfile endpoint_status "${status}" \
      --arg status "${aggregate_status}" --arg next_action "${next_action}" \
      --arg request_sha "${request_sha}" \
      --arg endpoint_status_sha "${status_sha}" \
      --argjson run_id "${run_id}" \
      --argjson run_attempt "$(jq -er '.run_attempt' "${run_record}")" \
      --arg control_commit "$(jq -er '.head_sha' "${run_record}")" \
      --arg conclusion "${conclusion}" \
      --argjson artifact_id "${artifact_id}" \
      --arg artifact_digest "$(jq -er '.digest' "${artifact_record}")" \
      --arg aggregate_sha "$(sha256_file "${aggregate}")" \
      --arg checksums_sha "$(sha256_file "${checksums}")" '
        {
          contract: "ait.release.operator.clean-host-status/v1",
          status: $status,
          release: ($request[0].release + {
            endpoint_config_sha256: $request[0].evidence.endpoint_config_sha256,
            operator_status_sha256: $request[0].evidence.operator_status_sha256
          }),
          clean_host_request_sha256: $request_sha,
          endpoint_publication: {
            operator_status_sha256: $endpoint_status_sha,
            workflow: $endpoint_status[0].publication_workflow,
            platforms: $endpoint_status[0].platforms
          },
          clean_host_workflow: {
            run_id: $run_id,
            run_attempt: $run_attempt,
            control_commit: $control_commit,
            conclusion: $conclusion,
            artifact_id: $artifact_id,
            artifact_digest: $artifact_digest
          },
          evidence: {
            aggregate_sha256: $aggregate_sha,
            checksum_inventory_sha256: $checksums_sha,
            rows: $aggregate[0].matrix
          },
          failures: $aggregate[0].failures,
          promotion_allowed: $aggregate[0].promotion.allowed,
          terminal_for_release: $aggregate[0].promotion.terminal_for_release,
          next_action: $next_action
        }
      ' >"${output}"
    printf '%s\n' "${output}"
    ;;

  *) usage ;;
esac
