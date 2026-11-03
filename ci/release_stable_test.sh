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
mkdir -p "${records}" "${workspace}"/{ait-core,ait-native,ait-server,ait-runner,ait-python,ait-node,public,web-test,personal,community}
for fixture in web-components.json candidate-ait personal-bin community-bin community-cli-bin chrome; do
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
      core_root:($workspace+"/ait-core"),native_root:($workspace+"/ait-native"),public_source:($workspace+"/public"),
      web_components:($workspace+"/web-components.json"),web_test_root:($workspace+"/web-test"),
      personal_root:($workspace+"/personal"),
      community_root:($workspace+"/community"),
      personal_bin:($workspace+"/personal-bin"),community_bin:($workspace+"/community-bin"),
      community_cli_bin:($workspace+"/community-cli-bin"),chrome:($workspace+"/chrome")
    },
    web:{server_url:"http://127.0.0.1:8088/",server_data:$workspace,seed:20260822,community_snapshot:"SNP-666666666666"},
    component_closeouts:[
      ["core","RCT-1001","SNP-111111111111"],
      ["server","RST-1002","SNP-222222222222"],
      ["runner","RRT-1003","SNP-333333333333"],
      ["python","RPT-1004","SNP-444444444444"],
      ["node","RNT-1005","SNP-555555555555"]
    ] | map({id:.[0],task:.[1],snapshot:.[2],patchset:null,
      repository_root:($workspace+"/ait-"+.[0]),edit_root:($workspace+"/ait-"+.[0]),
      remote:"origin",finish_local_before_ready:true,
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
  .release.tag == "v1.1.3" and (.phases | length) == 14 and
  [.phases[].id] == [
    "source_preflight","component_release","candidate_freeze",
    "public_qualification","candidate_admission","component_receipts","clean_host_qualification",
    "web_admission","tag","protected_promotion","endpoint_publication","winget_submission",
    "latest_alias","closeout"
  ] and
  (.phases[] | select(.id == "source_preflight") | .actions[] |
    select(.id == "source-authority-preflight") | .argv) == [
      ($workspace + "/ait-native/ci/release_authority_preflight.sh"),
      ($workspace + "/ait-native"),
      ($records + "/00-authority.json")
    ] and
  (.phases[] | select(.id == "candidate_freeze") | .actions[] |
    select(.id == "export-release-source") | .argv) == [
      ($workspace + "/ait-native/ci/release_export_source.sh"),
      ($workspace + "/ait-native"),
      "1.1.2",
      ($records + "/release-source")
    ] and
  ([.phases[].actions[] | .argv[]?, .probe.argv[]?, .dispatch_argv[]?, .bind.argv[]?] |
    map(select(type == "string" and test("/ait-core/ci/"))) | length) == 0 and
  ([.phases[] | select(.id == "component_release") | .actions[] | .patchset] | sort) ==
    [null,null,null,null,null]
' --arg workspace "${workspace}" --arg records "${records}" \
  "${records}/conductor-plan.json" >/dev/null
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

# The selected 1.2.0 owner scope compiles without Personal paths. Earlier
# releases retain the full-Web input requirement.
jq '.release.prior_version="1.1.4" | .release.version="1.2.0" |
  del(.paths.personal_root,.paths.personal_bin)' "${temporary_root}/accepted-input.json" \
  >"${temporary_root}/retained-input.json"
node "${repo_root}/ci/release_stable_spec.mjs" \
  --input "${temporary_root}/retained-input.json" --output "${records}/retained-spec.json" >/dev/null
jq -e '.values.personal_root == "" and .values.personal_bin == ""' "${records}/retained-spec.json" >/dev/null
node "${repo_root}/ci/release_conductor_init.mjs" \
  --spec "${records}/retained-spec.json" --recipe "${repo_root}/release/stable-conductor.recipe.json" \
  --plan "${records}/retained-plan.json" >/dev/null
node "${repo_root}/ci/release_conductor.mjs" status \
  --plan "${records}/retained-plan.json" --state "${records}/retained-state.json" >/dev/null
# Paused Community uses the same recipe, with no binary or Chrome paths.
jq 'del(.paths.community_root,.paths.community_bin,.paths.community_cli_bin,.paths.chrome,.web.community_snapshot)' "${temporary_root}/retained-input.json" >"${temporary_root}/paused-input.json"
node "${repo_root}/ci/release_stable_spec.mjs" --input "${temporary_root}/paused-input.json" --output "${records}/paused-spec.json" >/dev/null
jq -e '.values.community_bin == "" and .values.community_cli_bin == "" and .values.chrome == ""' "${records}/paused-spec.json" >/dev/null
node "${repo_root}/ci/release_conductor_init.mjs" --spec "${records}/paused-spec.json" --recipe "${repo_root}/release/stable-conductor.recipe.json" --plan "${records}/paused-plan.json" >/dev/null
node "${repo_root}/ci/release_conductor.mjs" status --plan "${records}/paused-plan.json" --state "${records}/paused-state.json" >/dev/null
jq '.release.prior_version="1.1.3" | .release.version="1.1.4"' "${temporary_root}/retained-input.json" \
  >"${temporary_root}/legacy-missing-personal.json"
if node "${repo_root}/ci/release_stable_spec.mjs" \
  --input "${temporary_root}/legacy-missing-personal.json" --output "${records}/legacy-missing-personal-spec.json" \
  >"${temporary_root}/legacy-missing-personal.stdout" 2>"${temporary_root}/legacy-missing-personal.stderr"; then
  printf 'legacy full-Web input accepted missing Personal\n' >&2
  exit 1
fi
printf 'release stable recipe tests passed\n'
