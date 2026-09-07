#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-conductor.XXXXXX")
temporary_root=$(CDPATH='' cd -- "${temporary_root}" && pwd)
cleanup() {
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
if [[ $1 == task && $2 == show ]]; then
  if [[ -f ${state}/task-finished ]]; then printf '{"status":"completed"}\n';
  else printf '{"status":"active"}\n'; fi
elif [[ $1 == task && $2 == finish ]]; then
  count=0
  [[ ! -f ${state}/local-finish-count ]] || count=$(cat "${state}/local-finish-count")
  printf '%s\n' "$((count + 1))" >"${state}/local-finish-count"
  printf '{"status":"completed"}\n'
elif [[ $1 == workflow && $2 == ready ]]; then
  count=0
  [[ ! -f ${state}/ready-count ]] || count=$(cat "${state}/ready-count")
  count=$((count + 1))
  printf '%s\n' "${count}" >"${state}/ready-count"
  if [[ ${count} == 1 ]]; then printf '503 capacity is exhausted\n' >&2; exit 1; fi
  printf 'internal RXT-1000/C-01/P-01\nRXT-1000/P-01\n'
elif [[ $1 == patchset && $2 == ci-status ]]; then
  printf '{"overall_status":"pass","blocking_failure_count":0}\n'
elif [[ $1 == workflow && $2 == finish ]]; then
  count=0
  [[ ! -f ${state}/finish-count ]] || count=$(cat "${state}/finish-count")
  printf '%s\n' "$((count + 1))" >"${state}/finish-count"
  : >"${state}/task-finished"
else
  exit 64
fi
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
elif [[ $1 == run && $2 == view ]]; then
  if [[ -f ${state}/fail-view ]]; then
    printf '{"status":"completed","conclusion":"failure","url":"https://example.invalid/run"}\n'
  else
    printf '{"status":"completed","conclusion":"success","url":"https://example.invalid/run"}\n'
  fi
elif [[ $1 == api ]]; then
  if [[ ${2:-} == --method ]]; then
    [[ $3 == POST && $4 == repos/owner/repository/actions/runs/*/pending_deployments &&
      $5 == --input && $6 == - ]] || exit 64
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
  "source_preflight", "prior_qualification", "component_release",
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
    {approve_environments: ["stable-promotion"]}
  else {} end);
  {
    contract: "ait.release.conductor-plan/v1",
    release: {version: "1.1.2", prior_version: "1.1.1", channel: "stable", tag: "v1.1.2"},
    records_root: $records,
    phases: [
      {id: $phases[0], actions: [command("source")]},
      {id: $phases[1], actions: [command("prior")]},
      {id: $phases[2], actions: [{
        id: "component", type: "ait_closeout", task: "LXT-1000",
        snapshot: "SNP-ABCDEF123456", repository_root: $workspace,
        edit_root: $workspace, patchset: null, remote: "origin",
        finish_local_before_ready: true,
        review_message: "Reviewed files: component source. Findings: none. Risks: bounded. Tests: pass. Recommendation: approve."
      }]},
      {id: $phases[3], actions: [command("candidate")]},
      {id: $phases[4], actions: [workflow("public-qualification")]},
      {id: $phases[5], actions: [command("admission")]},
      {id: $phases[6], actions: [workflow("receipts")]},
      {id: $phases[7], actions: [workflow("clean-host")]},
      {id: $phases[8], actions: [{
        id: "web", type: "evidence_gate", path: ($records + "/web-evidence.json"),
        message: "provide candidate-bound Web evidence"
      }]},
      {id: $phases[9], actions: [{
        id: "tag", type: "command", cwd: $workspace, mutation: true,
        argv: ["fixture", "mutate", "tag", output("tag")], outputs: [output("tag")],
        probe: {cwd: $workspace, argv: ["fixture", "probe", "tag"]}
      }]},
      {id: $phases[10], actions: [workflow("protected")]},
      {id: $phases[11], actions: [workflow("endpoints")]},
      {id: $phases[12], actions: [{
        id: "winget", type: "command", cwd: $workspace, mutation: true,
        argv: ["fixture", "mutate", "winget", output("winget")], outputs: [output("winget")],
        probe: {cwd: $workspace, argv: ["fixture", "probe", "winget"]}
      }]},
      {id: $phases[13], actions: [workflow("latest")]},
      {id: $phases[14], actions: [command("closeout")]}
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
    collections: {component_closeouts: $plan[0].phases[2].actions}
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

grep -F 'RXT-1000/P-01: completed' "${temporary_root}/first.stdout" >/dev/null
if grep -F '/C-' "${temporary_root}/first.stdout" >/dev/null; then
  printf 'conductor exposed an internal Change reference\n' >&2
  exit 65
fi
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/ready-count")" = 3
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/local-finish-count")" = 1
test "$(cat "${AIT_CONDUCTOR_FIXTURE_STATE}/finish-count")" = 1

printf 'browser-pass\n' >"${records}/web-evidence.json"
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
: >"${AIT_CONDUCTOR_FIXTURE_STATE}/fail-view"
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
