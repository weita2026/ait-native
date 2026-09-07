#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' 'usage: release_web_admission.sh {run|status} --input <absolute-json> --repository <absolute-path> --ait <absolute-path> --personal-bin <absolute-path> --community-bin <absolute-path> --community-cli-bin <absolute-path> --server-url <loopback-url> [--server-bin <absolute-path> --server-data <absolute-path>] --chrome <absolute-path> --output-dir <absolute-path>' >&2
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
for required in input repository ait_bin personal_bin community_bin community_cli_bin server_url chrome output_dir; do
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
[[ ${input} == "${output_dir}"/* && -f ${input} && ! -L ${input} ]] || {
  printf 'immutable Web admission input must be a regular file below output-dir\n' >&2
  exit 66
}
executable_paths=("${ait_bin}" "${personal_bin}" "${community_bin}" "${community_cli_bin}" "${chrome}")
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
input_personal=$(jq -er '.executables.personal.path' "${input}")
input_community=$(jq -er '.executables.community.path' "${input}")
input_community_cli=$(jq -er '.executables.community_cli.path' "${input}")
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
verify_binary personal "${personal_bin}"
verify_binary community "${community_bin}"
verify_binary community_cli "${community_cli_bin}"
[[ -z ${server_bin} ]] || verify_binary server "${server_bin}"

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
if [[ ${mode} == status ]]; then
  print_status
  exit 0
fi

owned_server_pid=''
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

current_mode() {
  (cd "${repository}" && "${ait_bin}" config show --json) | jq -er '.workflow_mode.value'
}
set_mode() {
  local requested=$1
  [[ $(current_mode) == "${requested}" ]] && return
  # The explicit cwd is a release invariant. AIT config is Repository-scoped.
  (cd "${repository}" && "${ait_bin}" config set --workflow-mode "${requested}" --json) \
    >"${log_dir}/set-mode-${requested}.json"
  [[ $(current_mode) == "${requested}" ]] || {
    printf 'failed to set ait-web-test workflow mode to %s\n' "${requested}" >&2
    exit 65
  }
}
restore_remote() {
  if [[ $(current_mode 2>/dev/null || true) != solo_remote ]]; then
    (cd "${repository}" && "${ait_bin}" config set --workflow-mode solo_remote --json) \
      >"${log_dir}/restore-solo-remote.json" 2>&1 || true
  fi
}
cleanup_runtime() {
  restore_remote
  if [[ -n ${owned_server_pid} ]]; then
    kill "${owned_server_pid}" 2>/dev/null || true
    wait "${owned_server_pid}" 2>/dev/null || true
  fi
}
trap cleanup_runtime EXIT HUP INT TERM

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
  jq -e --arg phase "${phase}" '.status == "pass" and .phase == $phase' \
    "${destination}/summary.json" >/dev/null
}

if ! receipt_complete foundation; then
  set_mode solo_remote
  foundation_log=${log_dir}/foundation.log
  marker=${log_dir}/foundation.marker
  : >"${marker}"
  (
    candidate_path=$(dirname -- "${ait_bin}")
    export PATH="${candidate_path}:${PATH}"
    cd "${repository}"
    ./acceptance/self-test.sh --repository "${repository}" --mode solo_remote
    ./acceptance/check-inventory.sh "${ait_bin}"
    ./acceptance/check-surfaces.sh
    ./acceptance/run.sh --repository "${repository}" --mode solo_remote \
      --phase inventory --ait "${ait_bin}"
  ) >"${foundation_log}" 2>&1
  archive_latest_direct_result inventory "${marker}" "${result_archive}/foundation"
  write_receipt foundation "${result_archive}/foundation/summary.json"
  printf 'foundation: complete\n'
fi

run_direct() {
  local workflow_mode=$1
  local phase_name=$2
  local phase_label=$3
  local evidence_relative evidence_path marker direct_log
  if receipt_complete "${phase_name}"; then return
  fi
  set_mode "${workflow_mode}"
  evidence_relative=$(jq -er ".browser_evidence.${workflow_mode}.path" "${input}")
  evidence_path=${output_dir}/${evidence_relative}
  if [[ ! -e ${evidence_path} ]]; then
    node "${repo_root}/ci/release_web_browser_evidence.mjs" \
      --repository "${repository}" --mode "${workflow_mode}" --ait "${ait_bin}" \
      --personal-bin "${personal_bin}" --community-bin "${community_bin}" \
      --community-cli-bin "${community_cli_bin}" --server-url "${server_url}" \
      --chrome "${chrome}" --output "${evidence_path}" \
      >"${log_dir}/${phase_name}-browser.log" 2>&1
  fi
  local expected_personal expected_community expected_community_cli
  expected_personal=$(jq -er '.executables.personal.sha256' "${input}")
  expected_community=$(jq -er '.executables.community.sha256' "${input}")
  expected_community_cli=$(jq -er '.executables.community_cli.sha256' "${input}")
  jq -e --arg mode "${workflow_mode}" --arg personal "${expected_personal}" \
    --arg community "${expected_community}" --arg community_cli "${expected_community_cli}" '
    .contract == "ait-web-test.browser-evidence.v1" and .mode == $mode and
    .personal.binary_sha256 == $personal and .personal.browser_desktop == "pass" and
    .personal["browser_mobile-390x844"] == "pass" and .personal.browser_console == "pass" and
    .personal.console_warning_or_error_count == 0 and .personal.failed_request_count == 0 and
    .personal.external_request_count == 0 and
    .community.binary_sha256 == $community and .community.companion_binary_sha256 == $community_cli and
    .community.browser_desktop == "pass" and .community["browser_mobile-390x844"] == "pass" and
    .community.browser_console == "pass" and .community.console_warning_or_error_count == 0 and
    .community.failed_request_count == 0 and .community.external_request_count == 0 and
    .community.catalog.definition_count == 25 and .community.catalog.enabled_count == 0 and
    .community.catalog.buttons == 0 and .community.catalog.forms == 0 and
    .community.catalog.executable_action_links == 0 and
    .personal.responsive.document_horizontal_overflow == false and
    .community.responsive.document_horizontal_overflow == false
  ' "${evidence_path}" >/dev/null || {
    printf 'browser evidence does not satisfy the immutable %s binding\n' "${workflow_mode}" >&2
    exit 65
  }
  marker=${log_dir}/${phase_name}.marker
  : >"${marker}"
  direct_log=${log_dir}/${phase_name}.log
  (
    cd "${repository}"
    ./acceptance/run.sh --repository "${repository}" --mode "${workflow_mode}" \
      --phase "${phase_label}" --ait "${ait_bin}" --personal-bin "${personal_bin}" \
      --community-bin "${community_bin}" --community-cli-bin "${community_cli_bin}" \
      --browser-evidence "${evidence_path}"
  ) >"${direct_log}" 2>&1
  archive_latest_direct_result "${phase_label}" "${marker}" "${result_archive}/${phase_name}"
  write_receipt "${phase_name}" "${result_archive}/${phase_name}/summary.json"
  printf '%s: complete\n' "${phase_name}"
}

run_direct solo_local solo_local solo-local
run_direct solo_remote solo_remote solo-remote

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

if ! receipt_complete closeout; then
  set_mode solo_remote
  verify_binary ait "${ait_bin}"
  verify_binary personal "${personal_bin}"
  verify_binary community "${community_bin}"
  verify_binary community_cli "${community_cli_bin}"
  [[ -z ${server_bin} ]] || verify_binary server "${server_bin}"
  (cd "${repository}" && "${ait_bin}" status --json --full) >"${evidence_dir}/final-status.json"
  (cd "${repository}" && "${ait_bin}" config show --json) >"${evidence_dir}/final-config.json"
  (cd "${repository}" && "${ait_bin}" queue summary --remote origin --json) >"${evidence_dir}/final-queue.json"
  jq -e '.current_line == "main" and .workspace_dirty == false and .workspace_changed_count == 0 and
    .worktree_name == null and .worktree_hygiene.manual_review_candidate_count == 0 and
    .worktree_hygiene.stale_count == 0 and .reconciliation.total_finding_count == 0' \
    "${evidence_dir}/final-status.json" >/dev/null
  jq -e '.repo_name == "ait-web-test" and .repository_index == 23 and .is_worktree == false and
    .workflow_mode.value == "solo_remote" and .id_namespace_prefix.value == "WT"' \
    "${evidence_dir}/final-config.json" >/dev/null
  jq -e '.summary.attention_required_count == 0 and .summary.dirty_worktree_count == 0 and
    .remote.reviewer_inbox.count == 0' "${evidence_dir}/final-queue.json" >/dev/null
  if find "${result_archive}" -type f -name '*.argv' -exec grep -EH '[A-Z]+T-[0-9]{4}/C-[0-9]{2}' {} + \
    >"${log_dir}/internal-change-argv.tsv" 2>/dev/null; then
    printf 'Web admission found an internal Change ID in recorded argv\n' >&2
    exit 65
  fi
  local_browser=${output_dir}/$(jq -er '.browser_evidence.solo_local.path' "${input}")
  remote_browser=${output_dir}/$(jq -er '.browser_evidence.solo_remote.path' "${input}")
  local_browser_sha=$(sha256_file "${local_browser}")
  remote_browser_sha=$(sha256_file "${remote_browser}")
  final_output=${output_dir}/web-admission.json
  jq -n --arg contract ait.release.web-admission/v1 --arg status admitted \
    --arg input_sha256 "${input_sha}" --arg release_id "${release_id}" \
    --arg local_browser_sha256 "${local_browser_sha}" --arg remote_browser_sha256 "${remote_browser_sha}" \
    --argjson seed "${seed}" --argjson groups "${groups}" \
    --arg completed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{contract:$contract,status:$status,input_sha256:$input_sha256,release_id:$release_id,
      direct:{solo_local:"pass",solo_remote:"pass"},
      browser:{solo_local_sha256:$local_browser_sha256,solo_remote_sha256:$remote_browser_sha256},
      randomized:{seed:$seed,groups:$groups,status:"pass"},
      final_authority:{mode:"solo_remote",repository_index:23,namespace:"WT",status:"clean"},
      public_cli_identity:{internal_change_argv_count:0},completed_at:$completed_at}' \
    >"${final_output}.new"
  chmod 600 "${final_output}.new"
  mv "${final_output}.new" "${final_output}"
  write_receipt closeout "${final_output}"
  printf 'closeout: complete\n'
fi

trap - EXIT HUP INT TERM
print_status
