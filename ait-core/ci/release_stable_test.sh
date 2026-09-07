#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-stable.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-stable.*) rm -rf -- "${temporary_root}" ;;
    *) return 1 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

records=${temporary_root}/records
workspace=${temporary_root}/workspace
mkdir -p "${records}" "${workspace}"/{ait-core,ait-server,ait-runner,ait-python,ait-node,public,web-test,personal}
for fixture in prior-family.json prior-coordinator.json final-family.json final-coordinator.json web-components.json candidate-ait personal-bin community-bin community-cli-bin chrome; do
  printf '{}\n' >"${workspace}/${fixture}"
done
chmod +x "${workspace}/candidate-ait" "${workspace}/personal-bin" "${workspace}/community-bin" \
  "${workspace}/community-cli-bin" "${workspace}/chrome"

jq -n --arg records "${records}" --arg workspace "${workspace}" '
  {
    contract:"ait.release.accepted-input/v1",
    release:{channel:"stable",prior_version:"1.1.2",version:"1.1.3"},
    records_root:$records,
    github_repository:"owner/ait-native",
    winget_fork:"owner/winget-pkgs",
    paths:{
      core_root:($workspace+"/ait-core"),public_source:($workspace+"/public"),
      prior_family:($workspace+"/prior-family.json"),prior_coordinator:($workspace+"/prior-coordinator.json"),
      final_family:($workspace+"/final-family.json"),final_coordinator:($workspace+"/final-coordinator.json"),
      web_components:($workspace+"/web-components.json"),web_test_root:($workspace+"/web-test"),
      personal_root:($workspace+"/personal"),
      personal_bin:($workspace+"/personal-bin"),community_bin:($workspace+"/community-bin"),
      community_cli_bin:($workspace+"/community-cli-bin"),chrome:($workspace+"/chrome")
    },
    web:{server_url:"http://127.0.0.1:8088/",server_data:$workspace,seed:20260822},
    component_closeouts:[
      ["core","RCT-1001","SNP-111111111111"],
      ["server","RST-1002","SNP-222222222222"],
      ["runner","RRT-1003","SNP-333333333333"],
      ["python","RPT-1004","SNP-444444444444"],
      ["node","RNT-1005","SNP-555555555555"]
    ] | map({id:.[0],task:.[1],snapshot:.[2],patchset:(.[1]+"/P-01"),
      repository_root:($workspace+"/ait-"+.[0]),edit_root:($workspace+"/ait-"+.[0]),
      remote:"origin",finish_local_before_ready:false,
      review_message:"Reviewed files: release component. Findings: accepted. Risks: bounded. Tests: pass. Recommendation: approve."})
  }
' >"${temporary_root}/accepted-input.json"
# Core is named ait-core; every generated test cwd must exist.
mkdir -p "${workspace}/ait-core"

node "${repo_root}/ci/release_stable_spec.mjs" \
  --input "${temporary_root}/accepted-input.json" --output "${records}/release-spec.json" >/dev/null
node "${repo_root}/ci/release_conductor_init.mjs" \
  --spec "${records}/release-spec.json" --recipe "${repo_root}/release/stable-conductor.recipe.json" \
  --plan "${records}/conductor-plan.json" >/dev/null
node "${repo_root}/ci/release_conductor.mjs" status \
  --plan "${records}/conductor-plan.json" --state "${records}/conductor-state.json" \
  >"${temporary_root}/status.txt"

jq -e '
  .release.tag == "v1.1.3" and (.phases | length) == 15 and
  [.phases[].id] == [
    "source_preflight","prior_qualification","component_release","candidate_freeze",
    "public_qualification","candidate_admission","component_receipts","clean_host_qualification",
    "web_admission","tag","protected_promotion","endpoint_publication","winget_submission",
    "latest_alias","closeout"
  ] and
  ([.phases[] | select(.id == "component_release") | .actions[] | .patchset] | sort) ==
    ["RCT-1001/P-01","RNT-1005/P-01","RPT-1004/P-01","RRT-1003/P-01","RST-1002/P-01"]
' "${records}/conductor-plan.json" >/dev/null
if grep -E '[A-Z]+T-[0-9]{4}/C-[0-9]{2}' "${records}/conductor-plan.json"; then
  printf 'stable release plan exposed an internal Change reference\n' >&2
  exit 1
fi
grep -F 'release: v1.1.3 running' "${temporary_root}/status.txt" >/dev/null

jq '.component_closeouts[0].review_message = "Approve."' \
  "${temporary_root}/accepted-input.json" >"${temporary_root}/invalid-review-input.json"
if node "${repo_root}/ci/release_stable_spec.mjs" \
  --input "${temporary_root}/invalid-review-input.json" \
  --output "${records}/invalid-review-spec.json" \
  >"${temporary_root}/invalid-review.stdout" 2>"${temporary_root}/invalid-review.stderr"; then
  printf 'stable release spec accepted an incomplete review summary\n' >&2
  exit 65
fi
grep -F 'stable release component review is incomplete: core' \
  "${temporary_root}/invalid-review.stderr" >/dev/null

printf 'release stable recipe tests passed\n'
