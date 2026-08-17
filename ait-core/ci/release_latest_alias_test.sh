#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
latest_alias=${repo_root}/ci/release_latest_alias.sh
defaults=${repo_root}/release/endpoint-publication.defaults.json
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-latest-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-latest-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected latest-alias test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected latest-alias failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

test -x "${latest_alias}"
bash -n "${latest_alias}"

version=1.2.3-rc.6
python_version=1.2.3rc6
release_id=REL-FAM-0123456789ABCDEF
source_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
server_digest=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
runner_digest=sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
config=${temporary_root}/endpoints.json
status=${temporary_root}/status.json

jq -S -n --slurpfile defaults "${defaults}" \
  --arg version "${version}" --arg python_version "${python_version}" \
  --arg release_id "${release_id}" --arg source_commit "${source_commit}" '
  ($defaults[0]) as $d |
  {
    contract: "ait.release.family.endpoints/v1",
    release: {
      id: $release_id,
      version: $version,
      channel: "rc",
      python_version: $python_version,
      tag: ("v" + $version),
      source_commit: $source_commit,
      coordinator_snapshot: "SNP-ABCDEF123456",
      frozen_manifest_sha256: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
      frozen_checksums_sha256: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    },
    source_dossier: {
      workflow_run_id: 101,
      workflow_run_attempt: 1,
      workflow_control_commit: "ffffffffffffffffffffffffffffffffffffffff",
      artifact_id: 201,
      artifact_digest: "sha256:1111111111111111111111111111111111111111111111111111111111111111"
    },
    protected_authorization: {
      workflow_run_id: 102,
      workflow_run_attempt: 1,
      workflow_control_commit: "2222222222222222222222222222222222222222",
      artifact_id: 202,
      artifact_digest: "sha256:3333333333333333333333333333333333333333333333333333333333333333",
      evidence_sha256: "4444444444444444444444444444444444444444444444444444444444444444"
    },
    publisher: $d.publisher,
    endpoints: {
      github: ($d.endpoints.github + {prerelease: false}),
      pypi: $d.endpoints.pypi,
      npm: ($d.endpoints.npm + {dist_tag: "rc"}),
      homebrew: (($d.endpoints.homebrew | del(.formula_paths)) + {
        formula_path: $d.endpoints.homebrew.formula_paths.rc
      }),
      apt: (($d.endpoints.apt | del(.suites)) + {
        suite: $d.endpoints.apt.suites.rc
      }),
      winget: ({identity: $d.endpoints.winget.identity} + $d.endpoints.winget.routes.rc),
      oci: ($d.endpoints.oci + {immutable_tag: $version, moving_tag: "rc"})
    }
  }
' >"${config}"

jq -S -n --arg release_id "${release_id}" --arg version "${version}" \
  --arg server "${server_digest}" --arg runner "${runner_digest}" '
  {
    contract: "ait.release.operator.status/v1",
    status: "published_pending_clean_host_smoke",
    next_action: "run_all_declared_clean_host_install_upgrade_uninstall_smoke",
    release: {id: $release_id, tag: ("v" + $version), version: $version},
    publication_workflow: {
      run_id: 103,
      artifact_id: 203,
      artifact_digest: "sha256:5555555555555555555555555555555555555555555555555555555555555555",
      conclusion: "success"
    },
    platforms: {
      github: "published_and_read_back",
      pypi: "published_and_read_back",
      npm: "published_and_read_back",
      homebrew: "published_and_read_back",
      apt: "published_signed_and_read_back",
      winget: "validation_assets_published_no_community_submission",
      oci: {
        server: $server,
        runner: $runner,
        immutable_tag: $version,
        moving_tag: "rc"
      }
    }
  }
' >"${status}"

state=${temporary_root}/state
bin=${temporary_root}/bin
mkdir "${state}" "${bin}"
printf '%s\n' v0.9.0 >"${state}/github-latest"
jq -n --slurpfile config "${config}" --arg version "${version}" '
  reduce $config[0].endpoints.npm.packages[] as $package ({};
    .[$package] = {rc: $version, latest: "1.0.0-rc.3"})
' >"${state}/npm.json"
printf '{}\n' >"${state}/npm-stale.json"
printf '0\n' >"${state}/npm-stale-read-count"
jq -n --arg server "${server_digest}" --arg runner "${runner_digest}" '
  {
    "ghcr.io/weita2026/ait-server": {digest: $server, latest: null},
    "ghcr.io/weita2026/ait-runner": {digest: $runner, latest: null}
  }
' >"${state}/oci.json"

cat >"${bin}/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_TEST_LATEST_STATE:?}
[[ ${1:-} == api ]] || exit 64
shift
if [[ ${1:-} == --method ]]; then
  [[ $2 == PATCH && $3 == repos/weita2026/ait-native/releases/777 &&
    $4 == -f && $5 == make_latest=true ]] || exit 64
  printf '%s\n' v1.2.3-rc.6 >"${state}/github-latest"
  printf '{}\n'
  exit 0
fi
path=$1
shift
case "${path}" in
  repos/weita2026/ait-native/releases/tags/v1.2.3-rc.6)
    printf '{"id":777,"tag_name":"v1.2.3-rc.6","draft":false,"prerelease":false}\n'
    ;;
  repos/weita2026/ait-native/git/ref/tags/v1.2.3-rc.6)
    printf '{"object":{"type":"tag","sha":"9999999999999999999999999999999999999999"}}\n'
    ;;
  repos/weita2026/ait-native/git/tags/9999999999999999999999999999999999999999)
    printf '{"object":{"type":"commit","sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}\n'
    ;;
  repos/weita2026/ait-native/releases/latest)
    value=$(sed -n '1p' "${state}/github-latest")
    if [[ ${1:-} == --jq ]]; then printf '%s\n' "${value}"; else printf '{"tag_name":"%s"}\n' "${value}"; fi
    ;;
  *) exit 64 ;;
esac
STUB

cat >"${bin}/npm" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
state_root=${AIT_TEST_LATEST_STATE:?}
state=${state_root}/npm.json
stale=${state_root}/npm-stale.json
case "${1:-}" in
  view)
    subject=$2
    field=$3
    if [[ ${field} == version ]]; then
      printf '"%s"\n' "${subject##*@}"
    elif [[ ${field} == dist-tags ]]; then
      stale_value=$(jq -c --arg package "${subject}" '.[$package] // empty' "${stale}")
      if [[ -n ${stale_value} ]]; then
        printf '%s\n' "${stale_value}"
        jq --arg package "${subject}" 'del(.[$package])' "${stale}" >"${stale}.new"
        mv "${stale}.new" "${stale}"
        stale_count=$(sed -n '1p' "${state_root}/npm-stale-read-count")
        printf '%s\n' "$((stale_count + 1))" >"${state_root}/npm-stale-read-count"
      else
        jq -e --arg package "${subject}" '.[$package]' "${state}"
      fi
    else
      exit 64
    fi
    ;;
  dist-tag)
    [[ $2 == add && $4 == latest ]] || exit 64
    subject=$3
    package=${subject%@*}
    version=${subject##*@}
    before=$(jq -c --arg package "${package}" '.[$package]' "${state}")
    jq --arg package "${package}" --arg version "${version}" \
      '.[$package].latest = $version' "${state}" >"${state}.new"
    mv "${state}.new" "${state}"
    jq --arg package "${package}" --argjson before "${before}" \
      '.[$package] = $before' "${stale}" >"${stale}.new"
    mv "${stale}.new" "${stale}"
    ;;
  *) exit 64 ;;
esac
STUB

cat >"${bin}/docker" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_TEST_LATEST_STATE:?}/oci.json
[[ ${1:-} == buildx && ${2:-} == imagetools ]] || exit 64
case "${3:-}" in
  inspect)
    reference=$4
    if [[ ${reference} == *@sha256:* ]]; then
      digest=${reference#*@}
    else
      image=${reference%:*}
      tag=${reference##*:}
      case "${tag}" in
        latest) digest=$(jq -er --arg image "${image}" '.[$image].latest' "${state}") ;;
        rc|1.2.3-rc.6) digest=$(jq -er --arg image "${image}" '.[$image].digest' "${state}") ;;
        *) exit 64 ;;
      esac
    fi
    jq -n --arg digest "${digest}" '$digest'
    ;;
  create)
    [[ $4 == --tag ]] || exit 64
    target=$5
    source=$6
    image=${target%:latest}
    digest=${source#*@}
    jq --arg image "${image}" --arg digest "${digest}" \
      '.[$image].latest = $digest' "${state}" >"${state}.new"
    mv "${state}.new" "${state}"
    ;;
  *) exit 64 ;;
esac
STUB

cat >"${bin}/sleep" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 1 && $1 == 5 ]]
STUB
chmod 0755 "${bin}/gh" "${bin}/npm" "${bin}/docker" "${bin}/sleep"

export AIT_TEST_LATEST_STATE=${state}
export PATH=${bin}:${PATH}

expect_failure verify-before "${latest_alias}" verify "${config}" "${status}" \
  "${temporary_root}/verify-before.json"
expect_failure missing-approval "${latest_alias}" apply "${config}" "${status}" \
  "${temporary_root}/missing-approval.json"

apply_evidence=${temporary_root}/apply.json
AIT_RELEASE_LATEST_RELEASE_ID=${release_id} \
  "${latest_alias}" apply "${config}" "${status}" "${apply_evidence}" >/dev/null
test "$(sed -n '1p' "${state}/npm-stale-read-count")" = 7
jq -e 'length == 0' "${state}/npm-stale.json" >/dev/null
jq -e --arg version "${version}" --arg server "${server_digest}" \
  --arg runner "${runner_digest}" '
  .contract == "ait.release.latest-alias/v1" and
  .status == "promoted_and_read_back" and
  .release.version == $version and .release.channel == "rc" and
  .aliases.github.after == ("v" + $version) and
  (.aliases.npm.packages | length) == 7 and
  ([.aliases.npm.packages[].after] | unique) == [$version] and
  .aliases.npm.rc_alias_retained == true and
  (.aliases.oci.images | length) == 2 and
  ([.aliases.oci.images[].after] | sort) == ([$server, $runner] | sort) and
  .aliases.oci.rc_alias_retained == true and
  .native_prerelease_routes.pypi.mutable_latest_alias_supported == false and
  .native_prerelease_routes.pypi.exact_selector == "ait-native==1.2.3rc6" and
  .native_prerelease_routes.homebrew.stable_formula_unchanged == true and
  .native_prerelease_routes.apt.stable_suite_unchanged == true and
  .native_prerelease_routes.winget.route == "validation" and
  .mutation.github_release_write == true and
  .mutation.npm_dist_tag_write_count == 7 and
  .mutation.oci_tag_write_count == 2 and
  .mutation.artifact_rebuild == false and
  .mutation.immutable_version_write == false and
  .mutation.tag_write == false
' "${apply_evidence}" >/dev/null

verify_evidence=${temporary_root}/verify.json
"${latest_alias}" verify "${config}" "${status}" "${verify_evidence}" >/dev/null
jq -e '
  .status == "verified" and
  .mutation.github_release_write == false and
  .mutation.npm_dist_tag_write_count == 0 and
  .mutation.oci_tag_write_count == 0 and
  ([.aliases.npm.packages[].mutated] | all(. == false)) and
  ([.aliases.oci.images[].mutated] | all(. == false))
' "${verify_evidence}" >/dev/null

jq '.release.id = "REL-FAM-FFFFFFFFFFFFFFFF"' "${status}" \
  >"${temporary_root}/wrong-status.json"
expect_failure wrong-status "${latest_alias}" verify "${config}" \
  "${temporary_root}/wrong-status.json" "${temporary_root}/wrong-status-output.json"

printf 'release latest alias contract: pass\n'
