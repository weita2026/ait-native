#!/usr/bin/env bash
set -euo pipefail

operation_owner=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/release_operation_owner.mjs
if [[ $# -gt 0 && ${1:-} != --help && ${1:-} != -h ]]; then
  if [[ -z ${AIT_RELEASE_OPERATION_TOKEN:-} ]]; then
    exec node "${operation_owner}" --shell "$0" "$@"
  fi
  node "${operation_owner}" --check "${AIT_RELEASE_OPERATION_ROOT:?}" || exit 75
fi

usage() {
  printf '%s\n' 'usage: release_web_admission.sh {run|status} --input <absolute-json> --repository <absolute-path> --ait <absolute-path> [--personal-bin <absolute-path>] [--community-bin <absolute-path> --community-cli-bin <absolute-path> --chrome <absolute-path>] --server-url <loopback-url> [--server-bin <absolute-path> --server-data <absolute-path>] --output-dir <absolute-path>' >&2
  exit 64
}

mode=${1:-}
[[ ${mode} == run || ${mode} == status ]] || usage
shift
input=''
repository=''
ait_bin=''
personal_bin=''
community_bin=''
community_cli_bin=''
server_url=''
server_bin=''
server_data=''
chrome=''
output_dir=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --input) input=${2:-}; shift 2 ;;
    --repository) repository=${2:-}; shift 2 ;;
    --ait) ait_bin=${2:-}; shift 2 ;;
    --personal-bin) personal_bin=${2:-}; shift 2 ;;
    --community-bin) community_bin=${2:-}; shift 2 ;;
    --community-cli-bin) community_cli_bin=${2:-}; shift 2 ;;
    --server-url) server_url=${2:-}; shift 2 ;;
    --server-bin) server_bin=${2:-}; shift 2 ;;
    --server-data) server_data=${2:-}; shift 2 ;;
    --chrome) chrome=${2:-}; shift 2 ;;
    --output-dir) output_dir=${2:-}; shift 2 ;;
    *) usage ;;
  esac
done
[[ (-z ${server_bin} && -z ${server_data}) || (${server_bin} == /* && ${server_data} == /*) ]] || usage
for required in input repository ait_bin server_url output_dir; do
  [[ ${!required} == /* || ${required} == server_url ]] || usage
  [[ -n ${!required} ]] || usage
done
[[ ${server_url} =~ ^http://(127\.0\.0\.1|localhost):[0-9]+/$ ]] || {
  printf 'server URL must be an exact loopback HTTP origin with a trailing slash\n' >&2
  exit 64
}

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repository=$(CDPATH='' cd -- "${repository}" && pwd)
output_dir=$(mkdir -p -- "${output_dir}" && CDPATH='' cd -- "${output_dir}" && pwd)
[[ -f ${input} && ! -L ${input} ]] || {
  printf 'immutable Web admission input must be a regular file\n' >&2
  exit 66
}
community=$(node --input-type=module - "${repo_root}/ci/release_web_controls.mjs" "${input}" <<'NODE'
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const { webCommunityInScope } = await import(pathToFileURL(process.argv[2]).href);
console.log(webCommunityInScope(JSON.parse(readFileSync(process.argv[3], "utf8"))) ? "1" : "0");
NODE
)
executable_paths=("${ait_bin}")
if [[ ${community} == 1 ]]; then
  for required in community_bin community_cli_bin chrome; do [[ ${!required} == /* ]] || usage; done
  executable_paths+=("${community_bin}" "${community_cli_bin}" "${chrome}")
else
  [[ -z ${community_bin}${community_cli_bin}${chrome} ]] || { printf 'Community differs from immutable paused scope\n' >&2; exit 65; }
fi
[[ -z ${personal_bin} ]] || executable_paths+=("${personal_bin}")
[[ -z ${server_bin} ]] || executable_paths+=("${server_bin}")
for executable_path in "${executable_paths[@]}"; do
  [[ -f ${executable_path} && ! -L ${executable_path} && -x ${executable_path} ]] || {
    printf 'required executable is unavailable: %s\n' "${executable_path}" >&2
    exit 69
  }
done
for command_name in jq awk sed find cp curl lsof ps; do
  command -v "${command_name}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command_name}" >&2
    exit 69
  }
done
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else printf 'no SHA-256 utility is available\n' >&2; exit 69
  fi
}

evidence_dir=${output_dir}/evidence
receipt_dir=${output_dir}/receipts
log_dir=${output_dir}/private-logs
result_archive=${output_dir}/results
mkdir -p -- "${evidence_dir}" "${receipt_dir}" "${log_dir}" "${result_archive}"
chmod 700 "${log_dir}"

input_sha=$(sha256_file "${input}")
release_id=$(jq -er '.release.id' "${input}")
seed=$(jq -er '.execution.random_seed | select(type == "number" and . > 0)' "${input}")
groups=$(jq -er '.execution.random_groups | select(. == 3000)' "${input}")
input_ait=$(jq -er '.executables.ait.path' "${input}")
input_personal=$(jq -r '.executables.personal.path // ""' "${input}")
if jq -e '.executables | has("personal")' "${input}" >/dev/null; then
  [[ ${personal_bin} == /* ]] || usage
else
  jq -e '.release.version == "1.2.0" or (.release.version == "1.2.1" and .release.channel == "stable")' "${input}" >/dev/null || {
    printf 'Personal omission is authorized only for 1.2.0 or stable 1.2.1\n' >&2
    exit 65
  }
  [[ -z ${personal_bin} ]] || { printf 'Personal differs from immutable retained scope\n' >&2; exit 65; }
  [[ -n ${server_bin} ]] || { printf 'retained native coverage requires the bound --server-bin input\n' >&2; exit 64; }
fi
personal_args=()
[[ -z ${personal_bin} ]] || personal_args=(--personal-bin "${personal_bin}")
input_community=$(jq -r '.executables.community.path // ""' "${input}")
input_community_cli=$(jq -r '.executables.community_cli.path // ""' "${input}")
[[ ${input_ait} == "${ait_bin}" && ${input_personal} == "${personal_bin}" && \
   ${input_community} == "${community_bin}" && ${input_community_cli} == "${community_cli_bin}" ]] || {
  printf 'Web admission executable paths differ from the immutable input\n' >&2
  exit 65
}

verify_binary() {
  local selector=$1
  local executable_path=$2
  local expected actual
  expected=$(jq -er ".executables.${selector}.sha256" "${input}")
  actual=$(sha256_file "${executable_path}")
  [[ ${actual} == "${expected}" ]] || {
    printf 'Web admission binary drifted: %s\n' "${selector}" >&2
    exit 65
  }
}
verify_binary ait "${ait_bin}"
[[ -z ${personal_bin} ]] || verify_binary personal "${personal_bin}"
[[ -z ${community_bin} ]] || verify_binary community "${community_bin}"
[[ -z ${community_cli_bin} ]] || verify_binary community_cli "${community_cli_bin}"
[[ -z ${server_bin} ]] || verify_binary server "${server_bin}"

validate_web_evidence() {
  node --input-type=module - "${repo_root}/ci/release_web_controls.mjs" "${input}" "$1" "$2" "$3" "${repository}" <<'NODE'
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
const controls = await import(pathToFileURL(process.argv[2]).href);
const input = JSON.parse(readFileSync(process.argv[3], "utf8"));
const evidence = JSON.parse(readFileSync(process.argv[5], "utf8"));
if (process.argv[4] === "browser") controls.validateWebBrowser(evidence, input, process.argv[6]);
else controls.validateWebDirect(evidence, input, process.argv[6], process.argv[7], path.dirname(process.argv[5]));
NODE
}

# Existing receipts or a bound conductor input are authority, never a missing
# phase. Reject changed scope/input before any Server or WT process can start.
if ! composed=$(node --input-type=module - "${repo_root}/ci/release_web_controls.mjs" "${input}" "${repository}" "${output_dir}" <<'NODE'
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const { validateWebOperation } = await import(pathToFileURL(process.argv[2]).href);
console.log(validateWebOperation(process.argv[3], JSON.parse(readFileSync(process.argv[3], "utf8")), process.argv[4], process.argv[5]) ? '1' : '0');
NODE
)
then
  printf 'Web admission HOLD: original operation/input/evidence binding differs; nothing is replayed\n' >&2
  exit 75
fi

# Companion selection comes exclusively from the immutable input, including
# for the existing WT leaf scripts that also accept environment inputs.
export AIT_WEB_TEST_PERSONAL_BIN="${personal_bin}"
export AIT_WEB_TEST_COMMUNITY_BIN="${community_bin}"
export AIT_WEB_TEST_COMMUNITY_CLI_BIN="${community_cli_bin}"
export AIT_WEB_TEST_BROWSER_EVIDENCE=''
export AIT_WEB_TEST_SERVER_BIN="${server_bin}"

receipt_path() { printf '%s/%s.json\n' "${receipt_dir}" "$1"; }
receipt_complete() {
  local step=$1
  local receipt
  receipt=$(receipt_path "${step}")
  [[ -f ${receipt} && ! -L ${receipt} ]] || return 1
  jq -e --arg input_sha "${input_sha}" --arg step "${step}" \
    '.contract == "ait.release.web-admission-step/v1" and .status == "pass" and .input_sha256 == $input_sha and .step == $step' \
    "${receipt}" >/dev/null
}
write_receipt() {
  local step=$1
  local evidence_path=$2
  local receipt evidence_sha
  receipt=$(receipt_path "${step}")
  [[ ! -e ${receipt} ]] || {
    printf 'Web admission receipt already exists: %s\n' "${receipt}" >&2
    exit 73
  }
  evidence_sha=$(sha256_file "${evidence_path}")
  jq -n --arg contract ait.release.web-admission-step/v1 --arg status pass \
    --arg input_sha256 "${input_sha}" --arg step "${step}" \
    --arg evidence_path "${evidence_path}" --arg evidence_sha256 "${evidence_sha}" \
    --arg completed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{contract:$contract,status:$status,input_sha256:$input_sha256,step:$step,
      evidence:{path:$evidence_path,sha256:$evidence_sha256},completed_at:$completed_at}' \
    >"${receipt}.new"
  chmod 600 "${receipt}.new"
  mv "${receipt}.new" "${receipt}"
}
print_status() {
  local step
  for step in foundation solo_local solo_remote randomized_3000 closeout; do
    if receipt_complete "${step}"; then printf '%s: complete\n' "${step}"
    else printf '%s: pending\n' "${step}"
    fi
  done
}
verify_admission() {
  if ! node "${repo_root}/ci/release_web_admission_verify.mjs" "${input}" "${output_dir}/web-admission.json" "${repository}"; then
    printf 'Web admission HOLD: existing evidence failed verification; preserve its receipts/results without replay\n' >&2
    return 75
  fi
}
if [[ ${composed} == 1 ]]; then
  # The bounded commands already ran under WT ownership. Consume their native
  # action output bindings before any listener/leaf code below can execute.
  node --input-type=module - "${repo_root}/ci/release_web_controls.mjs" "${input}" "${repository}" "${output_dir}" "${mode}" <<'NODE'
import assert from 'node:assert/strict';
import { readFileSync, existsSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
const [controls, file, repository, root, mode] = process.argv.slice(2);
const { validateComposedWebCoverage } = await import(pathToFileURL(controls));
const bytes = readFileSync(file), input = JSON.parse(bytes);
validateComposedWebCoverage(file, input, repository, root);
if (mode === 'run') {
  const output = path.join(root, 'web-admission.json');
  const value = { contract:'ait.release.web-admission/v1', status:'admitted',
    input_sha256:createHash('sha256').update(bytes).digest('hex'), release_id:input.release.id,
    direct:{solo_local:'pass',solo_remote:'pass'}, browser:{},
    randomized:{seed:input.execution.random_seed,groups:3000,status:'pass'},
    final_authority:{mode:'solo_remote',repository_index:23,namespace:'WT',status:'clean'},
    public_cli_identity:{internal_change_argv_count:0} };
  if (existsSync(output)) {
    const previous = JSON.parse(readFileSync(output));
    for (const [key, expected] of Object.entries(value)) assert.deepEqual(previous[key], expected, 'existing admission differs');
  } else writeFileSync(output, JSON.stringify({...value, completed_at:new Date().toISOString()})+'\n', {mode:0o600,flag:'wx'});
}
NODE
  if [[ ${mode} == run ]]; then
    if ! receipt_complete closeout; then write_receipt closeout "${output_dir}/web-admission.json"; fi
    verify_admission
  elif receipt_complete closeout; then verify_admission
  fi
  printf 'reviewed assertion coverage: complete\n'
  exit 0
fi
if [[ ${mode} == status ]]; then
  if receipt_complete closeout; then verify_admission; fi
  print_status
  exit 0
fi

owned_server_pid=''
settings_baseline=${evidence_dir}/starting-config.json
[[ -f ${settings_baseline} && ! -L ${settings_baseline} ]] || {
  printf 'original Web configuration capture is unavailable\n' >&2
  exit 65
}
node "${repo_root}/ci/release_web_controls.mjs" "${ait_bin}" "${repository}" "${settings_baseline}"
set_mode() {
  # These are scenario identifiers. Authority is selected explicitly by each
  # acceptance command; the child runner owns temporary settings and restore.
  [[ $1 == solo_local || $1 == solo_remote ]] || return 64
  node "${repo_root}/ci/release_web_controls.mjs" "${ait_bin}" "${repository}" "${settings_baseline}"
}
cleanup_runtime() {
  if [[ -n ${owned_server_pid} ]]; then
    kill "${owned_server_pid}" 2>/dev/null || true
    wait "${owned_server_pid}" 2>/dev/null || true
    owned_server_pid=''
  fi
  node "${repo_root}/ci/release_web_controls.mjs" "${ait_bin}" "${repository}" "${settings_baseline}"
}
trap 'operation_exit=$?; trap - EXIT; if ! cleanup_runtime; then exit 65; fi; exit "${operation_exit}"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

if receipt_complete closeout; then verify_admission; print_status; exit 0; fi
server_address=${server_url#http://}
server_address=${server_address%/}
server_port=${server_address##*:}
listener_pid=$(lsof -nP -iTCP:"${server_port}" -sTCP:LISTEN -t 2>/dev/null | head -n 1 || true)
if [[ -n ${listener_pid} && -n ${server_bin} ]]; then
  listener_command=$(ps -p "${listener_pid}" -o command=)
  [[ ${listener_command} == "${server_bin} --data ${server_data} --listen ${server_address}" ]] || {
    printf 'Web admission listener is not the bound candidate server\n' >&2
    exit 65
  }
elif [[ -z ${listener_pid} && -n ${server_bin} ]]; then
  "${server_bin}" --data "${server_data}" --listen "${server_address}" \
    >"${log_dir}/candidate-server.log" 2>&1 &
  owned_server_pid=$!
  for attempt in {1..100}; do
    curl -fsS "${server_url}healthz" >/dev/null 2>&1 && break
    kill -0 "${owned_server_pid}" 2>/dev/null || {
      printf 'candidate server exited before readiness\n' >&2
      exit 65
    }
    [[ ${attempt} -lt 100 ]] || {
      printf 'candidate server readiness timed out\n' >&2
      exit 75
    }
    sleep 0.1
  done
elif [[ -z ${listener_pid} ]]; then
  printf 'Web admission server is not listening\n' >&2
  exit 69
fi
curl -fsS "${server_url}healthz" >/dev/null

archive_latest_direct_result() {
  local phase=$1
  local marker=$2
  local destination=$3
  local summaries=()
  while IFS= read -r summary_path; do summaries+=("${summary_path}"); done < <(
    find "${repository}/.ait-runtime/direct-test-results" -type f -name summary.json -newer "${marker}" -print | LC_ALL=C sort
  )
  [[ ${#summaries[@]} -eq 1 ]] || {
    printf 'expected one new %s direct result; found %s\n' "${phase}" "${#summaries[@]}" >&2
    exit 65
  }
  [[ ! -e ${destination} ]] || {
    printf 'direct result archive already exists: %s\n' "${destination}" >&2
    exit 73
  }
  cp -R "$(dirname -- "${summaries[0]}")" "${destination}"
  validate_web_evidence direct "${destination}/summary.json" "${phase}"
}

# Each collection worker keeps errexit inside its own process. Calling a
# multi-command phase directly from an if/|| list would disable that protection.
collection_failed=0
collection_blocked=0
collection_preflight() {
  local live_status live_queue current_listener
  set_mode solo_remote || return 75
  verify_binary ait "${ait_bin}"
  [[ -z ${personal_bin} ]] || verify_binary personal "${personal_bin}"
  [[ -z ${community_bin} ]] || verify_binary community "${community_bin}"
  [[ -z ${community_cli_bin} ]] || verify_binary community_cli "${community_cli_bin}"
  [[ -z ${server_bin} ]] || verify_binary server "${server_bin}"
  curl -fsS "${server_url}healthz" >/dev/null || return 75
  if [[ -n ${server_bin} ]]; then
    current_listener=$(lsof -nP -iTCP:"${server_port}" -sTCP:LISTEN -t) || return 75
    [[ -n ${current_listener} && ${current_listener} != *$'\n'* ]] || return 75
    [[ $(ps -p "${current_listener}" -o command=) == "${server_bin} --data ${server_data} --listen ${server_address}" ]] || return 75
  fi
  live_status=$(cd "${repository}" && "${ait_bin}" status --json --full) || return 75
  live_queue=$(cd "${repository}" && "${ait_bin}" queue summary --remote origin --json) || return 75
  jq -e '.current_line == "main" and .workspace_dirty == false and .workspace_changed_count == 0 and
    .worktree_name == null and .worktree_hygiene.manual_review_candidate_count == 0 and
    .worktree_hygiene.stale_count == 0 and .reconciliation.total_finding_count == 0' <<<"${live_status}" >/dev/null || return 75
  jq -e '.summary.attention_required_count == 0 and .summary.dirty_worktree_count == 0 and
    .remote.reviewer_inbox.count == 0' <<<"${live_queue}" >/dev/null || return 75
}
collect_step() {
  local step=$1 phase_status
  shift
  if receipt_complete "${step}"; then return 0; fi
  if [[ ${step} != foundation ]] && ! receipt_complete foundation; then
    printf 'BLOCKED %s: foundation did not pass; command not executed\n' "${step}" >&2
    collection_blocked=1
    return 0
  fi
  set +e
  (trap - EXIT; set -e; collection_preflight)
  phase_status=$?
  set -e
  if [[ ${phase_status} != 0 ]]; then
    printf 'BLOCKED %s: authority, restoration or frozen-input preflight failed (exit %s); command not executed\n' "${step}" "${phase_status}" >&2
    collection_blocked=1
    return 0
  fi
  set +e
  (trap - EXIT; set -e; "$@")
  phase_status=$?
  set -e
  if [[ ${phase_status} == 75 ]]; then
    printf 'BLOCKED %s: original phase requires recovery/review (exit 75); preserve its results\n' "${step}" >&2
    collection_blocked=1
  elif [[ ${phase_status} != 0 ]]; then
    printf 'FAIL %s: original phase returned exit %s; preserve its results\n' "${step}" "${phase_status}" >&2
    collection_failed=1
  fi
  return 0
}
collect_foundation_check() {
  local check=$1 check_status
  shift
  set +e
  (trap - EXIT; set -e; "$@")
  check_status=$?
  set -e
  if [[ ${check_status} != 0 ]]; then
    printf 'FAIL foundation/%s: exit %s\n' "${check}" "${check_status}" >&2
    foundation_failed=1
  fi
  return 0
}

run_native_direct() (
  # WT's full-Web wrapper requires Community. Reuse its original Native leaves
  # and output contract under the same configuration/recovery guard instead.
  local workflow_mode=$1 phase_label=$2 original_results=${3:-}
  awt_script_dir=${repository}/acceptance
  . "${awt_script_dir}/lib.sh"
  awt_validate_repository "${repository}" "${ait_bin}" "${workflow_mode}"
  cd "${repository}"
  export AIT_WEB_TEST_REPOSITORY_ROOT="${repository}" AIT_WEB_TEST_AIT_BIN="${ait_bin}" AIT_WEB_TEST_EXPECTED_MODE="${workflow_mode}"
  local initial_digest final_digest command_nodes
  if [[ -n ${original_results} ]]; then
    [[ ${original_results} == "${repository}/.ait-runtime/direct-test-results/${workflow_mode}-${phase_label}-"* &&
       ${original_results##*/} != *[!a-zA-Z0-9_-]* && -d ${original_results} && ! -L ${original_results} ]] || return 75
    export AIT_WEB_TEST_RESULTS_DIR=${original_results}
    awt_config_baseline=${original_results}/direct-run.config-original.json
    [[ -f ${awt_config_baseline} && ! -L ${awt_config_baseline} ]] || return 75
    [[ -f ${original_results}/.initial-authority-digest && ! -L ${original_results}/.initial-authority-digest ]] || return 75
    # Never recapture settings or replay an original leaf on resume.
    initial_digest=$(cat "${original_results}/.initial-authority-digest")
  else
    export AIT_WEB_TEST_RESULTS_DIR="${repository}/.ait-runtime/direct-test-results/${workflow_mode}-${phase_label}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
    mkdir -p "$(dirname -- "${AIT_WEB_TEST_RESULTS_DIR}")"
    mkdir "${AIT_WEB_TEST_RESULTS_DIR}"
    printf '%s\n' "${AIT_WEB_TEST_RESULTS_DIR}" >"${marker}"
    : >"${AIT_WEB_TEST_RESULTS_DIR}/cases.tsv"
    awt_config_capture direct-run
    awt_config_guard awt_config_restore
    initial_digest=$(awt_state_digest)
    printf '%s\n' "${initial_digest}" >"${AIT_WEB_TEST_RESULTS_DIR}/.initial-authority-digest"
    awt_run_case inventory.ait "${awt_script_dir}/check-inventory.sh" "${ait_bin}"
  fi
  if [[ ${phase_label} == inventory ]]; then
    final_digest=$(awt_state_digest)
    awt_inventory_unchanged() { [[ ${initial_digest} == "${final_digest}" ]]; }
    awt_run_case inventory.authority-unchanged awt_inventory_unchanged
  else
    if [[ -z ${original_results} ]]; then
      "${awt_script_dir}/run-cli-contracts.sh" ait "${ait_bin}" "${awt_script_dir}/manifests/ait-command-matrix.tsv"
      "${awt_script_dir}/run-ait-solo-local.sh"
      if [[ ${workflow_mode} == solo_remote ]]; then "${awt_script_dir}/run-ait-solo-remote-lifecycle.sh"; fi
    fi
    awt_final_mode() { awt_validate_repository "${repository}" "${ait_bin}" "${workflow_mode}" && awt_config_assert_restored; }
    awt_run_case "final.${phase_label}-mode" awt_final_mode
  fi
  final_digest=$(awt_state_digest)
  command_nodes=$(awk -F '\t' '!/^#/ && NF { count += 1 } END { print count + 0 }' "${awt_script_dir}/manifests/ait-command-matrix.tsv")
  node --input-type=module - "${repo_root}/ci/release_web_controls.mjs" "${input}" "${repository}" "${AIT_WEB_TEST_RESULTS_DIR}" \
    "${workflow_mode}" "${phase_label}" "${initial_digest}" "${final_digest}" "${command_nodes}" <<'NODE'
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
const [controls, inputFile, repository, result, mode, phase, initial, final, commands] = process.argv.slice(2);
const { validateWebDirect } = await import(pathToFileURL(controls).href);
const value = { contract: "ait-web-test.direct-run.v1", status: "pass", mode, phase,
  initial_authority_digest: initial, final_authority_digest: final, command_nodes: Number(commands), personal_surfaces: 0, community_surfaces: 0 };
validateWebDirect(value, JSON.parse(readFileSync(inputFile)), phase, repository, result);
writeFileSync(path.join(result, "summary.json"), JSON.stringify(value) + "\n", { mode: 0o600, flag: "wx" });
NODE
)


run_foundation() {
if ! receipt_complete foundation; then
  set_mode solo_remote
  foundation_log=${log_dir}/foundation.log
  marker=${log_dir}/foundation.marker
  : >"${marker}"
  (
    candidate_path=$(dirname -- "${ait_bin}")
    export PATH="${candidate_path}:${PATH}"
    cd "${repository}"
    foundation_failed=0
    if [[ ${community} == 0 ]]; then
      collect_foundation_check workflow-controls sh ./acceptance/workflow-controls-self-test.sh "${ait_bin}"
      collect_foundation_check package-lifecycle sh ./acceptance/package-lifecycle-self-test.sh
      collect_foundation_check native-formal sh ./acceptance/native-formal-self-test.sh
      collect_foundation_check command-discovery ./acceptance/command-discovery-self-test.sh "${ait_bin}"
      collect_foundation_check randomized ./acceptance/randomized-self-test.sh
      [[ ${foundation_failed} == 0 ]] || { printf 'BLOCKED inventory: foundation checks failed; command not executed\n' >&2; exit 1; }
      collection_preflight
      run_native_direct solo_remote inventory
    else
      collect_foundation_check self-test ./acceptance/self-test.sh --repository "${repository}" --mode solo_remote
      collect_foundation_check inventory ./acceptance/check-inventory.sh "${ait_bin}"
      collect_foundation_check surfaces ./acceptance/check-surfaces.sh
      [[ ${foundation_failed} == 0 ]] || { printf 'BLOCKED inventory: foundation checks failed; command not executed\n' >&2; exit 1; }
      collection_preflight
      ./acceptance/run.sh --repository "${repository}" --mode solo_remote \
        --phase inventory --ait "${ait_bin}"
    fi
  ) >"${foundation_log}" 2>&1
  archive_latest_direct_result inventory "${marker}" "${result_archive}/foundation"
  write_receipt foundation "${result_archive}/foundation/summary.json"
  printf 'foundation: complete\n'
fi

}

run_direct() {
  local workflow_mode=$1
  local phase_name=$2
  local phase_label=$3
  local browser_scope=$4
  local browser_remote=${5:-}
  local browser_authority=(--scope "${browser_scope}")
  [[ ${browser_scope} != remote ]] || browser_authority+=(--remote "${browser_remote}")
  local evidence_relative evidence_path marker direct_log
  if receipt_complete "${phase_name}"; then return
  fi
  set_mode "${workflow_mode}"
  if [[ ${community} == 1 ]]; then
    evidence_relative=$(jq -er ".browser_evidence.${workflow_mode}.path" "${input}")
    evidence_path=${output_dir}/${evidence_relative}
    if [[ ! -e ${evidence_path} ]]; then
      if [[ -f ${log_dir}/${phase_name}-browser.log ]] &&
        grep -E '^Personal browser (Task-start|workflow) dispatch prepared; do not repeat this attempt$' \
          "${log_dir}/${phase_name}-browser.log" >/dev/null; then
        printf 'browser mutation attempt requires exact-authority reconciliation; retain %s\n' \
          "${log_dir}/${phase_name}-browser.log" >&2
        exit 75
      fi
      node "${repo_root}/ci/release_web_browser_evidence.mjs" \
        --repository "${repository}" --mode "${workflow_mode}" --ait "${ait_bin}" \
        ${personal_args[@]+"${personal_args[@]}"} --community-bin "${community_bin}" \
        --community-cli-bin "${community_cli_bin}" --server-url "${server_url}" \
        --chrome "${chrome}" --output "${evidence_path}" "${browser_authority[@]}" \
        >"${log_dir}/${phase_name}-browser.log" 2>&1
    fi
    validate_web_evidence browser "${evidence_path}" "${workflow_mode}"
  fi
  marker=${log_dir}/${phase_name}.marker
  direct_log=${log_dir}/${phase_name}.log
  if [[ -e ${direct_log} ]]; then
    # Reviewer HOLD resumes inside WT's original results directory. Replaying
    # run.sh would create new Local history and another package preparation.
    [[ -f ${marker} && ! -L ${marker} ]] || { printf 'existing WT phase marker is unavailable; retain its original results\n' >&2; exit 75; }
    local completed=()
    while IFS= read -r summary; do completed+=("${summary}"); done < <(
      find "${repository}/.ait-runtime/direct-test-results" -type f -name summary.json -newer "${marker}" -print | LC_ALL=C sort
    )
    if [[ ${#completed[@]} -eq 0 && ${community} == 0 ]]; then
      # An operator resumes the original WT leaf after real review. Only
      # complete original rows can now produce the phase summary; no leaf runs.
      local original_results
      original_results=$(cat "${marker}")
      [[ -n ${original_results} ]] || { printf 'original Native result directory is unavailable; nothing is replayed\n' >&2; exit 75; }
      run_native_direct "${workflow_mode}" "${phase_label}" "${original_results}" >"${direct_log}.resume" 2>&1 || {
        printf 'original Native evidence remains incomplete; resume its original WT leaf: %s\n' "${original_results}" >&2
        exit 75
      }
    elif [[ ${#completed[@]} -ne 1 ]]; then
      printf 'existing WT phase is incomplete or ambiguous; resume its original results directory and leaf, never run.sh: %s\n' "${direct_log}" >&2
      exit 75
    fi
    archive_latest_direct_result "${phase_label}" "${marker}" "${result_archive}/${phase_name}"
    write_receipt "${phase_name}" "${result_archive}/${phase_name}/summary.json"
    return
  fi
  : >"${marker}"
  if [[ ${community} == 0 ]]; then
    run_native_direct "${workflow_mode}" "${phase_label}" >"${direct_log}" 2>&1
  else (
    cd "${repository}"
    AIT_WEB_TEST_SERVER_BIN="${server_bin}" ./acceptance/run.sh --repository "${repository}" --mode "${workflow_mode}" \
      --phase "${phase_label}" --ait "${ait_bin}" ${personal_args[@]+"${personal_args[@]}"} \
      --community-bin "${community_bin}" --community-cli-bin "${community_cli_bin}" \
      --browser-evidence "${evidence_path}"
  ) >"${direct_log}" 2>&1
  fi
  archive_latest_direct_result "${phase_label}" "${marker}" "${result_archive}/${phase_name}"
  write_receipt "${phase_name}" "${result_archive}/${phase_name}/summary.json"
  printf '%s: complete\n' "${phase_name}"
}

run_randomized() {
if ! receipt_complete randomized_3000; then
  set_mode solo_remote
  random_runtime=${repository}/.ait-runtime/random-command-results/release-${release_id}-${seed}
  if [[ -e ${random_runtime} && ! -f ${random_runtime}/summary.json ]]; then
    printf 'incomplete randomized evidence requires recovery: %s\n' "${random_runtime}" >&2
    exit 75
  fi
  if [[ ! -e ${random_runtime} ]]; then
    (
      cd "${repository}"
      ./acceptance/run-randomized-commands.sh --repository "${repository}" \
        --mode solo_remote --groups "${groups}" --seed "${seed}" --ait "${ait_bin}" \
        --output-dir "${random_runtime}"
    ) >"${log_dir}/randomized-3000.log" 2>&1
  fi
  jq -e --argjson groups "${groups}" --argjson seed "${seed}" '
    .contract == "ait-web-test.random-command-run.v1" and .status == "pass" and
    .mode == "solo_remote" and .repository_index == 23 and .namespace == "WT" and
    .seed == $seed and .requested_groups == $groups and .executed_groups == $groups and
    .retirement_invocations == 0 and .initial_authority_digest == .final_authority_digest
  ' "${random_runtime}/summary.json" >/dev/null
  [[ ! -e ${result_archive}/randomized_3000 ]] || {
    printf 'randomized result archive already exists\n' >&2
    exit 73
  }
  cp -R "${random_runtime}" "${result_archive}/randomized_3000"
  write_receipt randomized_3000 "${result_archive}/randomized_3000/summary.json"
  printf 'randomized_3000: complete\n'
fi

}

# A failed assertion does not suppress an independent phase. Every phase
# rechecks clean authority and immutable inputs before any further mutation.
collect_step foundation run_foundation
collect_step solo_local run_direct solo_local solo_local solo-local local
collect_step solo_remote run_direct solo_remote solo_remote solo-remote remote origin
collect_step randomized_3000 run_randomized
if [[ ${collection_failed} != 0 || ${collection_blocked} != 0 ]]; then
  printf 'Web admission collection incomplete: failures=%s blocked=%s; no admission is issued\n' "${collection_failed}" "${collection_blocked}" >&2
  print_status
  [[ ${collection_failed} == 0 ]] || exit 1
  exit 75
fi

if ! receipt_complete closeout; then
  set_mode solo_remote
  verify_binary ait "${ait_bin}"
  [[ -z ${personal_bin} ]] || verify_binary personal "${personal_bin}"
  [[ -z ${community_bin} ]] || verify_binary community "${community_bin}"
  [[ -z ${community_cli_bin} ]] || verify_binary community_cli "${community_cli_bin}"
  [[ -z ${server_bin} ]] || verify_binary server "${server_bin}"
  (cd "${repository}" && "${ait_bin}" status --json --full) >"${evidence_dir}/final-status.json"
  (cd "${repository}" && "${ait_bin}" config show --json) >"${evidence_dir}/final-config.json"
  (cd "${repository}" && "${ait_bin}" queue summary --remote origin --json) >"${evidence_dir}/final-queue.json"
  jq -e '.current_line == "main" and .workspace_dirty == false and .workspace_changed_count == 0 and
    .worktree_name == null and .worktree_hygiene.manual_review_candidate_count == 0 and
    .worktree_hygiene.stale_count == 0 and .reconciliation.total_finding_count == 0' \
    "${evidence_dir}/final-status.json" >/dev/null
  jq -e '.repo_name == "ait-web-test" and .repository_index == 23 and .is_worktree == false and
    .id_namespace_prefix.value == "WT"' \
    "${evidence_dir}/final-config.json" >/dev/null
  cleanup_runtime
  jq -e '.summary.attention_required_count == 0 and .summary.dirty_worktree_count == 0 and
    .remote.reviewer_inbox.count == 0' "${evidence_dir}/final-queue.json" >/dev/null
  if find "${result_archive}" -type f -name '*.argv' -exec grep -EH '[A-Z]+T-[0-9]{4}/C-[0-9]{2}' {} + \
    >"${log_dir}/internal-change-argv.tsv" 2>/dev/null; then
    printf 'Web admission found an internal Change ID in recorded argv\n' >&2
    exit 65
  fi
  browser_hashes='{}'
  if [[ ${community} == 1 ]]; then
    local_browser=${output_dir}/$(jq -er '.browser_evidence.solo_local.path' "${input}")
    remote_browser=${output_dir}/$(jq -er '.browser_evidence.solo_remote.path' "${input}")
    browser_hashes=$(jq -n --arg local "$(sha256_file "${local_browser}")" --arg remote "$(sha256_file "${remote_browser}")" \
      '{solo_local_sha256:$local,solo_remote_sha256:$remote}')
  fi
  final_output=${output_dir}/web-admission.json
  jq -n --arg contract ait.release.web-admission/v1 --arg status admitted \
    --arg input_sha256 "${input_sha}" --arg release_id "${release_id}" \
    --argjson browser "${browser_hashes}" \
    --argjson seed "${seed}" --argjson groups "${groups}" \
    --arg completed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{contract:$contract,status:$status,input_sha256:$input_sha256,release_id:$release_id,
      direct:{solo_local:"pass",solo_remote:"pass"},
      browser:$browser,
      randomized:{seed:$seed,groups:$groups,status:"pass"},
      final_authority:{mode:"solo_remote",repository_index:23,namespace:"WT",status:"clean"},
      public_cli_identity:{internal_change_argv_count:0},completed_at:$completed_at}' \
    >"${final_output}.new"
  chmod 600 "${final_output}.new"
  mv "${final_output}.new" "${final_output}"
  write_receipt closeout "${final_output}"
fi

verify_admission
print_status
