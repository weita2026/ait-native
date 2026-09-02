#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
defaults=${repo_root}/release/endpoint-publication.defaults.json

usage() {
  cat >&2 <<'USAGE'
usage:
  release_operator.sh bind-qualification --source-root <absolute-dir> --run-id <id> --output <absolute-json>
  release_operator.sh bind-qualification --source-root <absolute-dir> --run-record <json> --artifact-record <json> --evidence-root <absolute-dir> --output <absolute-json>
  release_operator.sh admit --source-root <absolute-dir> --qualification <json> --output <absolute-json>
  release_operator.sh prepare --source-root <absolute-dir> --admission <json> --prior-version <semver> --prior-python-version <pep440> --output <absolute-json> [--dispatch]
  release_operator.sh bind-receipts --prepare <json> --run-id <id> --output <absolute-json> [--dispatch]
  release_operator.sh bind-receipts --prepare <json> --run-record <json> --artifact-record <json> --dossier-root <absolute-dir> --output <absolute-json> [--dispatch]
  release_operator.sh bind-candidate --source-root <absolute-dir> --receipts <json> --run-id <id> --output <absolute-json>
  release_operator.sh bind-candidate --source-root <absolute-dir> --receipts <json> --run-record <json> --candidate-artifact-record <json> --candidate-root <absolute-dir> --aggregate-artifact-record <json> --aggregate-root <absolute-dir> --output <absolute-json>
  release_operator.sh authorize --source-root <absolute-dir> --candidate <json> --output <absolute-json> [--dispatch]
  release_operator.sh bind-authorization --receipts <json> --run-id <id> --output <absolute-json> [--dispatch]
  release_operator.sh bind-authorization --receipts <json> --run-record <json> --artifact-record <json> --protected-evidence <json> --output <absolute-json> [--dispatch]
  release_operator.sh validate-config --config <json> [--expected-release-id <REL-FAM-id>]
  release_operator.sh validate-candidate-config --config <json> [--expected-release-id <REL-FAM-id>]
  release_operator.sh status --config <json> --run-id <id> --output <absolute-json>
  release_operator.sh status --config <json> --run-record <json> --artifact-record <json> --evidence-root <absolute-dir> --output <absolute-json>

Qualification and admission are mandatory and non-publishing. Admission,
component receipts, frozen candidate creation, and the complete clean-host
install/upgrade matrix must finish while the exact release commit is still
untagged. Only a successful bound candidate qualification may advance to
immutable tag creation and protected publishing authorization.
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

validate_pretag_native_bundle() {
  local family=$1
  local version=$2
  local major minor
  [[ ${version} =~ ^([0-9]+)\.([0-9]+)\. ]] ||
    fail 65 'release version is not canonical SemVer'
  major=${BASH_REMATCH[1]}
  minor=${BASH_REMATCH[2]}
  if ((major < 1 || (major == 1 && minor < 1))); then
    return
  fi
  jq -e '
    def exact_runner_bundle:
      (.components | length) == 3 and
      (.components | sort) == ["ait", "ait-runner", "ait-server"];
    ([.distributions[]? |
      select(
        .role == "product" and
        (.channel == "homebrew" or .channel == "apt" or .channel == "winget")
      )] | length) == 3 and
    ([.distributions[]? |
      select(
        .role == "product" and
        (.channel == "homebrew" or .channel == "apt" or .channel == "winget")
      ) | .channel] | sort) == ["apt", "homebrew", "winget"] and
    ([.distributions[]? |
      select(
        .role == "product" and
        (.channel == "homebrew" or .channel == "apt" or .channel == "winget")
      )] | all(exact_runner_bundle))
  ' "${family}" >/dev/null ||
    fail 65 'pre-tag admission requires exact ait, ait-server, and ait-runner product bundles for Homebrew, apt, and WinGet on 1.1+'
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
    (.pre_tag_qualification.workflow_run_id | type == "number" and . > 0 and floor == .) and
    (.pre_tag_qualification.workflow_run_attempt | type == "number" and . > 0 and floor == .) and
    (.pre_tag_qualification.workflow_control_commit | test("^[0-9a-f]{40}$")) and
    (.pre_tag_qualification.candidate_artifact_id | type == "number" and . > 0 and floor == .) and
    (.pre_tag_qualification.candidate_artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
    (.pre_tag_qualification.candidate_status_sha256 | test("^[0-9a-f]{64}$")) and
    (.pre_tag_qualification.aggregate_artifact_id | type == "number" and . > 0 and floor == .) and
    (.pre_tag_qualification.aggregate_artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
    (.pre_tag_qualification.aggregate_status_sha256 | test("^[0-9a-f]{64}$")) and
    .pre_tag_qualification.clean_host_rows == 32 and
    .pre_tag_qualification.tag_state_at_closeout == "absent" and
    (.protected_authorization.workflow_run_id | type == "number" and . > 0 and floor == .) and
    (.protected_authorization.workflow_run_attempt | type == "number" and . > 0 and floor == .) and
    (.protected_authorization.workflow_control_commit | test("^[0-9a-f]{40}$")) and
    (.protected_authorization.artifact_id | type == "number" and . > 0 and floor == .) and
    (.protected_authorization.artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
    (.protected_authorization.evidence_sha256 | test("^[0-9a-f]{64}$")) and
    .publisher == $d.publisher and
    .endpoints == {
      github: ($d.endpoints.github + {
        prerelease: (.release.channel == "rc")
      }),
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

validate_candidate_config() {
  local config=$1
  local expected_release_id=${2:-}
  local version channel tag python_version release_id prior_version prior_python_version
  require_regular_file "${config}" 'pre-tag candidate authority'
  validate_defaults
  if ! jq -e --slurpfile defaults "${defaults}" '
    ($defaults[0]) as $d |
    .contract == "ait.release.family.pre-tag-candidate-authority/v1" and
    .status == "ready_for_pre_tag_clean_host" and
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
    (.pre_rc_admission.admission_sha256 | test("^[0-9a-f]{64}$")) and
    .tag_authority == {
      required_state: "absent",
      remote_readback_required_at_start: true,
      remote_readback_required_at_closeout: true
    } and
    (.frozen_checksum_count | type == "number" and . > 0 and floor == .) and
    (.packages | type == "array" and length == 5) and
    ([.packages[].channel] | sort) == ["apt", "homebrew", "npm", "pypi", "winget"] and
    all(.packages[];
      (.receipt_sha256 | test("^[0-9a-f]{64}$")) and
      (.checksum_sha256 | test("^[0-9a-f]{64}$")) and
      (.artifact_count | type == "number" and . > 0 and floor == .)) and
    .prior_release == {
      version: .prior_release.version,
      python_version: .prior_release.python_version
    } and
    .publisher == $d.publisher and
    .endpoints == {
      github: ($d.endpoints.github + {
        prerelease: (.release.channel == "rc")
      }),
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
    } and
    .mutation == {
      artifact_rebuild: false,
      component_rebuild: false,
      endpoint_repository_write: false,
      registry_write: false,
      tag_write: false
    }
  ' "${config}" >/dev/null; then
    fail 65 'pre-tag candidate authority differs from the exact frozen dossier and reviewed routes'
  fi
  IFS=$'\t' read -r release_id version channel tag python_version prior_version prior_python_version < <(
    jq -er '[.release.id, .release.version, .release.channel, .release.tag,
      .release.python_version, .prior_release.version,
      .prior_release.python_version] | @tsv' "${config}"
  )
  validate_version_identity "${version}" "${channel}" "${tag}" "${python_version}"
  validate_prior_release \
    "${version}" "${channel}" "${prior_version}" "${prior_python_version}"
  if [[ -n ${expected_release_id} && ${release_id} != "${expected_release_id}" ]]; then
    fail 65 'pre-tag candidate authority release ID differs from the requested identity'
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

validate_pre_rc_qualification_evidence() {
  local evidence=$1
  local source_commit=$2
  local control_commit=$3
  require_regular_file "${evidence}" 'pre-RC qualification evidence'
  jq -e \
    --arg source_commit "${source_commit}" \
    --arg control_commit "${control_commit}" '
      .contract == "ait.release.pre-rc-qualification/v1" and
      .status == "qualified" and
      .source == {
        repository: "weita2026/ait-native",
        git_commit: $source_commit,
        workflow_control_commit: $control_commit,
        release_tags_at_qualification: []
      } and
      .gates.core_quality == "pass" and
      .gates.release_controls == "pass" and
      .gates.command_inventory == "pass" and
      .gates.environment_inventory == "pass" and
      .gates.windows == {
        "aarch64-pc-windows-msvc": {
          core_init: "pass",
          server_init: "pass",
          server_run: "pass"
        },
        "x86_64-pc-windows-msvc": {
          core_init: "pass",
          server_init: "pass",
          server_run: "pass"
        }
      } and
      .immutable_release_tag_created == false and
      .release_receipts_created == false and
      .public_endpoint_writes == false
    ' "${evidence}" >/dev/null ||
    fail 65 'pre-RC qualification evidence is incomplete or inconsistent'
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

mode=${1:-}
[[ -n ${mode} ]] || usage
shift

case "${mode}" in
  bind-qualification)
    source_root=
    run_id=
    run_record=
    artifact_record=
    evidence_root=
    output=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --source-root) [[ $# -ge 2 ]] || usage; source_root=$2; shift 2 ;;
        --run-id) [[ $# -ge 2 ]] || usage; run_id=$2; shift 2 ;;
        --run-record) [[ $# -ge 2 ]] || usage; run_record=$2; shift 2 ;;
        --artifact-record) [[ $# -ge 2 ]] || usage; artifact_record=$2; shift 2 ;;
        --evidence-root) [[ $# -ge 2 ]] || usage; evidence_root=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${source_root} && -n ${output} ]] || usage
    require_real_directory "${source_root}" 'qualified public source root'
    require_new_output "${output}" 'qualification binding output'
    source_root=$(canonical_directory "${source_root}")
    output=$(canonical_file "${output}")
    source_commit=$(git -C "${source_root}" rev-parse HEAD)
    [[ ${source_commit} =~ ^[0-9a-f]{40}$ ]] ||
      fail 65 'qualified public source HEAD is not a full Git commit'
    [[ -z $(git -C "${source_root}" status --porcelain --untracked-files=all) ]] ||
      fail 65 'qualified public source checkout is not clean'
    if git -C "${source_root}" tag --points-at "${source_commit}" | grep -Eq '^v[0-9]'; then
      fail 65 'qualified repair commit already has a release tag'
    fi
    node "${source_root}/build-release.mjs" \
      --validate-only --git-commit "${source_commit}" >/dev/null
    repository=$(jq -er '.public_source.identity | select(. == "weita2026/ait-native")' \
      "${source_root}/ait-release-family.json")
    artifact_name="ait-pre-rc-qualification-${source_commit}"
    if [[ -n ${run_id} ]]; then
      [[ ${run_id} =~ ^[1-9][0-9]*$ && -z ${run_record}${artifact_record}${evidence_root} ]] || usage
      run_record=${temporary_root}/qualification-run.json
      artifact_record=${temporary_root}/qualification-artifact.json
      fetch_run_record "${repository}" "${run_id}" "${run_record}"
      fetch_artifact_record "${repository}" "${run_id}" \
        "${artifact_name}" "${artifact_record}"
      evidence_root=${temporary_root}/qualification-evidence
      download_run_artifact "${repository}" "${run_id}" \
        "${artifact_name}" "${evidence_root}"
    else
      [[ -n ${run_record} && -n ${artifact_record} && -n ${evidence_root} ]] || usage
      run_id=$(jq -er '.id | select(type == "number" and . > 0 and floor == .)' \
        "${run_record}")
    fi
    require_real_directory "${evidence_root}" 'pre-RC qualification evidence root'
    validate_workflow_run "${run_record}" "${run_id}" \
      'ait pre-RC qualification' \
      '.github/workflows/ait-release-pre-rc-qualification.yml'
    control_commit=$(jq -er '.head_sha' "${run_record}")
    [[ ${control_commit} == "${source_commit}" ]] ||
      fail 65 'pre-RC qualification workflow did not run from the repair commit'
    validate_artifact_record "${artifact_record}" \
      "$(jq -er '.id' "${artifact_record}")" "${artifact_name}" "${run_id}"
    evidence=${evidence_root}/ait-release.pre-rc-qualification.json
    validate_pre_rc_qualification_evidence \
      "${evidence}" "${source_commit}" "${control_commit}"
    evidence_sha=$(sha256_file "${evidence}")
    artifact_id=$(jq -er '.id' "${artifact_record}")
    artifact_digest=$(jq -er '.digest' "${artifact_record}")
    run_attempt=$(jq -er '.run_attempt' "${run_record}")
    jq -S -n \
      --arg repository "${repository}" \
      --arg source_commit "${source_commit}" \
      --arg control_commit "${control_commit}" \
      --arg evidence_sha "${evidence_sha}" \
      --arg artifact_digest "${artifact_digest}" \
      --argjson run_id "${run_id}" \
      --argjson run_attempt "${run_attempt}" \
      --argjson artifact_id "${artifact_id}" '
        {
          contract: "ait.release.operator.qualification-binding/v1",
          status: "qualified_for_version_only_release",
          source: {
            repository: $repository,
            git_commit: $source_commit,
            release_tag_present: false
          },
          workflow: {
            run_id: $run_id,
            run_attempt: $run_attempt,
            control_commit: $control_commit,
            artifact_id: $artifact_id,
            artifact_digest: $artifact_digest,
            evidence_sha256: $evidence_sha
          },
          gates: {
            core_quality: "pass",
            release_controls: "pass",
            command_inventory: "pass",
            environment_inventory: "pass",
            windows_x64_lifecycle: "pass",
            windows_arm64_lifecycle: "pass"
          },
          mutation: {
            version_authority_write: false,
            tag_write: false,
            receipt_write: false,
            registry_write: false,
            endpoint_repository_write: false
          }
        }
      ' >"${output}"
    printf '%s\n' "${output}"
    ;;

  admit)
    source_root=
    qualification=
    output=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --source-root) [[ $# -ge 2 ]] || usage; source_root=$2; shift 2 ;;
        --qualification) [[ $# -ge 2 ]] || usage; qualification=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${source_root} && -n ${qualification} && -n ${output} ]] || usage
    require_real_directory "${source_root}" 'release public source root'
    require_regular_file "${qualification}" 'pre-RC qualification binding'
    require_new_output "${output}" 'pre-tag admission output'
    source_root=$(canonical_directory "${source_root}")
    qualification=$(canonical_file "${qualification}")
    output=$(canonical_file "${output}")
    jq -e '
      .contract == "ait.release.operator.qualification-binding/v1" and
      .status == "qualified_for_version_only_release" and
      .source.repository == "weita2026/ait-native" and
      (.source.git_commit | test("^[0-9a-f]{40}$")) and
      .source.release_tag_present == false and
      ([.gates[]] | all(. == "pass")) and
      ([.mutation[]] | all(. == false))
    ' "${qualification}" >/dev/null ||
      fail 65 'pre-RC qualification binding is not admissible'
    qualified_commit=$(jq -er '.source.git_commit' "${qualification}")
    release_commit=$(git -C "${source_root}" rev-parse HEAD)
    [[ ${release_commit} =~ ^[0-9a-f]{40}$ ]] ||
      fail 65 'release public source HEAD is not a full Git commit'
    [[ -z $(git -C "${source_root}" status --porcelain --untracked-files=all) ]] ||
      fail 65 'release public source checkout is not clean'
    family=${source_root}/ait-release-family.json
    IFS=$'\t' read -r version channel tag python_version < <(release_identity_tsv "${family}")
    validate_version_identity "${version}" "${channel}" "${tag}" "${python_version}"
    validate_pretag_native_bundle "${family}" "${version}"
    [[ ${channel} == rc || ${channel} == stable ]] ||
      fail 65 'pre-tag admission accepts only an rc or stable release commit'
    if git -C "${source_root}" show-ref --verify --quiet "refs/tags/${tag}"; then
      fail 65 'release tag exists before pre-tag admission'
    fi
    if git -C "${source_root}" tag --points-at "${release_commit}" | grep -Eq '^v[0-9]'; then
      fail 65 'release commit has a release tag before pre-tag admission'
    fi
    if git -C "${source_root}" tag --points-at "${qualified_commit}" | grep -Eq '^v[0-9]'; then
      fail 65 'qualified repair commit gained a release tag after qualification'
    fi
    node "${source_root}/build-release.mjs" \
      --validate-only --git-commit "${release_commit}" >/dev/null
    delta=${temporary_root}/pre-rc-delta.json
    node "${repo_root}/ci/release_pre_rc_delta.mjs" \
      --repository "${source_root}" \
      --qualified-commit "${qualified_commit}" \
      --release-commit "${release_commit}" >"${delta}"
    jq -e \
      --arg qualified "${qualified_commit}" \
      --arg release "${release_commit}" \
      --arg version "${version}" '
        .contract == "ait.release.pre-rc-delta/v1" and
        .decision == "pass" and
        .qualified_commit == $qualified and
        .release_commit == $release and
        .release_version == $version
      ' "${delta}" >/dev/null || fail 65 'pre-RC release delta is not admissible'
    qualification_sha=$(sha256_file "${qualification}")
    delta_sha=$(sha256_file "${delta}")
    qualification_run_id=$(jq -er '.workflow.run_id' "${qualification}")
    qualification_run_attempt=$(jq -er '.workflow.run_attempt' "${qualification}")
    qualification_control_commit=$(jq -er '.workflow.control_commit' "${qualification}")
    qualification_artifact_id=$(jq -er '.workflow.artifact_id' "${qualification}")
    qualification_artifact_digest=$(jq -er '.workflow.artifact_digest' "${qualification}")
    qualification_evidence_sha=$(jq -er '.workflow.evidence_sha256' "${qualification}")
    jq -S -n \
      --arg repository 'weita2026/ait-native' \
      --arg version "${version}" \
      --arg channel "${channel}" \
      --arg python_version "${python_version}" \
      --arg tag "${tag}" \
      --arg qualified_commit "${qualified_commit}" \
      --arg release_commit "${release_commit}" \
      --arg qualification_sha "${qualification_sha}" \
      --arg delta_sha "${delta_sha}" \
      --arg qualification_control_commit "${qualification_control_commit}" \
      --arg qualification_artifact_digest "${qualification_artifact_digest}" \
      --arg qualification_evidence_sha "${qualification_evidence_sha}" \
      --argjson qualification_run_id "${qualification_run_id}" \
      --argjson qualification_run_attempt "${qualification_run_attempt}" \
      --argjson qualification_artifact_id "${qualification_artifact_id}" \
      --slurpfile delta "${delta}" '
        {
          contract: "ait.release.operator.pre-tag-admission/v1",
          status: "ready_for_component_receipts",
          release: {
            repository: $repository,
            version: $version,
            channel: $channel,
            python_version: $python_version,
            tag: $tag,
            source_commit: $release_commit
          },
          qualification: {
            source_commit: $qualified_commit,
            workflow_run_id: $qualification_run_id,
            workflow_run_attempt: $qualification_run_attempt,
            workflow_control_commit: $qualification_control_commit,
            artifact_id: $qualification_artifact_id,
            artifact_digest: $qualification_artifact_digest,
            evidence_sha256: $qualification_evidence_sha,
            binding_sha256: $qualification_sha
          },
          delta: {
            contract: $delta[0].contract,
            sha256: $delta_sha,
            changed_paths: $delta[0].changed_paths
          },
          tag: {created: false, verified: false},
          mutation: {
            tag_write: false,
            receipt_write: false,
            registry_write: false,
            endpoint_repository_write: false
          }
        }
      ' >"${output}"
    printf '%s\n' "${output}"
    ;;

  prepare)
    source_root=
    admission=
    prior_version=
    prior_python_version=
    output=
    dispatch=false
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --source-root) [[ $# -ge 2 ]] || usage; source_root=$2; shift 2 ;;
        --admission) [[ $# -ge 2 ]] || usage; admission=$2; shift 2 ;;
        --prior-version) [[ $# -ge 2 ]] || usage; prior_version=$2; shift 2 ;;
        --prior-python-version) [[ $# -ge 2 ]] || usage; prior_python_version=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        --dispatch) dispatch=true; shift ;;
        *) usage ;;
      esac
    done
    [[ -n ${source_root} && -n ${admission} && -n ${prior_version} &&
      -n ${prior_python_version} && -n ${output} ]] || usage
    require_real_directory "${source_root}" 'public source root'
    require_regular_file "${admission}" 'pre-tag admission'
    require_new_output "${output}" 'prepare output'
    source_root=$(canonical_directory "${source_root}")
    admission=$(canonical_file "${admission}")
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
    validate_pretag_native_bundle "${family}" "${version}"
    validate_prior_release \
      "${version}" "${channel}" "${prior_version}" "${prior_python_version}"
    source_commit=$(git -C "${source_root}" rev-parse HEAD)
    [[ ${source_commit} =~ ^[0-9a-f]{40}$ ]] || fail 65 'public source HEAD is not a full Git commit'
    [[ -z $(git -C "${source_root}" status --porcelain --untracked-files=all) ]] ||
      fail 65 'public source checkout is not clean'
    if git -C "${source_root}" show-ref --verify --quiet "refs/tags/${tag}"; then
      fail 65 'public release tag exists before candidate qualification'
    fi
    if git -C "${source_root}" tag --points-at "${source_commit}" | grep -Eq '^v[0-9]'; then
      fail 65 'release commit has a release tag before candidate qualification'
    fi
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
    jq -e \
      --arg repository "${repository}" \
      --arg version "${version}" \
      --arg channel "${channel}" \
      --arg python_version "${python_version}" \
      --arg tag "${tag}" \
      --arg source_commit "${source_commit}" '
        .contract == "ait.release.operator.pre-tag-admission/v1" and
        .status == "ready_for_component_receipts" and
        .release == {
          repository: $repository,
          version: $version,
          channel: $channel,
          python_version: $python_version,
          tag: $tag,
          source_commit: $source_commit
        } and
        (.qualification.source_commit | test("^[0-9a-f]{40}$")) and
        (.qualification.workflow_run_id | type == "number" and . > 0 and floor == .) and
        (.qualification.workflow_run_attempt | type == "number" and . > 0 and floor == .) and
        (.qualification.workflow_control_commit | test("^[0-9a-f]{40}$")) and
        (.qualification.artifact_id | type == "number" and . > 0 and floor == .) and
        (.qualification.artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
        (.qualification.evidence_sha256 | test("^[0-9a-f]{64}$")) and
        (.qualification.binding_sha256 | test("^[0-9a-f]{64}$")) and
        .delta.contract == "ait.release.pre-rc-delta/v1" and
        (.delta.sha256 | test("^[0-9a-f]{64}$")) and
        (.delta.changed_paths | type == "array" and length > 4) and
        .tag == {created: false, verified: false} and
        ([.mutation[]] | all(. == false))
      ' "${admission}" >/dev/null ||
      fail 65 'pre-tag admission does not bind the exact release commit'
    admission_sha=$(sha256_file "${admission}")
    qualified_commit=$(jq -er '.qualification.source_commit' "${admission}")
    qualification_run_id=$(jq -er '.qualification.workflow_run_id' "${admission}")
    qualification_artifact_id=$(jq -er '.qualification.artifact_id' "${admission}")
    delta_sha=$(jq -er '.delta.sha256' "${admission}")
    admission_b64=$(base64 <"${admission}" | tr -d '\r\n')
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
      --arg admission_sha "${admission_sha}" \
      --arg qualified_commit "${qualified_commit}" \
      --arg delta_sha "${delta_sha}" \
      --arg admission_b64 "${admission_b64}" \
      --arg prior_version "${prior_version}" \
      --arg prior_python_version "${prior_python_version}" \
      --argjson qualification_run_id "${qualification_run_id}" \
      --argjson qualification_artifact_id "${qualification_artifact_id}" \
      --argjson dispatch "${dispatch}" '
        {
          contract: "ait.release.operator.prepare/v3",
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
          pre_rc_admission: {
            admission_sha256: $admission_sha,
            qualified_source_commit: $qualified_commit,
            qualification_workflow_run_id: $qualification_run_id,
            qualification_artifact_id: $qualification_artifact_id,
            version_only_delta_sha256: $delta_sha,
            release_commit_untagged: true
          },
          prior_release: {
            version: $prior_version,
            python_version: $prior_python_version
          },
          receipt_dispatch: {
            workflow: "ait-release-component-receipts.yml",
            ref: "main",
            inputs: {
              coordinator_snapshot: $coordinator_snapshot,
              source_commit: $source_commit,
              pre_tag_admission_sha256: $admission_sha,
              pre_tag_admission_b64: $admission_b64
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
        -f "source_commit=${source_commit}" \
        -f "pre_tag_admission_sha256=${admission_sha}" \
        -f "pre_tag_admission_b64=${admission_b64}"
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
      .contract == "ait.release.operator.prepare/v3" and
      .status == "ready_for_component_receipts" and
      (.release.source_commit | test("^[0-9a-f]{40}$")) and
      (.release.coordinator_snapshot | test("^SNP-[0-9A-F]{12}$")) and
      (.pre_rc_admission.admission_sha256 | test("^[0-9a-f]{64}$")) and
      (.pre_rc_admission.qualified_source_commit | test("^[0-9a-f]{40}$")) and
      (.pre_rc_admission.qualification_workflow_run_id | type == "number" and . > 0 and floor == .) and
      (.pre_rc_admission.qualification_artifact_id | type == "number" and . > 0 and floor == .) and
      (.pre_rc_admission.version_only_delta_sha256 | test("^[0-9a-f]{64}$")) and
      .pre_rc_admission.release_commit_untagged == true and
      (.prior_release.version | test("^[0-9]+\\.[0-9]+\\.[0-9]+(-rc\\.[1-9][0-9]*)?$")) and
      (.prior_release.python_version | type == "string" and length > 0) and
      .receipt_dispatch.inputs.pre_tag_admission_sha256 ==
        .pre_rc_admission.admission_sha256 and
      (.receipt_dispatch.inputs.pre_tag_admission_b64 | test("^[A-Za-z0-9+/=]+$")) and
      ([.mutation[]] | all(. == false))
    ' "${prepare}" >/dev/null || fail 65 'prepare record is not an untagged receipt preparation'
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
    pre_tag_admission=${dossier_root}/ait-release.pre-tag-admission.json
    frozen_manifest=${dossier_root}/frozen/ait-release-family.manifest.json
    frozen_checksums=${dossier_root}/frozen/SHA256SUMS
    for input in "${candidate}" "${promotion}" "${source_mapping}" "${source_evidence}" \
      "${pre_tag_admission}" \
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
    admission_sha=$(jq -er '.pre_rc_admission.admission_sha256' "${prepare}")
    control_commit=$(jq -er '.head_sha' "${run_record}")
    run_attempt=$(jq -er '.run_attempt' "${run_record}")
    artifact_id=$(jq -er '.id' "${artifact_record}")
    artifact_digest=$(jq -er '.digest' "${artifact_record}")
    [[ $(sha256_file "${pre_tag_admission}") == "${admission_sha}" ]] ||
      fail 65 'family dossier pre-tag admission differs from preparation'
    jq -e \
      --arg version "${version}" --arg channel "${channel}" \
      --arg python_version "${python_version}" --arg tag "${tag}" \
      --arg repository "${repository}" --arg commit "${source_commit}" '
        .contract == "ait.release.operator.pre-tag-admission/v1" and
        .status == "ready_for_component_receipts" and
        .release == {
          repository: $repository,
          version: $version,
          channel: $channel,
          python_version: $python_version,
          tag: $tag,
          source_commit: $commit
        } and
        .tag == {created: false, verified: false} and
        ([.mutation[]] | all(. == false))
      ' "${pre_tag_admission}" >/dev/null ||
      fail 65 'family dossier pre-tag admission is invalid'
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
    frozen_checksum_count=$(awk 'NF {count += 1} END {print count + 0}' "${frozen_checksums}")
    ((frozen_checksum_count > 0)) || fail 65 'frozen checksum inventory is empty'
    package_rows=${temporary_root}/candidate-package-rows.jsonl
    : >"${package_rows}"
    for package_channel in apt homebrew npm pypi winget; do
      package_root=${dossier_root}/packages/${package_channel}
      package_receipt=${package_root}/ait-release.package.json
      package_checksums=${package_root}/SHA256SUMS
      require_regular_file "${package_receipt}" "${package_channel} package receipt"
      require_regular_file "${package_checksums}" "${package_channel} package checksums"
      package_artifact_count=$(jq -er \
        '.artifacts | length | select(. > 0)' "${package_receipt}")
      jq -cn \
        --arg channel "${package_channel}" \
        --arg receipt_sha256 "$(sha256_file "${package_receipt}")" \
        --arg checksum_sha256 "$(sha256_file "${package_checksums}")" \
        --argjson artifact_count "${package_artifact_count}" '
          {
            channel: $channel,
            receipt_sha256: $receipt_sha256,
            checksum_sha256: $checksum_sha256,
            artifact_count: $artifact_count
          }
        ' >>"${package_rows}"
    done
    packages=$(jq -s 'sort_by(.channel)' "${package_rows}")
    validate_defaults
    candidate_authority=${temporary_root}/ait-release.pre-tag-candidate-authority.json
    jq -S -n \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg python_version "${python_version}" \
      --arg tag "${tag}" --arg source_commit "${source_commit}" \
      --arg snapshot "${coordinator_snapshot}" \
      --arg frozen_manifest_sha "${frozen_manifest_sha}" \
      --arg frozen_checksums_sha "${frozen_checksums_sha}" \
      --arg admission_sha "${admission_sha}" \
      --argjson run_id "${run_id}" --argjson run_attempt "${run_attempt}" \
      --arg control_commit "${control_commit}" --argjson artifact_id "${artifact_id}" \
      --arg artifact_digest "${artifact_digest}" \
      --argjson frozen_checksum_count "${frozen_checksum_count}" \
      --argjson packages "${packages}" \
      --slurpfile prepare_record "${prepare}" --slurpfile defaults "${defaults}" '
        ($defaults[0]) as $d |
        {
          contract: "ait.release.family.pre-tag-candidate-authority/v1",
          status: "ready_for_pre_tag_clean_host",
          release: {
            id: $release_id,
            version: $version,
            channel: $channel,
            python_version: $python_version,
            tag: $tag,
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
          pre_rc_admission: $prepare_record[0].pre_rc_admission,
          prior_release: $prepare_record[0].prior_release,
          tag_authority: {
            required_state: "absent",
            remote_readback_required_at_start: true,
            remote_readback_required_at_closeout: true
          },
          frozen_checksum_count: $frozen_checksum_count,
          packages: $packages,
          publisher: $d.publisher,
          endpoints: {
            github: ($d.endpoints.github + {prerelease: ($channel == "rc")}),
            pypi: $d.endpoints.pypi,
            npm: ($d.endpoints.npm + {
              dist_tag: (if $channel == "rc" then "rc" else "latest" end)
            }),
            homebrew: (($d.endpoints.homebrew | del(.formula_paths)) + {
              formula_path: $d.endpoints.homebrew.formula_paths[$channel]
            }),
            apt: (($d.endpoints.apt | del(.suites)) + {
              suite: $d.endpoints.apt.suites[$channel]
            }),
            winget: ({identity: $d.endpoints.winget.identity} +
              $d.endpoints.winget.routes[$channel]),
            oci: ($d.endpoints.oci + {
              immutable_tag: $version,
              moving_tag: (if $channel == "rc" then "rc" else "latest" end)
            })
          },
          mutation: {
            artifact_rebuild: false,
            component_rebuild: false,
            endpoint_repository_write: false,
            registry_write: false,
            tag_write: false
          }
        }
      ' >"${candidate_authority}"
    validate_candidate_config "${candidate_authority}" "${release_id}"
    candidate_authority_sha=$(sha256_file "${candidate_authority}")
    candidate_authority_b64=$(base64 <"${candidate_authority}" | tr -d '\r\n')
    jq -S -n \
      --arg release_id "${release_id}" --arg version "${version}" \
      --arg channel "${channel}" --arg python_version "${python_version}" \
      --arg tag "${tag}" --arg repository "${repository}" \
      --arg source_commit "${source_commit}" --arg snapshot "${coordinator_snapshot}" \
      --arg frozen_manifest_sha "${frozen_manifest_sha}" \
      --arg frozen_checksums_sha "${frozen_checksums_sha}" \
      --argjson run_id "${run_id}" --argjson run_attempt "${run_attempt}" \
      --arg control_commit "${control_commit}" --argjson artifact_id "${artifact_id}" \
      --arg artifact_digest "${artifact_digest}" --argjson dispatch "${dispatch}" \
      --arg candidate_authority_sha "${candidate_authority_sha}" \
      --arg candidate_authority_b64 "${candidate_authority_b64}" \
      --slurpfile prepare_record "${prepare}" \
      --slurpfile candidate_authority "${candidate_authority}" '
        {
          contract: "ait.release.operator.receipt-binding/v2",
          status: "ready_for_pre_tag_qualification",
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
          pre_rc_admission: $prepare_record[0].pre_rc_admission,
          prior_release: $prepare_record[0].prior_release,
          candidate_authority: {
            sha256: $candidate_authority_sha,
            record: $candidate_authority[0]
          },
          pre_tag_qualification_dispatch: {
            workflow: "ait-release-pre-tag-qualification.yml",
            ref: "main",
            inputs: {
              release_id: $release_id,
              source_run_id: ($run_id | tostring),
              source_artifact_id: ($artifact_id | tostring),
              source_artifact_digest: $artifact_digest,
              candidate_authority_sha256: $candidate_authority_sha,
              candidate_authority_b64: $candidate_authority_b64,
              prior_version: $prepare_record[0].prior_release.version,
              prior_python_version: $prepare_record[0].prior_release.python_version
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
      gh workflow run ait-release-pre-tag-qualification.yml \
        --repo "${repository}" --ref main \
        -f "release_id=${release_id}" \
        -f "source_run_id=${run_id}" \
        -f "source_artifact_id=${artifact_id}" \
        -f "source_artifact_digest=${artifact_digest}" \
        -f "candidate_authority_sha256=${candidate_authority_sha}" \
        -f "candidate_authority_b64=${candidate_authority_b64}" \
        -f "prior_version=$(jq -er '.prior_release.version' "${prepare}")" \
        -f "prior_python_version=$(jq -er '.prior_release.python_version' "${prepare}")"
    fi
    printf '%s\n' "${output}"
    ;;

  bind-candidate)
    source_root=
    receipts=
    run_id=
    run_record=
    candidate_artifact_record=
    candidate_root=
    aggregate_artifact_record=
    aggregate_root=
    output=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --source-root) [[ $# -ge 2 ]] || usage; source_root=$2; shift 2 ;;
        --receipts) [[ $# -ge 2 ]] || usage; receipts=$2; shift 2 ;;
        --run-id) [[ $# -ge 2 ]] || usage; run_id=$2; shift 2 ;;
        --run-record) [[ $# -ge 2 ]] || usage; run_record=$2; shift 2 ;;
        --candidate-artifact-record) [[ $# -ge 2 ]] || usage; candidate_artifact_record=$2; shift 2 ;;
        --candidate-root) [[ $# -ge 2 ]] || usage; candidate_root=$2; shift 2 ;;
        --aggregate-artifact-record) [[ $# -ge 2 ]] || usage; aggregate_artifact_record=$2; shift 2 ;;
        --aggregate-root) [[ $# -ge 2 ]] || usage; aggregate_root=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        *) usage ;;
      esac
    done
    [[ -n ${source_root} && -n ${receipts} && -n ${output} ]] || usage
    require_real_directory "${source_root}" 'public source root'
    require_regular_file "${receipts}" 'receipt binding'
    require_new_output "${output}" 'candidate binding output'
    source_root=$(canonical_directory "${source_root}")
    receipts=$(canonical_file "${receipts}")
    output=$(canonical_file "${output}")
    jq -e '
      .contract == "ait.release.operator.receipt-binding/v2" and
      .status == "ready_for_pre_tag_qualification" and
      (.release.id | test("^REL-FAM-[0-9A-F]{16}$")) and
      (.candidate_authority.sha256 | test("^[0-9a-f]{64}$")) and
      .candidate_authority.record.contract ==
        "ait.release.family.pre-tag-candidate-authority/v1" and
      .candidate_authority.record.status == "ready_for_pre_tag_clean_host" and
      ([.mutation[]] | all(. == false))
    ' "${receipts}" >/dev/null ||
      fail 65 'receipt binding is not ready for pre-tag candidate qualification'
    repository=$(jq -er '.release.repository' "${receipts}")
    release_id=$(jq -er '.release.id' "${receipts}")
    candidate_authority=${temporary_root}/candidate-authority.json
    jq -S '.candidate_authority.record' "${receipts}" >"${candidate_authority}"
    [[ $(sha256_file "${candidate_authority}") == \
      $(jq -er '.candidate_authority.sha256' "${receipts}") ]] ||
      fail 65 'receipt binding candidate authority digest is not exact'
    validate_candidate_config "${candidate_authority}" "${release_id}"
    if [[ -n ${run_id} ]]; then
      [[ ${run_id} =~ ^[1-9][0-9]*$ &&
        -z ${run_record}${candidate_artifact_record}${candidate_root}${aggregate_artifact_record}${aggregate_root} ]] || usage
      run_record=${temporary_root}/candidate-run.json
      candidate_artifact_record=${temporary_root}/candidate-artifact.json
      aggregate_artifact_record=${temporary_root}/aggregate-artifact.json
      fetch_run_record "${repository}" "${run_id}" "${run_record}"
      fetch_artifact_record "${repository}" "${run_id}" \
        "ait-pre-tag-candidate-${release_id}" "${candidate_artifact_record}"
      fetch_artifact_record "${repository}" "${run_id}" \
        "ait-pre-tag-clean-host-${release_id}" "${aggregate_artifact_record}"
      candidate_root=${temporary_root}/candidate
      aggregate_root=${temporary_root}/aggregate
      download_run_artifact "${repository}" "${run_id}" \
        "ait-pre-tag-candidate-${release_id}" "${candidate_root}"
      download_run_artifact "${repository}" "${run_id}" \
        "ait-pre-tag-clean-host-${release_id}" "${aggregate_root}"
    else
      [[ -n ${run_record} && -n ${candidate_artifact_record} && -n ${candidate_root} &&
        -n ${aggregate_artifact_record} && -n ${aggregate_root} ]] || usage
      run_id=$(jq -er '.id | select(type == "number" and . > 0 and floor == .)' "${run_record}")
    fi
    require_regular_file "${run_record}" 'candidate qualification run record'
    require_regular_file "${candidate_artifact_record}" 'candidate artifact record'
    require_regular_file "${aggregate_artifact_record}" 'candidate aggregate artifact record'
    require_real_directory "${candidate_root}" 'pre-tag candidate root'
    require_real_directory "${aggregate_root}" 'pre-tag aggregate root'
    candidate_root=$(canonical_directory "${candidate_root}")
    aggregate_root=$(canonical_directory "${aggregate_root}")
    validate_workflow_run "${run_record}" "${run_id}" \
      'ait release pre-tag candidate qualification' \
      '.github/workflows/ait-release-pre-tag-qualification.yml'
    candidate_artifact_id=$(jq -er '.id' "${candidate_artifact_record}")
    candidate_artifact_digest=$(jq -er '.digest' "${candidate_artifact_record}")
    aggregate_artifact_id=$(jq -er '.id' "${aggregate_artifact_record}")
    aggregate_artifact_digest=$(jq -er '.digest' "${aggregate_artifact_record}")
    validate_artifact_record "${candidate_artifact_record}" "${candidate_artifact_id}" \
      "ait-pre-tag-candidate-${release_id}" "${run_id}"
    validate_artifact_record "${aggregate_artifact_record}" "${aggregate_artifact_id}" \
      "ait-pre-tag-clean-host-${release_id}" "${run_id}"
    candidate_status=${candidate_root}/ait-release.prepublish-candidate.json
    aggregate_status=${aggregate_root}/ait-release.clean-host-status.json
    embedded_authority=${candidate_root}/ait-release.endpoints.authority.json
    for input in "${candidate_status}" "${aggregate_status}" "${embedded_authority}"; do
      require_regular_file "${input}" 'pre-tag qualification binding input'
    done
    cmp -s "${candidate_authority}" "${embedded_authority}" ||
      fail 65 'qualified candidate authority differs from the receipt binding'
    candidate_status_sha=$(sha256_file "${candidate_status}")
    aggregate_status_sha=$(sha256_file "${aggregate_status}")
    node "${repo_root}/ci/release_prepublish_verify.mjs" stage \
      --root "${candidate_root}" \
      --config-sha256 "$(sha256_file "${candidate_authority}")" \
      --status-sha256 "${candidate_status_sha}" >/dev/null
    AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST="${candidate_artifact_digest}" \
      node "${repo_root}/ci/release_prepublish_verify.mjs" qualify \
        --root "${aggregate_root}" --candidate-root "${candidate_root}" \
        --candidate-artifact-digest "${candidate_artifact_digest}" \
        --aggregate-sha256 "${aggregate_status_sha}" >/dev/null
    jq -e --arg release_id "${release_id}" '
      .contract == "ait.release.clean-host.aggregate/v1" and
      .status == "qualified" and .release.id == $release_id and
      .release.verification_stage == "pre_tag" and
      .matrix.expected_rows == 32 and .matrix.admitted_rows == 32 and
      .matrix.evidence_files == 32 and .failures == [] and
      .promotion.allowed == true
    ' "${aggregate_status}" >/dev/null ||
      fail 65 'clean-host aggregate is not a complete pre-tag qualification'
    source_commit=$(jq -er '.release.source_commit' "${receipts}")
    tag=$(jq -er '.release.tag' "${receipts}")
    [[ $(git -C "${source_root}" rev-parse HEAD) == "${source_commit}" ]] ||
      fail 65 'public source HEAD differs from the qualified candidate commit'
    [[ -z $(git -C "${source_root}" status --porcelain --untracked-files=all) ]] ||
      fail 65 'public source checkout is not clean at candidate binding'
    if git -C "${source_root}" show-ref --verify --quiet "refs/tags/${tag}"; then
      fail 65 'release tag exists before candidate qualification is bound'
    fi
    if git -C "${source_root}" tag --points-at "${source_commit}" | grep -Eq '^v[0-9]'; then
      fail 65 'release commit gained a release tag before candidate qualification was bound'
    fi
    run_attempt=$(jq -er '.run_attempt' "${run_record}")
    control_commit=$(jq -er '.head_sha' "${run_record}")
    jq -S -n \
      --slurpfile receipts_record "${receipts}" \
      --argjson run_id "${run_id}" --argjson run_attempt "${run_attempt}" \
      --arg control_commit "${control_commit}" \
      --argjson candidate_artifact_id "${candidate_artifact_id}" \
      --arg candidate_artifact_digest "${candidate_artifact_digest}" \
      --arg candidate_status_sha "${candidate_status_sha}" \
      --argjson aggregate_artifact_id "${aggregate_artifact_id}" \
      --arg aggregate_artifact_digest "${aggregate_artifact_digest}" \
      --arg aggregate_status_sha "${aggregate_status_sha}" '
        ($receipts_record[0]) as $r |
        {
          contract: "ait.release.operator.pre-tag-candidate-binding/v1",
          status: "ready_for_immutable_tag",
          release: $r.release,
          source_dossier: $r.source_dossier,
          candidate_authority: $r.candidate_authority,
          qualification: {
            workflow_run_id: $run_id,
            workflow_run_attempt: $run_attempt,
            workflow_control_commit: $control_commit,
            candidate_artifact_id: $candidate_artifact_id,
            candidate_artifact_digest: $candidate_artifact_digest,
            candidate_status_sha256: $candidate_status_sha,
            aggregate_artifact_id: $aggregate_artifact_id,
            aggregate_artifact_digest: $aggregate_artifact_digest,
            aggregate_status_sha256: $aggregate_status_sha,
            clean_host_rows: 32,
            tag_state_at_closeout: "absent"
          },
          tag: {created: false, verified: false},
          mutation: {
            artifact_rebuild: false,
            component_rebuild: false,
            endpoint_repository_write: false,
            registry_write: false,
            tag_write: false
          }
        }
      ' >"${output}"
    printf '%s\n' "${output}"
    ;;

  authorize)
    source_root=
    candidate=
    output=
    dispatch=false
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --source-root) [[ $# -ge 2 ]] || usage; source_root=$2; shift 2 ;;
        --candidate) [[ $# -ge 2 ]] || usage; candidate=$2; shift 2 ;;
        --output) [[ $# -ge 2 ]] || usage; output=$2; shift 2 ;;
        --dispatch) dispatch=true; shift ;;
        *) usage ;;
      esac
    done
    [[ -n ${source_root} && -n ${candidate} && -n ${output} ]] || usage
    require_real_directory "${source_root}" 'public source root'
    require_regular_file "${candidate}" 'pre-tag candidate binding'
    require_new_output "${output}" 'tag authorization output'
    source_root=$(canonical_directory "${source_root}")
    candidate=$(canonical_file "${candidate}")
    output=$(canonical_file "${output}")
    jq -e '
      .contract == "ait.release.operator.pre-tag-candidate-binding/v1" and
      .status == "ready_for_immutable_tag" and
      (.release.id | test("^REL-FAM-[0-9A-F]{16}$")) and
      .qualification.clean_host_rows == 32 and
      .qualification.tag_state_at_closeout == "absent" and
      .tag == {created: false, verified: false} and
      ([.mutation[]] | all(. == false))
    ' "${candidate}" >/dev/null ||
      fail 65 'pre-tag candidate binding is not ready for immutable tag verification'
    source_commit=$(jq -er '.release.source_commit' "${candidate}")
    tag=$(jq -er '.release.tag' "${candidate}")
    repository=$(jq -er '.release.repository' "${candidate}")
    [[ $(git -C "${source_root}" rev-parse HEAD) == "${source_commit}" ]] ||
      fail 65 'public source HEAD differs from the qualified candidate commit'
    [[ -z $(git -C "${source_root}" status --porcelain --untracked-files=all) ]] ||
      fail 65 'public source checkout is not clean after tag creation'
    [[ $(git -C "${source_root}" cat-file -t "refs/tags/${tag}" 2>/dev/null) == tag ]] ||
      fail 65 'qualified public release tag must be an annotated tag object'
    [[ $(git -C "${source_root}" rev-list -n 1 "refs/tags/${tag}" 2>/dev/null) == \
      "${source_commit}" ]] ||
      fail 65 'qualified public release tag does not resolve to the candidate commit'
    release_id=$(jq -er '.release.id' "${candidate}")
    channel=$(jq -er '.release.channel' "${candidate}")
    source_run_id=$(jq -er '.source_dossier.workflow_run_id' "${candidate}")
    source_run_attempt=$(jq -er '.source_dossier.workflow_run_attempt' "${candidate}")
    dossier_artifact_id=$(jq -er '.source_dossier.artifact_id' "${candidate}")
    dossier_artifact_digest=$(jq -er '.source_dossier.artifact_digest' "${candidate}")
    source_control_commit=$(jq -er '.source_dossier.workflow_control_commit' "${candidate}")
    coordinator_snapshot=$(jq -er '.release.coordinator_snapshot' "${candidate}")
    frozen_manifest_sha=$(jq -er '.release.frozen_manifest_sha256' "${candidate}")
    frozen_checksums_sha=$(jq -er '.release.frozen_checksums_sha256' "${candidate}")
    qualification_run_id=$(jq -er '.qualification.workflow_run_id' "${candidate}")
    qualification_run_attempt=$(jq -er '.qualification.workflow_run_attempt' "${candidate}")
    qualification_control_commit=$(jq -er '.qualification.workflow_control_commit' "${candidate}")
    candidate_artifact_id=$(jq -er '.qualification.candidate_artifact_id' "${candidate}")
    candidate_artifact_digest=$(jq -er '.qualification.candidate_artifact_digest' "${candidate}")
    candidate_status_sha=$(jq -er '.qualification.candidate_status_sha256' "${candidate}")
    aggregate_artifact_id=$(jq -er '.qualification.aggregate_artifact_id' "${candidate}")
    aggregate_artifact_digest=$(jq -er '.qualification.aggregate_artifact_digest' "${candidate}")
    aggregate_status_sha=$(jq -er '.qualification.aggregate_status_sha256' "${candidate}")
    jq -S -n --slurpfile candidate_record "${candidate}" --argjson dispatch "${dispatch}" '
      ($candidate_record[0]) as $c |
      {
        contract: "ait.release.operator.tag-binding/v1",
        status: "ready_for_protected_authorization",
        release: $c.release,
        source_dossier: $c.source_dossier,
        candidate_authority: $c.candidate_authority,
        qualification: $c.qualification,
        tag: {created: true, verified: true, annotated: true},
        protected_dispatch: {
          workflow: "ait-release-protected-promotion.yml",
          ref: "main",
          inputs: {
            source_run_id: ($c.source_dossier.workflow_run_id | tostring),
            source_run_attempt: ($c.source_dossier.workflow_run_attempt | tostring),
            dossier_artifact_id: ($c.source_dossier.artifact_id | tostring),
            dossier_artifact_digest: $c.source_dossier.artifact_digest,
            release_id: $c.release.id,
            channel: $c.release.channel,
            tag: $c.release.tag,
            source_commit: $c.release.source_commit,
            source_control_commit: $c.source_dossier.workflow_control_commit,
            coordinator_snapshot: $c.release.coordinator_snapshot,
            frozen_manifest_sha256: $c.release.frozen_manifest_sha256,
            checksum_sha256: $c.release.frozen_checksums_sha256,
            qualification_run_id: ($c.qualification.workflow_run_id | tostring),
            qualification_run_attempt: ($c.qualification.workflow_run_attempt | tostring),
            qualification_control_commit: $c.qualification.workflow_control_commit,
            candidate_artifact_id: ($c.qualification.candidate_artifact_id | tostring),
            candidate_artifact_digest: $c.qualification.candidate_artifact_digest,
            candidate_status_sha256: $c.qualification.candidate_status_sha256,
            aggregate_artifact_id: ($c.qualification.aggregate_artifact_id | tostring),
            aggregate_artifact_digest: $c.qualification.aggregate_artifact_digest,
            aggregate_status_sha256: $c.qualification.aggregate_status_sha256
          },
          requested: $dispatch
        },
        mutation: {
          artifact_rebuild: false,
          component_rebuild: false,
          endpoint_repository_write: false,
          registry_write: false,
          tag_write: false
        }
      }
    ' >"${output}"
    if [[ ${dispatch} == true ]]; then
      require_gh
      gh workflow run ait-release-protected-promotion.yml \
        --repo "${repository}" --ref main \
        -f "source_run_id=${source_run_id}" \
        -f "source_run_attempt=${source_run_attempt}" \
        -f "dossier_artifact_id=${dossier_artifact_id}" \
        -f "dossier_artifact_digest=${dossier_artifact_digest}" \
        -f "release_id=${release_id}" -f "channel=${channel}" -f "tag=${tag}" \
        -f "source_commit=${source_commit}" \
        -f "source_control_commit=${source_control_commit}" \
        -f "coordinator_snapshot=${coordinator_snapshot}" \
        -f "frozen_manifest_sha256=${frozen_manifest_sha}" \
        -f "checksum_sha256=${frozen_checksums_sha}" \
        -f "qualification_run_id=${qualification_run_id}" \
        -f "qualification_run_attempt=${qualification_run_attempt}" \
        -f "qualification_control_commit=${qualification_control_commit}" \
        -f "candidate_artifact_id=${candidate_artifact_id}" \
        -f "candidate_artifact_digest=${candidate_artifact_digest}" \
        -f "candidate_status_sha256=${candidate_status_sha}" \
        -f "aggregate_artifact_id=${aggregate_artifact_id}" \
        -f "aggregate_artifact_digest=${aggregate_artifact_digest}" \
        -f "aggregate_status_sha256=${aggregate_status_sha}"
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
      .contract == "ait.release.operator.tag-binding/v1" and
      .status == "ready_for_protected_authorization" and
      (.release.id | test("^REL-FAM-[0-9A-F]{16}$")) and
      .tag == {created: true, verified: true, annotated: true} and
      .qualification.clean_host_rows == 32 and
      .qualification.tag_state_at_closeout == "absent" and
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
        .pre_tag_qualification == {
          workflow_run_id: ($r.qualification.workflow_run_id | tostring),
          workflow_run_attempt: ($r.qualification.workflow_run_attempt | tostring),
          workflow_control_commit: $r.qualification.workflow_control_commit,
          candidate_artifact_id: ($r.qualification.candidate_artifact_id | tostring),
          candidate_artifact_digest: $r.qualification.candidate_artifact_digest,
          candidate_status_sha256: $r.qualification.candidate_status_sha256,
          aggregate_artifact_id: ($r.qualification.aggregate_artifact_id | tostring),
          aggregate_artifact_digest: $r.qualification.aggregate_artifact_digest,
          aggregate_status_sha256: $r.qualification.aggregate_status_sha256,
          clean_host_rows: 32,
          tag_state_at_closeout: "absent"
        } and
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
          pre_tag_qualification: $r.qualification,
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
            github: ($d.endpoints.github + {
              prerelease: ($r.release.channel == "rc")
            }),
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

  validate-candidate-config)
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
    validate_candidate_config "${config}" "${expected_release_id}"
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
    config_sha=$(sha256_file "${config}")
    jq -e \
      --slurpfile config "${config}" \
      --arg config_sha "${config_sha}" \
      --arg winget "${winget_status}" '
      ($config[0]) as $c |
      .contract == "ait.release.family.endpoint-readback/v2" and
      .status == "published_after_prepublish_qualification" and
      .release == {
        id: $c.release.id,
        version: $c.release.version,
        python_version: $c.release.python_version,
        channel: $c.release.channel,
        tag: $c.release.tag,
        source_commit: $c.release.source_commit,
        endpoint_config_sha256: $config_sha
      } and
      .endpoints.github == "published_and_read_back" and
      .endpoints.pypi == "published_and_read_back" and
      .endpoints.npm == "published_and_read_back" and
      .endpoints.homebrew == "published_and_read_back" and
      .endpoints.apt == "published_signed_and_read_back" and
      .endpoints.winget == $winget and
      (.endpoints.oci.server | test("^sha256:[0-9a-f]{64}$")) and
      (.endpoints.oci.runner | test("^sha256:[0-9a-f]{64}$")) and
      .endpoints.oci.immutable_tag == $c.endpoints.oci.immutable_tag and
      .endpoints.oci.moving_tag == $c.endpoints.oci.moving_tag and
      .prepublication.status == "qualified" and
      (.prepublication.candidate_artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
      (.prepublication.aggregate_artifact_digest | test("^sha256:[0-9a-f]{64}$")) and
      (.prepublication.aggregate_status_sha256 | test("^[0-9a-f]{64}$")) and
      .prepublication.clean_host_rows == 32 and
      .mutation.artifact_rebuild == false and
      .mutation.component_rebuild == false and
      .mutation.registry_write == true and
      .mutation.github_release_write == true and
      .mutation.endpoint_repository_write == true and
      .mutation.tag_write == false and
      .next_action == "promote_mutable_aliases"
    ' "${readback}" >/dev/null || fail 65 'endpoint readback evidence is incomplete or belongs to another release'
    jq -e --arg release_id "${release_id}" '
      .contract == "ait.release.endpoint.apt/v1" and .release_id == $release_id and
      .status == "published_signed_and_read_back" and .signature_readback == true and
      .package_digest_readback == true and .apt_cache_search == true
    ' "${evidence_root}/apt.json" >/dev/null || fail 65 'APT status does not prove signed apt-cache searchability'
    jq -S -n \
      --slurpfile readback "${readback}" \
      --arg readback_sha "$(sha256_file "${readback}")" \
      --argjson run_id "${run_id}" \
      --argjson run_attempt "$(jq -er '.run_attempt' "${run_record}")" \
      --arg control_commit "$(jq -er '.head_sha' "${run_record}")" \
      --argjson artifact_id "${artifact_id}" \
      --arg artifact_digest "$(jq -er '.digest' "${artifact_record}")" '
        {
          contract: "ait.release.operator.status/v2",
          status: "published_readback_complete",
          release: $readback[0].release,
          prepublication: $readback[0].prepublication,
          publication_workflow: {
            run_id: $run_id,
            run_attempt: $run_attempt,
            control_commit: $control_commit,
            artifact_id: $artifact_id,
            artifact_digest: $artifact_digest,
            conclusion: "success"
          },
          endpoint_publication: {
            readback_sha256: $readback_sha,
            platforms: $readback[0].endpoints,
            mutation: $readback[0].mutation
          },
          promotion_allowed: true,
          terminal_for_release: false,
          next_action: $readback[0].next_action
        }
      ' >"${output}"
    printf '%s\n' "${output}"
    ;;

  *) usage ;;
esac
