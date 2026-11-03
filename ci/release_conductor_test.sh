#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-conductor.XXXXXX")
temporary_root=$(CDPATH='' cd -- "${temporary_root}" && pwd)
cleanup() {
  if [[ -n ${race_records:-} && -d ${race_records}/.release-operation.lock ]]; then
    for owner in "${race_records}"/.release-operation.lock/owner-*.json; do
      [[ -f ${owner} ]] || continue
      group=$(jq -er '.group_pid // empty' "${owner}" 2>/dev/null || true)
      [[ -z ${group} ]] || node -e 'try { process.kill(-Number(process.argv[1]), "SIGKILL"); } catch {}' "${group}"
    done
  fi
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-conductor.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

bin=${temporary_root}/bin
records=${temporary_root}/records
workspace=${temporary_root}/workspace
mkdir "${bin}" "${records}" "${workspace}"

cat >"${bin}/fixture" <<'FIXTURE'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_CONDUCTOR_FIXTURE_STATE:?}
action=$1
name=$2
case "${action}" in
  produce)
    output=$3
    printf '%s\n' "${name}" >"${output}"
    ;;
  probe)
    [[ -f ${state}/${name}.mutation ]]
    ;;
  mutate)
    output=$3
    count=0
    [[ ! -f ${state}/${name}.count ]] || count=$(cat "${state}/${name}.count")
    printf '%s\n' "$((count + 1))" >"${state}/${name}.count"
    printf 'applied\n' >"${state}/${name}.mutation"
    printf '%s\n' "${name}" >"${output}"
    ;;
  dispatch)
    count=0
    [[ ! -f ${state}/${name}.dispatch-count ]] || count=$(cat "${state}/${name}.dispatch-count")
    printf '%s\n' "$((count + 1))" >"${state}/${name}.dispatch-count"
    printf 'dispatched\n' >"${state}/${name}.dispatched"
    if [[ -f ${state}/hold-dispatch ]]; then
      : >"${state}/dispatch-entered"
      while [[ ! -f ${state}/dispatch-release ]]; do sleep 0.02; done
    fi
    [[ ! -f ${state}/dispatch-ack-loss ]] || exit 3
    ;;
  bind)
    output=$3
    run_id=$4
    printf '%s:%s\n' "${name}" "${run_id}" >"${output}"
    ;;
  *) exit 64 ;;
esac
FIXTURE

cat >"${bin}/ait" <<'AIT'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_CONDUCTOR_FIXTURE_STATE:?}
identity='{"repository_index":1,"package_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","worker_job_index":0,"base_snapshot_id":"SNP-111111111111","revision_snapshot_id":"SNP-ABCDEF123456"}'
original_identity=${identity}
if [[ -f ${state}/reprepared ]]; then identity=$(jq -c '.worker_job_index = 1' <<<"${identity}"); fi
if [[ $1 == config && $2 == show ]]; then
  printf '{"repository_index":1}\n'
elif [[ $1 == task && $2 == show ]]; then
  if [[ -f ${state}/local-finished ]]; then
    printf '{"task_id":"LXT-1000","status":"completed","local_finish":{"task_id":"LXT-1000","status":"succeeded","target_line":"main","landed_snapshot_id":"SNP-ABCDEF123456"}}\n'
  else printf '{"task_id":"LXT-1000","status":"active","local_finish":null}\n'; fi
elif [[ $1 == task && $2 == finish ]]; then
  [[ ${AIT_FIXTURE_REVIEWER:-} == true ]] || exit 64
  count=0
  [[ ! -f ${state}/local-finish-count ]] || count=$(cat "${state}/local-finish-count")
  printf '%s\n' "$((count + 1))" >"${state}/local-finish-count"
  : >"${state}/local-finished"
  printf '{"task_id":"LXT-1000","landed_snapshot_id":"SNP-ABCDEF123456"}\n'
elif [[ $1 == workflow ]]; then
  phase=$2; source_task=$3
  is_apply=false; [[ $* != *--apply* ]] || is_apply=true
  [[ ${phase} != finish || ${is_apply} != true || ${AIT_FIXTURE_REVIEWER:-} == true ]] || exit 64
  [[ ${source_task} == LXT-1000 ]] || exit 64
  if [[ $* == *--expected-identity* ]]; then
    while [[ $1 != --expected-identity ]]; do shift; done
    if [[ -f ${state}/reprepared && $(jq -r '.worker_job_index' "$2") == 0 && ${phase} == ready && ${is_apply} == true ]]; then
      : # The existing pinned ready-apply path internally reuses retained bytes.
    elif [[ -f ${state}/reprepared && $(jq -r '.worker_job_index' "$2") == 0 ]]; then
      [[ ${phase} == finish && ${is_apply} == false ]] || exit 64
      [[ ! -f ${state}/old-readback-unknown ]] || exit 3
      jq -n --argjson identity "${original_identity}" --argjson accepted "$([[ -f ${state}/old-accepted ]] && echo true || echo false)" '
        {contract:"ait-agent-action/v1",command:"workflow.finish",next_action:{code:"completion_unknown"},expected_identity:$identity} +
        if $accepted then {completion:{outcome:"incomplete",expected_identity:$identity,patchset_id:"RXT-1000/P-01",submission_id:"RXT-1000/L-01",entries:[]}} else {} end'
      exit 0
    else
    jq -e --argjson expected "${identity}" ' . == $expected ' "$2" >/dev/null
    fi
  elif [[ ${phase} == finish ]]; then exit 64; fi
  counter=${phase}; [[ ${is_apply} == true ]] || counter=${phase}-readback
  count=0
  [[ ! -f ${state}/${counter}-count ]] || count=$(cat "${state}/${counter}-count")
  printf '%s\n' "$((count + 1))" >"${state}/${counter}-count"
  [[ ${phase} != finish || ${is_apply} == false ]] || : >"${state}/task-finished"
  if [[ ${phase} == ready && ${is_apply} == false && -f ${state}/new-readback-foreign ]]; then
    identity=$(jq -c '.package_hash = ("b" * 64)' <<<"${identity}")
  fi
  jq -n --arg phase "${phase}" --argjson apply "${is_apply}" --argjson identity "${identity}" --argjson queued "$([[ ${phase} == ready && ${is_apply} == false && -f ${state}/live-ci-pending ]] && echo true || echo false)" --argjson accepted "$([[ -f ${state}/task-finished ]] && echo true || echo false)" '
    {contract:"ait-agent-action/v1",command:("workflow."+$phase),task_id:"LXT-1000",remote_name:"origin",apply_status:"done",next_action:{code:"done"},expected_identity:$identity} +
    if $accepted then {closeout_status:"complete",remote_task_id:"RXT-1000",patchset_id:"RXT-1000/P-01",submission_id:"RXT-1000/L-01",
      completion:{outcome:"confirmed",expected_identity:$identity,patchset_id:"RXT-1000/P-01",submission_id:"RXT-1000/L-01",entries:[{local_task_id:"LXT-1000",remote_task_id:"RXT-1000",changes:[]}]}} |
      if $apply then . else .apply_status="partial" | .closeout_status="partial" | .next_action.code="restore_publication_mappings" | .completion.outcome="incomplete" | .completion.entries=[] end
    else {package_execution:{expected_identity:$identity,ci_state:"passed",availability:"available",recoverable:true}} end |
      if $queued then .package_execution.ci_state="queued" else . end'
else exit 64; fi
AIT

cat >"${bin}/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
state=${AIT_CONDUCTOR_FIXTURE_STATE:?}
if [[ $1 == run && $2 == list ]]; then
  workflow=
  while [[ $# -gt 0 ]]; do
    if [[ $1 == --workflow ]]; then workflow=$2; break; fi
    shift
  done
  if [[ -f ${state}/${workflow}.dispatched ]]; then
    ordinal=$(printf '%s' "${workflow}" | cksum | awk '{print ($1 % 9000) + 1000}')
    printf '[{"databaseId":%s,"headSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"completed","conclusion":"success","createdAt":"2026-09-06T00:00:00Z"}]\n' "${ordinal}"
  else
    printf '[]\n'
  fi
elif [[ $1 == run && $2 == rerun ]]; then
  [[ $* == *--failed* ]] || exit 64
  count=0; [[ ! -f ${state}/rerun-count ]] || count=$(cat "${state}/rerun-count")
  printf '%s\n' "$((count + 1))" >"${state}/rerun-count"
  printf "requested\n" >"${state}/retry-requested"
  [[ ! -f ${state}/rerun-ack-loss ]] || exit 3
  rm "${state}/fail-view"
elif [[ $1 == run && $2 == view ]]; then
  attempt=1; [[ ! -f ${state}/view-attempt ]] || attempt=$(cat "${state}/view-attempt")
  if [[ -f ${state}/fail-view ]]; then
    printf '{"status":"completed","conclusion":"failure","attempt":%s,"url":"https://example.invalid/run"}\n' "${attempt}"
  else
    printf '{"status":"completed","conclusion":"success","attempt":%s,"url":"https://example.invalid/run"}\n' "${attempt}"
  fi
elif [[ $1 == api ]]; then
  if [[ ${2:-} == --method ]]; then
    [[ $3 == POST && $4 == repos/owner/repository/actions/runs/*/pending_deployments &&
      $5 == --input && $6 == - ]] || exit 64
    [[ ! -f ${state}/deny-approval ]] || exit 3
    jq -e '.environment_ids == [77] and .state == "approved" and
      (.comment | type == "string" and length > 0)' >/dev/null
    count=0
    [[ ! -f ${state}/environment-approval-count ]] ||
      count=$(cat "${state}/environment-approval-count")
    printf '%s\n' "$((count + 1))" >"${state}/environment-approval-count"
    : >"${state}/environment-approved"
    printf '{}\n'
  else
    [[ ${2:-} == repos/owner/repository/actions/runs/*/pending_deployments ]] || exit 64
    if [[ -f ${state}/environment-approved ]]; then
      printf '[]\n'
    else
      printf '[{"environment":{"id":77,"name":"stable-promotion"}}]\n'
    fi
  fi
else
  exit 64
fi
GH
chmod +x "${bin}/fixture" "${bin}/ait" "${bin}/gh"

head_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
phases='[
  "source_preflight", "component_release",
  "candidate_freeze", "public_qualification", "candidate_admission",
  "component_receipts", "clean_host_qualification", "web_admission",
  "tag", "protected_promotion", "endpoint_publication",
  "winget_submission", "latest_alias", "closeout"
]'

jq -S -n \
  --arg records "${records}" --arg workspace "${workspace}" \
  --arg head_sha "${head_sha}" --argjson phases "${phases}" '
  def output($id): ($records + "/" + $id + ".receipt");
  def command($id): {
    id: $id, type: "command", cwd: $workspace, mutation: false,
    argv: ["fixture", "produce", $id, output($id), "{release_version}"],
    bindings: {
      release_version: {
        path: ($records + "/binding.json"), format: "json", selector: "release.version"
      }
    },
    outputs: [output($id)]
  };
  def workflow($id): ({
    id: $id, type: "github_workflow", cwd: $workspace,
    repository: "owner/repository", workflow: $id,
    head_sha_path: ($records + "/head-sha.txt"),
    dispatch_argv: ["fixture", "dispatch", $id, "{head_sha}"],
    bind: {
      cwd: $workspace,
      argv: ["fixture", "bind", $id, output($id), "{run_id}"],
      outputs: [output($id)]
    }
  } + if $id == "protected" then
    {approve_environments: ["stable-promotion"], auto_approve: true}
  else {} end);
  {
    contract: "ait.release.conductor-plan/v1",
    release: {version: "1.1.2", prior_version: "1.1.1", channel: "stable", tag: "v1.1.2"},
    records_root: $records,
    phases: [
      {id: $phases[0], actions: [command("source")]},
      {id: $phases[1], actions: [{
        id: "component", type: "ait_closeout", task: "LXT-1000",
        snapshot: "SNP-ABCDEF123456", repository_root: $workspace,
        edit_root: $workspace, patchset: null, remote: "origin",
        finish_local_before_ready: true,
        review_message: "Reviewed files: component source. Findings: none. Risks: bounded. Tests: pass. Recommendation: approve."
      }]},
      {id: $phases[2], actions: [command("candidate")]},
      {id: $phases[3], actions: [workflow("public-qualification")]},
      {id: $phases[4], actions: [command("admission")]},
      {id: $phases[5], actions: [workflow("receipts")]},
      {id: $phases[6], actions: [workflow("clean-host")]},
      {id: $phases[7], actions: [{
        id: "web", type: "evidence_gate", path: ($records + "/web-evidence.json"),
        message: "provide candidate-bound Web evidence"
      }]},
      {id: $phases[8], actions: [{
        id: "tag", type: "command", cwd: $workspace, mutation: true,
        argv: ["fixture", "mutate", "tag", output("tag")], outputs: [output("tag")],
        probe: {cwd: $workspace, argv: ["fixture", "probe", "tag"]}
      }]},
      {id: $phases[9], actions: [workflow("protected")]},
      {id: $phases[10], actions: [workflow("endpoints")]},
      {id: $phases[11], actions: [{
        id: "winget", type: "command", cwd: $workspace, mutation: true,
        argv: ["fixture", "mutate", "winget", output("winget")], outputs: [output("winget")],
        probe: {cwd: $workspace, argv: ["fixture", "probe", "winget"]}
      }]},
      {id: $phases[12], actions: [workflow("latest")]},
      {id: $phases[13], actions: [command("closeout")]}
    ]
  }
' >"${temporary_root}/plan.json"

jq -S --arg records "${records}" --arg workspace "${workspace}" --arg head_sha "${head_sha}" '
  def replace_values:
    walk(if type == "string" then
      gsub($records; "{{records_root}}") |
      gsub($workspace; "{{values.workspace}}") |
      gsub($head_sha; "{{values.head_sha}}")
    else . end);
  {
    contract: "ait.release.conductor-recipe/v1",
    phases: [.phases[] |
      if .id == "component_release" then
        {id, actions: [{"$for_each": "collections.component_closeouts", template: "{{item}}"}]}
      else
        {id, actions: (.actions | replace_values)}
      end
    ]
  }
' "${temporary_root}/plan.json" >"${temporary_root}/recipe.json"
jq -S -n --arg records "${records}" --arg workspace "${workspace}" --arg head_sha "${head_sha}" \
  --slurpfile plan "${temporary_root}/plan.json" '
  {
    contract: "ait.release.conductor-spec/v1",
    release: $plan[0].release,
    records_root: $records,
    values: {workspace: $workspace, head_sha: $head_sha},
    collections: {component_closeouts: $plan[0].phases[1].actions}
  }
' >"${temporary_root}/spec.json"
node "${repo_root}/ci/release_conductor_init.mjs" \
  --spec "${temporary_root}/spec.json" \
  --recipe "${temporary_root}/recipe.json" \
  --plan "${records}/conductor-plan.json" >/dev/null
jq -S . "${temporary_root}/plan.json" >"${temporary_root}/expected-plan.json"
jq -S 'del(.source)' "${records}/conductor-plan.json" >"${temporary_root}/actual-plan.json"
diff -u "${temporary_root}/expected-plan.json" "${temporary_root}/actual-plan.json"
if node "${repo_root}/ci/release_conductor_init.mjs" \
  --spec "${temporary_root}/spec.json" \
  --recipe "${temporary_root}/recipe.json" \
  --plan "${records}/conductor-plan.json" >/dev/null 2>"${temporary_root}/init-repeat.stderr"; then
  printf 'conductor init overwrote an immutable plan\n' >&2
  exit 65
fi
grep -F 'release conductor plan output already exists' "${temporary_root}/init-repeat.stderr" >/dev/null
printf '%s\n' "${head_sha}" >"${records}/head-sha.txt"
printf '{"release":{"version":"1.1.2"}}\n' >"${records}/binding.json"
"${repo_root}/ci/release_conductor.sh" status \
  --spec "${temporary_root}/spec.json" --recipe "${temporary_root}/recipe.json" \
  >"${temporary_root}/compiled-status.stdout"
grep -F 'release: v1.1.2 running' "${temporary_root}/compiled-status.stdout" >/dev/null

export AIT_CONDUCTOR_FIXTURE_STATE=${temporary_root}/fixture-state
mkdir "${AIT_CONDUCTOR_FIXTURE_STATE}"
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/dispatch-ack-loss"
capture_fixture_reviewer_local_finish() {
  AIT_FIXTURE_REVIEWER=true "${bin}/ait" task finish LXT-1000 --local --json >/dev/null
}
capture_fixture_reviewer_finish() {
  local root=$1
  AIT_FIXTURE_REVIEWER=true "${bin}/ait" workflow finish LXT-1000 --apply --remote origin \
    --expected-identity "${root}/conductor-receipts/component.package-identity.json" \
    --review-message 'Reviewed files: fixture. Findings: none. Risks: isolated test. Tests: protocol. Recommendation: pass.' --json \
    >"${root}/conductor-logs/component.ait.finish.log"
}
# Source closeout belongs to its Reviewer too. Repeated controller attempts
# must retain the same Task without Local finish or package preparation.
for attempt in 1 2; do
  if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
    --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
    >"${temporary_root}/local-review-wait.stdout" 2>"${temporary_root}/local-review-wait.stderr"; then exit 65; else test "$?" = 75; fi
  grep -F 'independent Reviewer must finish the original Local source Task' "${temporary_root}/local-review-wait.stderr" >/dev/null
  test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/local-finish-count"
  test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count"
done
capture_fixture_reviewer_local_finish
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/review-wait.stdout" 2>"${temporary_root}/review-wait.stderr"; then exit 65; else test "$?" = 75; fi
grep -F 'independent Reviewer must finish' "${temporary_root}/review-wait.stderr" >/dev/null
test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count"
capture_fixture_reviewer_finish "${records}"
# A full JSON value does not prove that the Reviewer's stdout capture ended.
# Keep an actual writable descriptor open in this independent test process.
exec 9>>"${records}/conductor-logs/component.ait.finish.log"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/capture-open.stdout" 2>"${temporary_root}/capture-open.stderr"; then exit 65; else test "$?" = 75; fi
exec 9>&-
grep -F 'original finish capture still has a writer' "${temporary_root}/capture-open.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
# A partial capture cannot stand in for the Reviewer's complete native output.
cp "${records}/conductor-logs/component.ait.finish.log" "${temporary_root}/original-finish.json"
jq '.apply_status="partial" | .closeout_status="partial" | .completion.outcome="incomplete" | .completion.entries=[]' \
  "${temporary_root}/original-finish.json" >"${records}/conductor-logs/component.ait.finish.log"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/partial.stdout" 2>"${temporary_root}/partial.stderr"; then exit 65; else test "$?" = 75; fi
grep -F 'original finish capture is partial' "${temporary_root}/partial.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
for capture_fault in truncated wrong-pin; do
  if [[ ${capture_fault} == truncated ]]; then
    printf '{' >"${records}/conductor-logs/component.ait.finish.log"
  else
    jq '.expected_identity.worker_job_index = 1' "${temporary_root}/original-finish.json" \
      >"${records}/conductor-logs/component.ait.finish.log"
  fi
  if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
    --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
    >"${temporary_root}/${capture_fault}.stdout" 2>"${temporary_root}/${capture_fault}.stderr"; then exit 65; else test "$?" = 75; fi
  grep -F 'original finish capture is partial, truncated or mismatched' "${temporary_root}/${capture_fault}.stderr" >/dev/null
  test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
done
cp "${temporary_root}/original-finish.json" "${records}/conductor-logs/component.ait.finish.log"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/first.stdout" 2>"${temporary_root}/first.stderr"; then
  printf 'conductor did not stop for missing Web evidence\n' >&2
  exit 65
fi
grep -F 'web: provide candidate-bound Web evidence' "${temporary_root}/first.stderr" >/dev/null
jq -e '.phases[] | select(.id == "web_admission") | .actions[0].status == "action_required"' \
  "${records}/state.json" >/dev/null
test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/tag.count"
test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/winget.count"

grep -F 'RXT-1000/P-01: exact package closeout complete' "${temporary_root}/first.stdout" >/dev/null
if grep -F '/C-' "${temporary_root}/first.stdout" >/dev/null; then
  printf 'conductor exposed an internal Change reference\n' >&2
  exit 65
fi
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/local-finish-count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1

printf 'browser-pass\n' >"${records}/web-evidence.json"
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/deny-approval"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${temporary_root}/plan.json" --state "${records}/state.json" >"${temporary_root}/approval.stdout" 2>"${temporary_root}/approval.stderr"; then exit 65; else test "$?" = 75; fi
jq -e '.status == "waiting_approval"' "${records}/state.json" >/dev/null
rm "${AIT_CONDUCTOR_FIXTURE_STATE}/deny-approval"
PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/resume.stdout"
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/tag.count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/winget.count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/environment-approval-count")" = 1

PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/second-resume.stdout"
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/local-finish-count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/tag.count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/winget.count")" = 1
jq -e '.status == "complete" and ([.phases[].status] | all(. == "complete"))' \
  "${records}/state.json" >/dev/null
cp "${records}/state.json" "${temporary_root}/completed-state.json"

# A cached completed phase must also hold while its original capture is open.
exec 9>>"${records}/conductor-logs/component.ait.finish.log"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/cached-capture-open.stdout" 2>"${temporary_root}/cached-capture-open.stderr"; then exit 65; else test "$?" = 75; fi
exec 9>&-
grep -F 'original finish capture still has a writer' "${temporary_root}/cached-capture-open.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
cp "${temporary_root}/completed-state.json" "${records}/state.json"
rm "${records}/conductor-logs/component.ait.finish.log"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/plan.json" --state "${records}/state.json" \
  >"${temporary_root}/cached-capture-missing.stdout" 2>"${temporary_root}/cached-capture-missing.stderr"; then exit 65; else test "$?" = 75; fi
grep -F 'original finish capture is unavailable' "${temporary_root}/cached-capture-missing.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
cp "${temporary_root}/original-finish.json" "${records}/conductor-logs/component.ait.finish.log"

# Lose all central state: recover each completed checkpoint without any dispatch.
rm "${records}/state.json"
PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${temporary_root}/plan.json" --state "${records}/state.json" >"${temporary_root}/recovered.stdout"
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/public-qualification.dispatch-count")" = 1

# Explicit Server-reset reprepare is an outside native invocation. The
# controller only reconciles its capture; it never creates the replacement.
reprepare_records=${temporary_root}/reprepare-records
mkdir "${reprepare_records}"
jq -S --arg records "${reprepare_records}" '.records_root = $records' "${temporary_root}/spec.json" >"${temporary_root}/reprepare-spec.json"
node "${repo_root}/ci/release_conductor_init.mjs" --spec "${temporary_root}/reprepare-spec.json" --recipe "${temporary_root}/recipe.json" --plan "${reprepare_records}/conductor-plan.json" >/dev/null
printf '%s\n' "${head_sha}" >"${reprepare_records}/head-sha.txt"
printf '{"release":{"version":"1.1.2"}}\n' >"${reprepare_records}/binding.json"
export AIT_CONDUCTOR_FIXTURE_STATE=${temporary_root}/reprepare-fixture-state
mkdir "${AIT_CONDUCTOR_FIXTURE_STATE}"
capture_fixture_reviewer_local_finish
reprepare_run() {
  local name=$1
  if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${reprepare_records}/conductor-plan.json" --state "${reprepare_records}/state.json" >"${temporary_root}/${name}.stdout" 2>"${temporary_root}/${name}.stderr"; then exit 65; else test "$?" = 75; fi
}
reprepare_run reprepare-initial
grep -F 'independent Reviewer must finish' "${temporary_root}/reprepare-initial.stderr" >/dev/null
cp "${reprepare_records}/state.json" "${temporary_root}/prior-reprepare-state.json"
cp "${reprepare_records}/conductor-receipts/component.package-identity.json" "${temporary_root}/prior-reprepare-pin.json"
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/reprepared"
"${bin}/ait" workflow ready LXT-1000 --apply --remote origin --expected-identity "${reprepare_records}/conductor-receipts/component.package-identity.json" --json >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
cp "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log" "${temporary_root}/explicit-ready.json"
# Wrong full invocation/context and a native409 cannot rebind the old pin.
for capture_fault in malformed wrong-task wrong-remote wrong-selector rejected409; do
  if [[ ${capture_fault} == malformed ]]; then
    printf '{"contract":' >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
  else
    jq --arg fault "${capture_fault}" --arg pin "${reprepare_records}/conductor-receipts/component.package-identity.json" --slurpfile original "${temporary_root}/prior-reprepare-pin.json" '
      del(.command) | .mode="workflow_package" | .expected_identity_path=$pin |
      if $fault=="wrong-task" then .task_id="LXT-9999"
      elif $fault=="wrong-remote" then .remote_name="other"
      elif $fault=="wrong-selector" then .expected_identity_path="/another/invocation.json"
      else .expected_identity=$original[0] | .apply_status="stopped" | .next_action.code="execution_unknown" | .package_execution=null end
    ' "${temporary_root}/explicit-ready.json" >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
  fi
  reprepare_run "reprepare-${capture_fault}"
  jq -e '.worker_job_index==0' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
  test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 2
  test ! -f "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count"
done
cp "${temporary_root}/explicit-ready.json" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
exec 9>>"${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
reprepare_run reprepare-open
exec 9>&-
grep -F 'explicit ready capture still has a writer' "${temporary_root}/reprepare-open.stderr" >/dev/null
jq '.expected_identity.package_hash=("b" * 64) | .package_execution.expected_identity=.expected_identity' "${temporary_root}/explicit-ready.json" >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
reprepare_run reprepare-foreign
grep -F 'explicit ready capture is incomplete or incompatible' "${temporary_root}/reprepare-foreign.stderr" >/dev/null
cp "${temporary_root}/explicit-ready.json" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
for fault in old-readback-unknown old-accepted new-readback-foreign; do
  : >"${AIT_CONDUCTOR_FIXTURE_STATE}/${fault}"
  reprepare_run "reprepare-${fault}"
  rm "${AIT_CONDUCTOR_FIXTURE_STATE}/${fault}"
  jq -e '.worker_job_index == 0' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
  test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 2
  test ! -f "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count"
done
grep -F 'original formal acceptance is retained' "${temporary_root}/reprepare-old-accepted.stderr" >/dev/null
grep -F 'native selected execution disagree' "${temporary_root}/reprepare-new-readback-foreign.stderr" >/dev/null
# The closed apply capture cannot turn a currently pending readback into CI success.
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/live-ci-pending"
reprepare_run reprepare-current-pending
jq -e '.phases[] | select(.id=="component_release") | .actions[0] | .expected_identity.worker_job_index==1 and .review_pending==false and .status=="waiting_external"' "${reprepare_records}/state.json" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 2
# Preserve a genuine full pending response while read-only CI progresses.
jq --arg pin "${reprepare_records}/conductor-receipts/component.package-identity.json" '
  del(.command) | .mode="workflow_package" | .expected_identity_path=$pin |
  .apply_status="stopped" | .next_action.code="ci_pending" | .package_execution.ci_state="queued"
' "${temporary_root}/explicit-ready.json" >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
cp "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log" "${temporary_root}/full-pending-ready.json"
reprepare_run reprepare-full-pending
jq -e '.worker_job_index == 1' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
rm "${AIT_CONDUCTOR_FIXTURE_STATE}/live-ci-pending"
reprepare_run reprepare-full-passed
cmp "${temporary_root}/full-pending-ready.json" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 2
grep -F 'independent Reviewer must finish' "${temporary_root}/reprepare-full-passed.stderr" >/dev/null
cp "${temporary_root}/explicit-ready.json" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
cp "${temporary_root}/prior-reprepare-state.json" "${reprepare_records}/state.json"
cp "${temporary_root}/prior-reprepare-pin.json" "${reprepare_records}/conductor-receipts/component.package-identity.json"
reprepare_run reprepare-adopted
grep -F 'independent Reviewer must finish' "${temporary_root}/reprepare-adopted.stderr" >/dev/null
jq -e '.worker_job_index == 1' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
# Recover either interruption order of the existing pin/state writes using
# the same closed capture and exact old/new native readback.
cp "${reprepare_records}/state.json" "${temporary_root}/next-reprepare-state.json"
cp "${temporary_root}/prior-reprepare-state.json" "${reprepare_records}/state.json"
reprepare_run reprepare-state-interrupted
cp "${temporary_root}/next-reprepare-state.json" "${reprepare_records}/state.json"
cp "${temporary_root}/prior-reprepare-pin.json" "${reprepare_records}/conductor-receipts/component.package-identity.json"
reprepare_run reprepare-pin-interrupted
jq -e '.worker_job_index == 1' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 2
test ! -f "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count"
# The browser uses its original pinned request and retains the actual v2
# response/HTTP header/confirmation in the existing WT output paths. It must
# never be relabeled as a native CLI envelope.
cp "${temporary_root}/prior-reprepare-state.json" "${reprepare_records}/state.json"
cp "${temporary_root}/prior-reprepare-pin.json" "${reprepare_records}/conductor-receipts/component.package-identity.json"
cp "${reprepare_records}/conductor-logs/component.ait.ready.log" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.log"
jq '{contract:"ait.web.personal.workflow-mutation-receipt.v2",ok:true,operation:"workflow_ready",target:"LXT-1000",scope:"remote",remote:"origin",summary:"Native ready",inspect_path:"/tasks/LXT-1000?scope=local",authority:{kind:"workflow",candidate_kind:"package",task_id:"LXT-1000",expected_identity:.expected_identity,apply_status:.apply_status,closeout_status:null,next_action:.next_action,package_execution:.package_execution,completion:null,formal_finish:null}}' "${temporary_root}/explicit-ready.json" >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.stdout"
printf 'HTTP 200\n' >"${reprepare_records}/conductor-logs/component.ait.ready-pinned.headers"
node --input-type=module -e 'import fs from "node:fs"; const pin=JSON.parse(fs.readFileSync(process.argv[1])); const request={contract:"ait.web.personal.workflow-mutation-request.v2",operation:"workflow_ready",task_id:"LXT-1000",scope:"remote",remote:"origin",candidate_kind:"package",apply:true,expected_identity:pin}; fs.writeFileSync(process.argv[2],"Run this exact operation once?\n\n"+JSON.stringify(request,null,2)+"\n\nKeep the original pin for observation and recovery.");' "${temporary_root}/prior-reprepare-pin.json" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout"
cp "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout" "${temporary_root}/browser-confirmation.stdout"
node --input-type=module -e 'import fs from "node:fs"; const file=process.argv[1], text=fs.readFileSync(file,"utf8"), start=text.indexOf("{"), end=text.lastIndexOf("}")+1, request=JSON.parse(text.slice(start,end)); request.expected_identity=null; fs.writeFileSync(file,text.slice(0,start)+JSON.stringify(request,null,2)+text.slice(end));' "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout"
reprepare_run reprepare-browser-unpinned
jq -e '.worker_job_index == 0' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
cp "${temporary_root}/browser-confirmation.stdout" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout"
node --input-type=module -e 'import fs from "node:fs"; const file=process.argv[1], text=fs.readFileSync(file,"utf8"), end="\n\nKeep the original pin for observation and recovery."; fs.writeFileSync(file,text.slice(0,-end.length)+"\n\nUnexpected replacement instruction"+end);' "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout"
reprepare_run reprepare-browser-unknown-notice
jq -e '.worker_job_index == 0' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
cp "${temporary_root}/browser-confirmation.stdout" "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout"
# Personal R06 adds its explicit same-seal/new-CI notice after the request JSON.
node --input-type=module -e 'import fs from "node:fs"; const file=process.argv[1], text=fs.readFileSync(file,"utf8"), end="\n\nKeep the original pin for observation and recovery."; fs.writeFileSync(file,text.slice(0,-end.length)+"\n\nIf Native verifies a reset, Ready may return a new Job for this same sealed package. New CI and required review apply."+end);' "${reprepare_records}/conductor-logs/component.ait.ready-pinned.confirmation.stdout"
reprepare_run reprepare-browser
jq -e '.worker_job_index == 1' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 2
# A current explicit pinned-ready call may itself return the released new-Job
# result. Reconcile that same original stdout in place, without a second apply.
cp "${temporary_root}/prior-reprepare-state.json" "${reprepare_records}/state.json"
cp "${temporary_root}/prior-reprepare-pin.json" "${reprepare_records}/conductor-receipts/component.package-identity.json"
for suffix in stdout headers confirmation.stdout log; do rm "${reprepare_records}/conductor-logs/component.ait.ready-pinned.${suffix}"; done
jq '.phases |= map(if .id=="component_release" then .actions[0].review_pending=false else . end)' "${reprepare_records}/state.json" >"${temporary_root}/pending-state.json"
mv "${temporary_root}/pending-state.json" "${reprepare_records}/state.json"
reprepare_run reprepare-inline
jq -e '.worker_job_index == 1' "${reprepare_records}/conductor-receipts/component.package-identity.json" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 3
capture_fixture_reviewer_finish "${reprepare_records}"
reprepare_run reprepare-finished
grep -F 'exact package closeout complete' "${temporary_root}/reprepare-finished.stdout" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 3
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1
test ! -f "${AIT_CONDUCTOR_FIXTURE_STATE}/tag.count"

failure_records=${temporary_root}/failure-records
mkdir "${failure_records}"
jq -S --arg records "${failure_records}" '.records_root = $records' \
  "${temporary_root}/spec.json" >"${temporary_root}/failure-spec.json"
node "${repo_root}/ci/release_conductor_init.mjs" \
  --spec "${temporary_root}/failure-spec.json" \
  --recipe "${temporary_root}/recipe.json" \
  --plan "${failure_records}/conductor-plan.json" >/dev/null
printf '%s\n' "${head_sha}" >"${failure_records}/head-sha.txt"
printf '{"release":{"version":"1.1.2"}}\n' >"${failure_records}/binding.json"
export AIT_CONDUCTOR_FIXTURE_STATE=${temporary_root}/failure-fixture-state
mkdir "${AIT_CONDUCTOR_FIXTURE_STATE}"
capture_fixture_reviewer_local_finish
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${failure_records}/conductor-plan.json" --state "${failure_records}/state.json" \
  >"${temporary_root}/failure-review.stdout" 2>"${temporary_root}/failure-review.stderr"; then exit 65; else test "$?" = 75; fi
capture_fixture_reviewer_finish "${failure_records}"
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/fail-view"
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/rerun-ack-loss"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${failure_records}/conductor-plan.json" \
  --state "${failure_records}/state.json" \
  >"${temporary_root}/failure.stdout" 2>"${temporary_root}/failure.stderr"; then
  printf 'conductor admitted a failed workflow\n' >&2
  exit 65
fi
grep -F 'public-qualification workflow run' "${temporary_root}/failure.stderr" >/dev/null
grep -F 'concluded failure' "${temporary_root}/failure.stderr" >/dev/null
test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/tag.count"
test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/winget.count"

test -f "${AIT_CONDUCTOR_FIXTURE_STATE}/retry-requested"
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/rerun-count")" = 1
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${failure_records}/conductor-plan.json" --state "${failure_records}/state.json" >"${temporary_root}/ack-wait.stdout" 2>"${temporary_root}/ack-wait.stderr"; then exit 65; else test "$?" = 75; fi
grep -F 'rerun acknowledgement pending' "${temporary_root}/ack-wait.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/rerun-count")" = 1
printf '2\n' >"${AIT_CONDUCTOR_FIXTURE_STATE}/view-attempt"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${failure_records}/conductor-plan.json" --state "${failure_records}/state.json" >"${temporary_root}/retry-two.stdout" 2>"${temporary_root}/retry-two.stderr"; then exit 65; else test "$?" = 75; fi
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/rerun-count")" = 2
printf '3\n' >"${AIT_CONDUCTOR_FIXTURE_STATE}/view-attempt"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${failure_records}/conductor-plan.json" --state "${failure_records}/state.json" >"${temporary_root}/exhaust.stdout" 2>"${temporary_root}/exhaust.stderr"; then exit 65; else test "$?" = 65; fi
grep -F 'workflow retry budget exhausted' "${temporary_root}/exhaust.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/rerun-count")" = 2
jq -e '.phases[] | select(.id == "public_qualification") | .actions[0].retry_count == 2' "${failure_records}/state.json" >/dev/null
# An external operator may later complete this same bound run; this controller
# still sends no third retry and never resets its consumed budget.
printf '4\n' >"${AIT_CONDUCTOR_FIXTURE_STATE}/view-attempt"
rm "${AIT_CONDUCTOR_FIXTURE_STATE}/fail-view"
printf 'browser-pass\n' >"${failure_records}/web-evidence.json"
PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${failure_records}/conductor-plan.json" --state "${failure_records}/state.json" >"${temporary_root}/retry.stdout"
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/public-qualification.dispatch-count")" = 1
jq -e '.status == "complete"' "${failure_records}/state.json" >/dev/null

# Race two real controller processes at a controlled dispatch boundary. Kill
# the owned command group after its side effect but before acknowledgement.
race_records=${temporary_root}/race-records
mkdir "${race_records}"
jq -S --arg records "${race_records}" '.records_root = $records' "${temporary_root}/spec.json" >"${temporary_root}/race-spec.json"
node "${repo_root}/ci/release_conductor_init.mjs" --spec "${temporary_root}/race-spec.json" --recipe "${temporary_root}/recipe.json" --plan "${race_records}/conductor-plan.json" >/dev/null
printf '%s\n' "${head_sha}" >"${race_records}/head-sha.txt"
printf '{"release":{"version":"1.1.2"}}\n' >"${race_records}/binding.json"
export AIT_CONDUCTOR_FIXTURE_STATE=${temporary_root}/race-fixture-state
mkdir "${AIT_CONDUCTOR_FIXTURE_STATE}"
capture_fixture_reviewer_local_finish
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${race_records}/conductor-plan.json" --state "${race_records}/state.json" >"${temporary_root}/race-review.stdout" 2>"${temporary_root}/race-review.stderr"; then exit 65; else test "$?" = 75; fi
capture_fixture_reviewer_finish "${race_records}"
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/hold-dispatch"
PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${race_records}/conductor-plan.json" --state "${race_records}/state.json" >"${temporary_root}/race-first.stdout" 2>"${temporary_root}/race-first.stderr" &
race_pid=$!
for ((attempt = 0; attempt < 500; attempt++)); do
  [[ ! -f ${AIT_CONDUCTOR_FIXTURE_STATE}/dispatch-entered ]] || break
  sleep 0.02
done
test -f "${AIT_CONDUCTOR_FIXTURE_STATE}/dispatch-entered"
jq -e --arg head "${head_sha}" '.phases[] | select(.id == "public_qualification") | .actions[0] | .status == "prepared" and .head_sha == $head' "${race_records}/state.json" >/dev/null
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${race_records}/conductor-plan.json" --state "${race_records}/state.json" >"${temporary_root}/race-second.stdout" 2>"${temporary_root}/race-second.stderr"; then exit 65; else test "$?" = 75; fi
grep -F 'already has a writer' "${temporary_root}/race-second.stderr" >/dev/null
race_group=$(jq -er '.group_pid' "${race_records}"/.release-operation.lock/owner-*.json)
node -e 'process.kill(-Number(process.argv[1]), "SIGKILL")' "${race_group}"
if wait "${race_pid}"; then exit 65; fi
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/public-qualification.dispatch-count")" = 1
printf '%040d\n' 1 >"${race_records}/head-sha.txt"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${race_records}/conductor-plan.json" --state "${race_records}/state.json" >"${temporary_root}/race-drift.stdout" 2>"${temporary_root}/race-drift.stderr"; then exit 65; else test "$?" = 65; fi
grep -F 'recorded workflow head SHA drifted' "${temporary_root}/race-drift.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/public-qualification.dispatch-count")" = 1
printf '%s\n' "${head_sha}" >"${race_records}/head-sha.txt"
rm "${AIT_CONDUCTOR_FIXTURE_STATE}/hold-dispatch"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run --plan "${race_records}/conductor-plan.json" --state "${race_records}/state.json" >"${temporary_root}/race-recover.stdout" 2>"${temporary_root}/race-recover.stderr"; then exit 65; else test "$?" = 75; fi
grep -F 'web: provide candidate-bound Web evidence' "${temporary_root}/race-recover.stderr" >/dev/null
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/public-qualification.dispatch-count")" = 1
test ! -e "${AIT_CONDUCTOR_FIXTURE_STATE}/tag.count"

cp "${temporary_root}/plan.json" "${temporary_root}/drifted-plan.json"
jq '.release.tag = "v9.9.9"' "${temporary_root}/drifted-plan.json" \
  >"${temporary_root}/drifted-plan.new"
mv "${temporary_root}/drifted-plan.new" "${temporary_root}/drifted-plan.json"
if PATH="${bin}:${PATH}" node "${repo_root}/ci/release_conductor.mjs" run \
  --plan "${temporary_root}/drifted-plan.json" --state "${records}/state.json" \
  >"${temporary_root}/drift.stdout" 2>"${temporary_root}/drift.stderr"; then
  printf 'conductor admitted a drifted release plan\n' >&2
  exit 65
fi
grep -F 'release conductor currently requires one exact stable patch or minor advance' \
  "${temporary_root}/drift.stderr" >/dev/null

printf 'release conductor tests passed\n'
