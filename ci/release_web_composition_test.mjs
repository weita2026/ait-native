import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync, readdirSync, cpSync, rmSync, chmodSync, realpathSync, symlinkSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import os from "node:os";
import { runReceiptTransaction, digest } from "./release_receipt_transaction.mjs";
import { installedCoreCases } from "./release_core_installed_cases.mjs";
import { webAcceptanceTree, validateComposedWebCoverage, validateWebDirect } from "./release_web_controls.mjs";
import { sourceAssertions } from "./release_web_composition.mjs";
import { previousCohort, correctedCohort } from "./release_web_applicability.mjs";
import { runCorrectedCohortFixture } from "./release_web_applicability_fixture.mjs";

// Isolated protocol fixtures. No fixture is a release result and no actual WT,
// candidate, Server, Runner, CI, review or publication action is executed.
const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-web-composition-")));
const here = path.dirname(fileURLToPath(import.meta.url));
const sha = file => createHash("sha256").update(readFileSync(file)).digest("hex");
const bytesHash = value => createHash("sha256").update(value).digest("hex");
const save = (file, value) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, JSON.stringify(value) + "\n"); };
const textFile = (file, value) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, value); };
const read = file => JSON.parse(readFileSync(file));
const phases = ["source_preflight", "component_release", "candidate_freeze", "public_qualification", "candidate_admission", "component_receipts",
  "clean_host_qualification", "web_admission", "tag", "protected_promotion", "endpoint_publication", "winget_submission", "latest_alias", "closeout"];
const makePhases = () => phases.map(id => ({ id, actions: [{ id: `${id}-fixture`, type: "evidence_gate", path: path.join(root, "fixture-marker"), message: "fixture" }] }));
const repository = path.join(root, "ait-web-test"), original = path.join(root, "original"), inputPath = path.join(original, "web-admission/qualification-input.json");
const release = { version: "1.2.0", prior_version: "1.1.4", channel: "stable", tag: "v1.2.0" };
const components = { ...previousCohort };
const config = { repo_name: "ait-web-test", repository_index: 23, is_worktree: false,
  workflow_default_scope: Object.fromEntries(["workflow", "task", "change"].map(k => [k, { value: "local", source: "core_route" }])),
  agent_teams: true, sprint: { value: "on", enabled: true, source: "repo_config" }, default_author_mode: "ai_with_human_review", default_model: null,
  task_review: { value: "automatic", source: "fixture" }, id_namespace_prefix: { value: "WT", source: "repo_config" } };
const status = { current_line: "main", head_snapshot_id: components.web_test, workspace_dirty: false, workspace_changed_count: 0, worktree_name: null,
  worktree_hygiene: { manual_review_candidate_count: 0, stale_count: 0 }, reconciliation: { total_finding_count: 0 } };
const queue = { summary: { attention_required_count: 0, dirty_worktree_count: 0 }, remote: { reviewer_inbox: { count: 0 } } };
let caseSequence = 0;
try {
  textFile(path.join(root, "fixture-marker"), "fixture\n");
  const cli = path.join(root, "bin/ait"), server = path.join(root, "bin/ait-server"), dispatch = path.join(root, "unexpected-dispatch");
  textFile(cli, `#!${process.execPath}
const fs=require('fs'),path=require('path'),assert=require('assert/strict');
const root=${JSON.stringify(root)},repo=${JSON.stringify(repository)},dispatch=${JSON.stringify(dispatch)},args=process.argv.slice(2);
const bank=path.join(root,'bounded-fixture-inputs'),fork=path.join(bank,'native.current.reviewed-fork');
const body=suffix=>JSON.parse(fs.readFileSync(path.join(fork,'ait.semantic.change.create'+suffix+'.stdout')));
const physical=path.join(root,'owned-fork-physical'),alias=path.join(repo,'.ait-worktree-links/lwtt-0001');
const reply=value=>process.stdout.write(JSON.stringify(value)+'\\n');
if(args[0]==='--fixture-installed'){
 assert.equal(fs.realpathSync(process.argv[1]),fs.realpathSync(fs.readFileSync(path.join(root,'expected-installed-cli'),'utf8').trim()));
 fs.appendFileSync(path.join(root,'installed-binding-dispatch'),'candidate\\n');reply({fixture:true});
}else if(args.join(' ')==='config show --json')reply(JSON.parse(fs.readFileSync(path.join(bank,'closeout/final-config.json'))));
else if(args.join(' ')==='status --json')reply({repo_name:'ait-web-test',line_name:'main',workspace:{dirty:false,changed_count:0},worktree:null,
 head_snapshot_id:fs.existsSync(path.join(root,'corrected-wt'))?fs.readFileSync(path.join(root,'corrected-wt'),'utf8').trim():${JSON.stringify(components.web_test)}});
else if(args.join(' ')==='status --json --full')reply(JSON.parse(fs.readFileSync(path.join(bank,'closeout/final-status.json'))));
else if(args.join(' ')==='queue summary --remote origin --json')reply(JSON.parse(fs.readFileSync(path.join(bank,'closeout/final-queue.json'))));
else if(args.join(' ')==='worktree list --json')reply([]);
else if(args.join(' ')==='repo show --remote origin --json')reply({repository:{repository_index:23,repository_name:'ait-web-test',namespace:'WT',tombstoned:false}});
else if(args[0]==='snapshot'&&args[1]==='diff'){
 const file=path.join(root,'approved-diff-fixture.json'),diffs=fs.existsSync(file)?JSON.parse(fs.readFileSync(file)):{};
 reply(diffs[args[2]+':'+args[3]]??{old_snapshot_id:args[2],new_snapshot_id:args[3],mode_changed:[],
 files:['run-randomized-commands.sh','randomized-self-test.sh','randomized-collection-self-test.sh'].map((name,index)=>({path:'acceptance/'+name,status:index?'added':'modified'}))});
}
else if(args[0]==='task'&&args[1]==='start'){
 assert(args.includes('--local')&&args.includes('--full'));fs.mkdirSync(path.join(physical,'fixtures'),{recursive:true});fs.mkdirSync(path.join(physical,'.ait'));fs.mkdirSync(path.dirname(alias),{recursive:true});fs.symlinkSync(physical,alias);
 fs.appendFileSync(path.join(root,'fork-dispatch'),'fork\\n');
 reply({task_id:'LWTT-0001',change:null,worktree:{name:'lwtt-0001',path:physical,open_path:alias,current_line:'feature/lwtt-0001'}});
}else if(args[0]==='task'&&args[1]==='abandon'){
 assert.equal(args[2],'LWTT-0001');fs.rmSync(alias,{force:true});fs.rmSync(physical,{recursive:true,force:true});reply({task_id:args[2],status:'abandoned'});
}else if(args[0]==='snapshot'&&args[1]==='create'){
 assert.equal(args[2],'LWTT-0001');assert.equal(process.cwd(),physical);const message=args[args.indexOf('--message')+1];
 reply(message==='ait-web-test-reviewed-fork-anchor'?body('.original'):body('.correction'));
}else if(args[0]==='snapshot'&&args[1]==='show'){
 assert.equal(args[2],body('.original').snapshot_id);reply(body('.original'));
}else if(args[0]==='change'&&args[1]==='create'){
 assert.equal(args[2],'LWTT-0001');assert.equal(process.cwd(),physical);assert.equal(args[args.indexOf('--fork-snapshot')+1],body('.original').snapshot_id);reply(body(''));
}else if(args[0]==='change'&&args[1]==='show'){
 assert.equal(args[2],'LWTT-0001');reply(body(''));
}else if(args[0]==='worktree'&&args[1]==='show'){
 assert.equal(args[2],'lwtt-0001');assert.equal(fs.realpathSync(alias),physical);reply(body('.worktree'));
}else{fs.writeFileSync(dispatch,args.join(' '));process.exit(99);}
`); chmodSync(cli, 0o755);
  textFile(server, "#!/bin/sh\nexit 99\n"); chmodSync(server, 0o755);
  const matrix = "init\tleaf\tpositive\tpositive\tinit\tnone\ntask show\tleaf\tpositive\tpositive\tshow\tnone\n";
  textFile(path.join(repository, "acceptance/manifests/ait-command-matrix.tsv"), matrix);
  for (const name of ["random-command-exclusions.tsv", "retired-command-paths.tsv", "random-command-scenarios.tsv"]) textFile(path.join(repository, "acceptance/manifests", name), "# fixture\n");
  // This boundary models the independent WT deterministic generator; it is
  // deliberately a tiny fixture, not a copy of the production implementation.
  const generator = path.join(repository, "acceptance/generate-random-command-schedule.sh");
  const forkBank = path.join(root, "bounded-fixture-inputs/native.current.reviewed-fork");
  const frozen = name => readFileSync(path.join(here, "fixtures/release-web-bounded", name), "utf8");
  textFile(path.join(repository, "acceptance/native-fork.sh"), frozen("native-fork.sh"));
  textFile(path.join(repository, "acceptance/run-native-authority-isolation.mjs"), `import fs from 'node:fs'; fs.cpSync(${JSON.stringify(path.join(root, "bounded-fixture-inputs/native.current.two-authorities/native-two-authorities"))},process.argv[4],{recursive:true});\n`);
  textFile(path.join(repository, "acceptance/lib.sh"), `awt_require_tools() { :; }
awt_validate_repository() { :; }
awt_config_capture() { :; }\nawt_config_set() { printf '{}\\n'; }\nawt_config_restore() { :; }
awt_config_guard() { trap 'awt_bounded_cleanup' EXIT; }
awt_state_digest() { printf '3:3\\n'; }
${frozen("capture-functions.sh")}
awt_append_case_result() { printf '%s\\t%s\\t%s\\t%s\\t%s\\n' "$@" >>"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv"; }
awt_run_case() { awt_id=$1; shift; "$@" || return; awt_append_case_result "$awt_id" pass 0 3:3 3:3; }
`);
  textFile(path.join(repository, "acceptance/run-ait-solo-local.sh"), `#!/bin/sh
touch '${dispatch}'
exit 99
awt_semantic_case() {
  [ "$1" = init ] || return 99
  [ ! -e '${root}/fail-init' ] || return 7
  for file in '${root}/bounded-fixture-inputs/ait.semantic.init/'*; do [ "$(basename "$file")" = cases.tsv ] || cp "$file" "$AIT_WEB_TEST_RESULTS_DIR/"; done
}
${frozen("solo-functions.sh")}
`);
  for (const marker of ["a", "b"]) textFile(path.join(repository, `acceptance/fixtures/local-marker-${marker}.txt`), marker + "\n");
  const unusedScenarios = ["repository_read_one", "repository_read", "line_snapshot_read", "plan_read", "review_evidence_one",
    "review_evidence_read", "storage_worktree_one", "storage_worktree_read", "boundary_rejection_one", "boundary_rejection"];
  textFile(path.join(repository, "acceptance/run-randomized-commands.sh"), `#!/bin/sh
# Fixture run boundary; scenario validation must never execute this top level.
while [ "$#" -gt 0 ]; do case "$1" in --output-dir) output=$2;; esac; shift 2; done
[ ! -e "$output" ] || exit 99
mkdir -p "$output"
cp -R '${root}/bounded-fixture-inputs/randomized_3000/.' "$output/"
printf 'run\\n' >>'${root}/random-dispatch'
exit 0
awt_json_string() {
  jq -Rn --arg value "$1" '$value'
}
awt_leaf_contract() {
  awt_command=$1; awt_token=$2; set -- $awt_command
  awt_success_text "$awt_command" "$awt_ait_bin" "$@" --help || return
  awt_expected_failure "$awt_command" "$awt_ait_bin" "$@" "--ait-web-test-invalid-$awt_token"
}
awt_task_change_read() {
  awt_success_json 'task show' '.task_id == "RWTT-0001"' "$awt_ait_bin" task show RWTT-0001 --remote origin --json
}
${unusedScenarios.map(name => `awt_${name}() {\n  return 64\n}`).join("\n")}
awt_execute_scenario() {
  case "$1" in
    leaf-contract) awt_leaf_contract "$2" "$5" ;;
    task-change-read) awt_task_change_read "$3" "$4" ;;
    *) return 64 ;;
  esac
}
`);
  textFile(generator, `#!/bin/sh\nset -eu\nwhile [ "$#" -gt 0 ]; do case "$1" in --seed) seed=$2;; --output) output=$2;; esac; shift 2; done\nawk -v seed="$seed" 'BEGIN { for(i=1;i<=3000;i++) { scenario=i<=2?"leaf-contract":"task-change-read"; command=i==1?"init":(i==2?"task show":"-"); print i "\\t" scenario "\\t" command "\\t1\\t1\\tr" seed "-" i } }' >"$output"\nprintf '{"schedule_cksum":"fixture-%s"}\\n' "$seed"\n`);
  const copy = path.join(root, "fixture-producer.mjs");
  textFile(copy, "import fs from 'node:fs'; fs.cpSync(process.argv[2],process.argv[3],{recursive:true});\n");
  const zip = (file, members, raw = false) => {
    const result = spawnSync("python3", ["-c", "import zipfile,json,sys\nwith zipfile.ZipFile(sys.argv[1],'w') as z:\n for n,s in json.load(sys.stdin).items(): z.writestr(n,s)", file],
      { input: JSON.stringify(Object.fromEntries(Object.entries(members).map(([n, v]) => [n, raw ? v : JSON.stringify(v) + "\n"]))), encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
  };
  const sourceCommit = "f".repeat(40), qCommit = "e".repeat(40), candidateDigest = "sha256:" + "d".repeat(64);
  const q = { contract: "ait.release.pre-rc-qualification/v1", status: "qualified", source: { git_commit: qCommit, workflow_control_commit: qCommit },
    gates: Object.fromEntries(["core_quality", "command_inventory", "environment_inventory", "release_controls"].map(k => [k, "pass"])),
    immutable_release_tag_created: false, public_endpoint_writes: false, release_receipts_created: false };
  const qZip = path.join(root, "q-carrier.zip"); zip(qZip, { "ait-release.pre-rc-qualification.json": q });
  const qLogs = path.join(root, "q-logs.zip"), qRun = path.join(root, "q-run.json");
  zip(qLogs, { "Canonical core quality gate/4_Run the canonical core quality module.txt": sourceAssertions.map(s => `test ${s} ... ok\n`).join("") }, true);
  save(qRun, { id: 1001, run_attempt: 1, head_sha: qCommit, status: "completed", conclusion: "success" });
  const qualified = { contract: "ait.release.operator.qualification-binding/v1", status: "qualified_for_version_only_release", source: { git_commit: qCommit },
    workflow: { artifact_digest: "sha256:" + sha(qZip), evidence_sha256: bytesHash(JSON.stringify(q) + "\n"), control_commit: qCommit, run_id: 1001, run_attempt: 1 } };
  const eRelease = { id: "REL-FAM-1111111111111111", version: "1.2.0", channel: "stable", source_commit: sourceCommit, candidate_artifact_digest: candidateDigest };
  const members = {}, inventory = [];
  for (const target of ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu", "aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"]) {
    const file = `rows/github-product-${target}.json`;
    members[file] = { status: "pass", failures: [], release: eRelease, row: { target }, phases: { install: { status: "pass", observations: { installed_core_cases: {
      core_snapshot: components.core, source_commit: sourceCommit, cases: installedCoreCases.map(([suite, selector, mode]) => ({ target: suite, selector, mode: mode ?? "fixture" })) } },
    commands: installedCoreCases.map(([suite, selector]) => ({ label: `installed Core ${suite}/${selector}`, exit_status: 0,
      stdout_tail: `running 1 test\ntest ${selector} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.1s\n` })) } } };
    inventory.push({ path: file, sha256: bytesHash(JSON.stringify(members[file]) + "\n") });
  }
  const aggregate = { contract: "ait.release.clean-host.aggregate/v1", status: "qualified", failures: [], release: eRelease, evidence_inventory: inventory };
  members["ait-release.clean-host-status.json"] = aggregate;
  const eZip = path.join(root, "e-aggregate.zip"); zip(eZip, members);
  const qualification = { aggregate_artifact_digest: "sha256:" + sha(eZip), aggregate_status_sha256: bytesHash(JSON.stringify(aggregate) + "\n"), candidate_artifact_digest: candidateDigest };
  const candidate = { contract: "ait.release.operator.pre-tag-candidate-binding/v1", status: "ready_for_immutable_tag", release: eRelease,
    qualification, candidate_authority: { sha256: "a".repeat(64), record: { prior_release: { version: "1.1.4" } } } };
  save(path.join(root, "candidate.json"), candidate); save(path.join(root, "pre-rc-qualification.json"), qualified);
  const input = { contract: "ait.release.web-admission-input/v1", release: { id: eRelease.id, version: "1.2.0", channel: "stable" }, components,
    executables: { ait: { path: cli, sha256: sha(cli) }, server: { path: server, sha256: sha(server) } }, browser_evidence: {},
    candidate_authority_sha256: candidate.candidate_authority.sha256, qualification, execution: { random_seed: 20261029, random_groups: 3000 },
    web_test: { repository_index: 23, namespace: "WT", head_snapshot_id: components.web_test, acceptance_set_sha256: webAcceptanceTree(path.join(repository, "acceptance")).sha256 } };
  const componentPath = path.join(root, "components.json"); save(componentPath, components);
  const actions = [
    ["qualify-public-source", "pre-rc-qualification.json", qualified], ["qualify-frozen-candidate", "candidate.json", candidate],
    ["bind-web-admission-input", "web-admission/qualification-input.json", input],
  ].map(([id, name, value]) => {
    const source = path.join(root, "fixture-inputs", name), output = path.join(original, name); save(source, value);
    return { id, type: "command", mutation: false, cwd: repository, outputs: [output], argv: [process.execPath, copy, source, output,
      ...(id === "bind-web-admission-input" ? ["--candidate", path.join(original, "candidate.json"), "--components", componentPath] : [])] };
  });
  const oldPhases = makePhases();
  oldPhases.find(p => p.id === "public_qualification").actions = [actions[0]];
  oldPhases.find(p => p.id === "clean_host_qualification").actions = [actions[1]];
  oldPhases.find(p => p.id === "web_admission").actions = [actions[2], { id: "run-web-admission", type: "command", outputs: [path.join(original, "web-admission/web-admission.json")], argv: [] }];
  const oldPlan = { contract: "ait.release.conductor-plan/v1", records_root: original, release, source: { control_files: {} }, phases: oldPhases };
  const oldPlanFile = path.join(original, "conductor-plan.json"); save(oldPlanFile, oldPlan);
  const oldState = { contract: "ait.release.conductor-state/v1", plan_sha256: sha(oldPlanFile), release,
    phases: oldPhases.map(p => ({ id: p.id, actions: p.actions.map(a => ({ id: a.id, status: "pending" })) })) };
  for (const action of actions) {
    mkdirSync(path.dirname(action.outputs[0]), { recursive: true });
    let execution = action;
    if (action.id === "bind-web-admission-input") {
      const entries = { "starting-config.json": config, "starting-status.json": status, "starting-queue.json": queue,
        "starting-worktrees.json": [], "remote-repository.json": { repository: { repository_index: 23 } }, "acceptance-set.sha256": input.web_test.acceptance_set_sha256 };
      const outputs = [inputPath, ...Object.keys(entries).map(name => path.join(original, "web-admission/evidence", name))];
      for (const output of outputs) mkdirSync(path.dirname(output), { recursive: true });
      const program = `const fs=require('fs'),values=JSON.parse(process.argv[2]);fs.copyFileSync(process.argv[1],process.argv[3]);for(let i=0;i<values.length;i++)fs.writeFileSync(process.argv[i+4],JSON.stringify(values[i])+'\\n');`;
      execution = { ...action, outputs, argv: [process.execPath, "-e", program, action.argv[2], JSON.stringify(Object.values(entries)), ...outputs] };
    }
    const result = runReceiptTransaction(original, action.id, { plan_sha256: oldState.plan_sha256, release, action, replacements: {} }, execution, {}, path.join(root, action.id + ".log"));
    assert.equal(result.status, 0, result.stderr);
    const recorded = oldState.phases.flatMap(p => p.actions).find(a => a.id === action.id);
    Object.assign(recorded, { status: "complete", outputs: Object.fromEntries(execution.outputs.map(f => [f, digest(f)])) });
    save(path.join(original, "conductor-checkpoints", action.id + ".json"), { plan_sha256: oldState.plan_sha256, action: recorded });
  }
  save(path.join(original, "conductor-state.json"), oldState);
  save(path.join(original, "web-admission/evidence/starting-config.json"), config);
  save(path.join(original, "qualified-public-source/ait-monorepo-source.json"), { schema: "ait.release.monorepo-source/v1",
    subtrees: [{ source_repository: "ait-core", source_snapshot: components.core }] });
  textFile(path.join(original, "native-runtime-bin/ait-runner"), "selected runner bytes\n");
  const partial = path.join(repository, ".ait-runtime/direct-test-results/solo_local-solo-local-original");
  const completed = ["inventory.ait", "ait.help.init", "ait.invalid.init", "ait.help.task.show", "ait.invalid.task.show"];
  textFile(path.join(partial, "cases.tsv"), completed.map(id => `${id}\tpass\t0\t1:1\t1:1\n`).join("") + "ait.semantic.init\tfail\t1\t1:1\t2:2\n");
  textFile(path.join(partial, "first-failure.tsv"), "ait.semantic.init\t1\t1:1\t2:2\n");
  textFile(path.join(partial, ".initial-authority-digest"), "1:1\n"); textFile(path.join(partial, "recovery.tsv"), "ait.semantic.init\tpass\n");
  save(path.join(partial, "ait.semantic.init.stdout"), { action: "reinitialized", agent_team_bootstrap: { status: "pending" } });
  textFile(path.join(partial, "ait.semantic.init.stderr"), ""); textFile(path.join(partial, "ait.semantic.init.argv"), `${cli}\ninit\n--json\n`);
  save(path.join(partial, "direct-run.config-original.json"), config);
  textFile(path.join(original, "web-admission/private-logs/solo_local.marker"), partial + "\n");
  const foundation = path.join(original, "web-admission/results/foundation");
  const inventorySummary = { contract: "ait-web-test.direct-run.v1", status: "pass", mode: "solo_remote", phase: "inventory",
    initial_authority_digest: "1:1", final_authority_digest: "1:1", command_nodes: 2, community_surfaces: 0, personal_surfaces: 0 };
  textFile(path.join(foundation, "cases.tsv"), "inventory.ait\tpass\ninventory.authority-unchanged\tpass\n");
  save(path.join(foundation, "summary.json"), inventorySummary);
  save(path.join(original, "web-admission/receipts/foundation.json"), { contract: "ait.release.web-admission-step/v1", status: "pass", step: "foundation",
    input_sha256: sha(inputPath), evidence: { path: path.join(foundation, "summary.json"), sha256: sha(path.join(foundation, "summary.json")) } });
  const history = path.join(root, "reviewed-history");
  textFile(path.join(history, "candidate-bin/ait-server"), readFileSync(server));
  textFile(path.join(history, "native-runtime-bin/ait-runner"), "selected runner bytes\n");
  for (const name of ["local-finish", "explicit-rebase"]) {
    const scene = path.join(history, "native-scenes", name), pinPath = path.join(scene, "conductor-receipts/component.package-identity.json");
    const pin = { repository_index: 4, package_hash: (name === "local-finish" ? "b" : "c").repeat(64), worker_job_index: 1,
      base_snapshot_id: "SNP-AAAAAAAAAAAA", revision_snapshot_id: "SNP-BBBBBBBBBBBB" };
    save(pinPath, pin);
    save(path.join(scene, "conductor-plan.json"), { release, phases: [{ id: "component_release", actions: [{ id: "component", type: "ait_closeout",
      task: "LT-0002", snapshot: pin.revision_snapshot_id, ait_bin: cli, remote: "origin" }] }] });
    const finish = { contract: "ait-agent-action/v1", mode: "workflow_package", task_id: "LT-0002", remote_name: "origin", expected_identity_path: pinPath,
      expected_identity: pin, apply_status: "done", closeout_status: "complete", next_action: { code: "done" },
      completion: { expected_identity: pin, outcome: "confirmed", entries: [{ local_task_id: "LT-0002", remote_task_id: "RT-0001", changes: [] }],
        patchset_id: "RT-0001/P-02", submission_id: "RT-0001/L-01" } };
    save(path.join(scene, "conductor-logs", name === "local-finish" ? "component.ait.finish.log" : "component.ait.finish-recovery.log"), finish);
    if (name === "explicit-rebase") {
      textFile(path.join(scene, "conductor-logs/component.ait.finish.log"), "");
      save(path.join(scene, "conductor-logs/component.ait.finish-interruption.json"), { status: "interrupted_after_formal_job", stdout_bytes: 0, worker_job_index: 1 });
    }
  }
  const bank = path.join(root, "bounded-fixture-inputs"), names = ["ait.semantic.init", "native.current.reviewed-fork", "native.current.two-authorities", "randomized_3000", "closeout"];
  const capture = (directory, name, body, args = ["fixture"]) => {
    save(path.join(directory, `${name}.stdout`), body); textFile(path.join(directory, `${name}.stderr`), "");
    textFile(path.join(directory, `${name}.argv`), [cli, ...args].join("\n") + "\n");
  };
  for (const name of names.slice(0, 3)) textFile(path.join(bank, name, "cases.tsv"), `${name}\tpass\t0\t2:2\t3:3\n`);
  capture(path.join(bank, names[0]), names[0], { action: "reinitialized", repo_name: "ait-web-test", repairs: [], repo_root: repository,
    authority_path: path.join(repository, ".ait"), default_author_mode: config.default_author_mode,
    agent_team_bootstrap: { status: "ready", session_readiness: "ready", synchronization: { status: "ready" }, work_started: false, session_id: "genuine-WT-fixture", target_session_id: "genuine-WT-fixture" } }, ["init", "--json"]);
  const fork = path.join(bank, names[1]), anchor = { snapshot_id: "SNP-CCCCCCCCCCCC", parent_snapshot_id: "SNP-AAAAAAAAAAAA", line_name: "feature/lwtt-0001", message: "ait-web-test-reviewed-fork-anchor", files: [{ path: "marker.txt", blob_id: "BLB-fixture" }] };
  const change = { task_id: "LWTT-0001", change_ref: "LWTT-0001/C-01", change_id: "C-01", status: "draft", fork_snapshot_id: anchor.snapshot_id };
  const forkWorktree = { name: "lwtt-0001", path: path.join(root, "owned-fork-physical"), alias_path: path.join(repository, ".ait-worktree-links/lwtt-0001"), open_path: path.join(repository, ".ait-worktree-links/lwtt-0001"), bound_task_id: change.task_id, bound_change_ref: null, repo_root: repository };
  capture(fork, "ait.semantic.change.create.original", anchor, ["snapshot", "show", anchor.snapshot_id, "--json"]);
  capture(fork, "ait.semantic.change.create", change, ["change", "create", change.task_id, "--title", "ait-web-test-local-reviewed-correction", "--fork-snapshot", anchor.snapshot_id, "--local", "--json"]);
  capture(fork, "ait.semantic.change.create.preserved", anchor, ["snapshot", "show", anchor.snapshot_id, "--json"]);
  capture(fork, "ait.semantic.change.create.owner", change, ["change", "show", change.task_id, "--local", "--json"]);
  capture(fork, "ait.semantic.change.create.worktree", forkWorktree, ["worktree", "show", forkWorktree.name, "--json"]);
  capture(fork, "ait.semantic.change.create.correction", { task_id: change.task_id, change_id: "C-01", snapshot_id: "SNP-DDDDDDDDDDDD", parent_snapshot_id: anchor.snapshot_id },
    ["snapshot", "create", change.task_id, "--message", "ait-web-test-reviewed-fork-correction", "--json", "--full"]);
  for (const [label, wrapper] of [["", "awt_native_fork_in_dir"], [".correction", "awt_in_dir"]]) {
    const file = path.join(fork, `ait.semantic.change.create${label}.argv`);
    textFile(file, `${wrapper}\n${forkWorktree.path}\n${readFileSync(file, "utf8")}`);
  }
  const dual = path.join(bank, names[2], "native-two-authorities"); let seq = 0;
  const cap = (label, body, args) => capture(dual, `${String(++seq).padStart(3, "0")}.${label}`, body, args);
  for (const [name, port] of [["server-a", 12345], ["server-b", 12346]]) textFile(path.join(dual, name + ".argv"), `${server}\n--data\n${root}/${name}\n--listen\n127.0.0.1:${port}\n--init-if-missing\n--defer-ci-admission\n`);
  cap("register-a", {}, ["remote", "add", "origin", "http://127.0.0.1:12345", "--default", "--json"]);
  cap("register-b", {}, ["remote", "add", "selected-b", "http://127.0.0.1:12346", "--default", "--json"]);
  cap("default-a-on-b-client", {}, ["remote", "add", "origin", "http://127.0.0.1:12345", "--default", "--json"]);
  for (const [label, remote] of [["authority-a", "origin"], ["authority-b", "selected-b"]]) cap(label, { repository: { repository_index: 7 } }, ["repo", "show", "--remote", remote, "--json"]);
  for (const [remote, intent, name] of [["origin", "Native authority A", "owned-a"], ["selected-b", "Native authority B", "owned-b"]])
    cap(`start-${remote}`, { task_id: "RT-0001", worktree: { name } }, ["task", "start", "--title", intent, "--intent", intent, "--remote", remote, "--json", "--full"]);
  cap("selected-a", { task_id: "RT-0001", intent: "Native authority A" }, ["task", "show", "RT-0001", "--remote", "origin", "--json"]);
  cap("selected-b", { task_id: "RT-0001", intent: "Native authority B" }, ["task", "show", "RT-0001", "--remote", "selected-b", "--json"]);
  const wa = { repo_root: "/owned/a", bound_task_id: "RT-0001", bound_change_ref: null, exists: true, capture_authority: { remote_name: "origin", repository_index: 7 }, head_snapshot_id: "SNP-AAAAAAAAAAAA" };
  const wb = { repo_root: "/owned/b", bound_task_id: "RT-0001", bound_change_ref: null, exists: true, capture_authority: { remote_name: "selected-b", repository_index: 7 }, current_line: "feature/rt-0001" };
  cap("worktree-a", wa, ["worktree", "show", "owned-a", "--json"]); cap("worktree-b", wb, ["worktree", "show", "owned-b", "--json"]);
  cap("capture-b", { snapshot_id: "SNP-BBBBBBBBBBBB", capture_binding: { contract: "ait.snapshot_capture_binding.v2", repository_index: 7,
    task_id: "RT-0001", change_ref: null, snapshot_id: "SNP-BBBBBBBBBBBB", line_name: wb.current_line } }, ["snapshot", "create", "RT-0001", "--message", "Native authority B only", "--json", "--full"]);
  cap("immutable-b-snapshot", { snapshot_id: "SNP-BBBBBBBBBBBB", message: "Native authority B only", files: [{ path: "selected-authority.txt" }] }, ["snapshot", "show", "SNP-BBBBBBBBBBBB", "--json"]);
  for (const label of ["a-unchanged-after-b-capture", "a-head-after-b-cleanup"]) cap(label, wa, ["worktree", "show", "owned-a", "--json"]);
  for (const label of ["a-still-active", "a-unchanged-by-b-cleanup"]) cap(label, { task_id: "RT-0001", status: "active" }, ["task", "show", "RT-0001", "--remote", "origin", "--json"]);
  for (const remote of ["selected-b", "origin"]) for (const action of ["cleanup", "cleaned"])
    cap(`${action}-${remote}`, { task_id: "RT-0001", status: "abandoned" }, ["task", action === "cleanup" ? "abandon" : "show", "RT-0001", "--remote", remote, "--json"]);
  cap("same-id-explicit-a-from-b-client", { task_id: "RT-0001", intent: "Native authority A" }, ["task", "show", "RT-0001", "--remote", "origin", "--json"]);
  const random = path.join(bank, names[3]); mkdirSync(random, { recursive: true });
  const schedule = spawnSync("sh", [generator, "--seed", "20261029", "--output", path.join(random, "schedule.tsv")], { encoding: "utf8" }); assert.equal(schedule.status, 0, schedule.stderr);
  textFile(path.join(random, "groups.tsv"), Array.from({ length: 3000 }, (_, i) => `${i + 1}\t${i < 2 ? "leaf-contract" : "task-change-read"}\t${i === 0 ? "init" : i === 1 ? "task show" : "-"}\tpass\t0\t${i < 2 ? 2 : 1}\t3:3\t3:3\n`).join(""));
  textFile(path.join(random, "commands.tsv"), Array.from({ length: 3000 }, (_, i) => {
    const command = i === 0 ? "init" : "task show";
    return Array.from({ length: i < 2 ? 2 : 1 }, (_, j) => {
      const id = `random.${String(i + 1).padStart(4, "0")}.${String(j + 1).padStart(2, "0")}`;
      const args = [...command.split(" "), ...(i < 2 ? [j === 0 ? "--help" : `--ait-web-test-invalid-r20261029-${i + 1}`] : ["RWTT-0001", "--remote", "origin", "--json"])];
      textFile(path.join(random, id + ".argv"), [cli, ...args].join("\n") + "\n");
      textFile(path.join(random, id + ".stdout"), i < 2 ? "help or rejection\n" : '{"task_id":"RWTT-0001"}\n'); textFile(path.join(random, id + ".stderr"), "");
      return `${i + 1}\t${j + 1}\t${id}\t${command}\t${i < 2 ? (j === 0 ? "text-success" : "nonzero-fail-closed") : "json-success"}\t${j === 1 ? 2 : 0}\tpass\t${id}.argv\t${id}.stdout\t${id}.stderr\n`;
    }).join("");
  }).join(""));
  save(path.join(random, "summary.json"), { contract: "ait-web-test.random-command-run.v1", status: "pass", mode: "solo_remote", repository_index: 23, namespace: "WT",
    seed: 20261029, requested_groups: 3000, executed_groups: 3000, replay_case: null, retirement_invocations: 0, command_invocations: 3002, allowed_commands: 2,
    initial_authority_digest: "3:3", final_authority_digest: "3:3", schedule_cksum: "fixture-20261029", scenario_counts: { "leaf-contract": 2, "task-change-read": 2998 } });
  for (const [file, value] of [["final-config.json", config], ["final-status.json", status], ["final-queue.json", queue]]) save(path.join(bank, names[4], file), value);
  textFile(path.join(bank, names[4], "internal-change-argv.tsv"), "");
  const pins = {};
  const pinTree = file => { if (readdirSafe(file)) for (const name of readdirSync(file)) pinTree(path.join(file, name)); else pins[file] = sha(file); };
  function readdirSafe(file) { try { readdirSync(file); return true; } catch { return false; } }
  const disposition = path.join(root, "docs/sprints/release_1_2_0_binding.md"); textFile(disposition, `Stage 5 assertion disposition — 2026-11-01\nFixture input ${sha(inputPath)}.\n`);
  for (const file of [original, partial, history, qZip, eZip, qLogs, qRun, cli, componentPath, disposition, path.join(here, "release_web_composition.mjs")]) pinTree(file);
  function fixture(mutate = () => {}, pending = null) {
    const operation = path.join(root, `binding-${++caseSequence}`), outputRoot = path.join(operation, "web-admission"), source = path.join(root, `source-${caseSequence}`);
    cpSync(bank, source, { recursive: true }); mutate(source);
    const planPhases = makePhases();
    const bounded = names.map((name, index) => ({ id: `bounded-${index}`, type: "command", mutation: false, cwd: repository,
      outputs: [path.join(outputRoot, "results", name)], argv: [process.execPath, copy, path.join(source, name), path.join(outputRoot, "results", name), inputPath, repository, cli,
        ...(name === "native.current.two-authorities" ? [server] : []),
        ...({ "native.current.reviewed-fork": [path.join(repository, "acceptance/native-fork.sh")],
          "native.current.two-authorities": [path.join(repository, "acceptance/run-native-authority-isolation.mjs")] }[name] ?? []),
        ...(name === "closeout" ? names.slice(0, 4).map(n => path.join(outputRoot, "results", n, n === "randomized_3000" ? "summary.json" : "cases.tsv")) : [])] }));
    planPhases.find(p => p.id === "web_admission").actions = bounded;
    const plan = { contract: "ait.release.conductor-plan/v1", records_root: operation, release, source: { control_files: pins }, phases: planPhases };
    const file = path.join(operation, "conductor-plan.json"); save(file, plan);
    for (const action of bounded) {
      if (action.id === pending) continue;
      mkdirSync(path.dirname(action.outputs[0]), { recursive: true });
      const result = runReceiptTransaction(operation, action.id, { plan_sha256: sha(file), release, action, replacements: {} }, action, {}, path.join(operation, action.id + ".log"));
      assert.equal(result.status, 0, result.stderr);
    }
    const verify = () => validateComposedWebCoverage(inputPath, input, repository, outputRoot);
    return { operation, outputRoot, verify, planFile: file, bounded };
  }
  // Real accepted input -> real stable spec/recipe -> real initializer -> all
  // generated command actions -> composed shell producer -> real verifier.
  // Only WT execution boundaries are fixture helpers. No Plan or pin is edited.
  const native = path.join(root, "native"), nativeCi = path.join(native, "ci"); mkdirSync(nativeCi, { recursive: true });
  for (const file of readdirSync(here).filter(name => name.endsWith(".mjs") || name === "release_web_admission.sh")) {
    cpSync(path.join(here, file), path.join(nativeCi, file));
  }
  chmodSync(path.join(nativeCi, "release_web_admission.sh"), 0o755);
  const reviewed = path.join(native, "docs/sprints/release_1_2_0_binding.md");
  textFile(reviewed, `### Stage 5 assertion disposition — 2026-11-01
- W: \`${path.dirname(inputPath)}\`. Input SHA \`${sha(inputPath)}\`.
- L: \`${partial}\`.
- N: \`${path.join(history, "native-scenes")}\`.
- Q: \`${qLogs}\`.
- E: \`${eZip.slice(0, -4)}/rows/github-product-<target>.json\`.
`);
  const recipePath = path.join(root, "stock-recipe.json"); cpSync(path.join(here, "../release/stable-conductor.recipe.json"), recipePath);
  let generatedSequence = 0;
  const initialize = (selectedRecipe = recipePath, mutateAccepted = () => {}) => {
    const operation = path.join(root, `generated-${++generatedSequence}`), acceptedPath = path.join(operation, "accepted-input.json");
    const accepted = { contract: "ait.release.accepted-input/v1", records_root: operation, release,
      paths: { core_root: root, native_root: native, public_source: root, web_components: componentPath, web_test_root: repository },
      web: { server_url: "http://127.0.0.1:8088/", server_data: root, seed: 20261029, acceptance_snapshot: components.web_test }, github_repository: "owner/repository", winget_fork: "owner/winget",
      component_closeouts: ["core", "server", "runner", "node", "python"].map((id, index) => ({ id, task: `RT-${String(index + 1).padStart(4, "0")}`,
        snapshot: components[id], patchset: null, finish_local_before_ready: true, remote: "origin", repository_root: root, edit_root: root,
        review_message: "Reviewed files: fixture. Findings: none. Risks: isolated fixtures. Tests: focused. Recommendation: fixture only." })) };
    mutateAccepted(accepted); save(acceptedPath, accepted);
    const specPath = path.join(operation, "release-spec.json"), planPath = path.join(operation, "conductor-plan.json");
    const spec = spawnSync(process.execPath, [path.join(nativeCi, "release_stable_spec.mjs"), "--input", acceptedPath, "--output", specPath], { encoding: "utf8" });
    assert.equal(spec.status, 0, spec.stderr);
    const result = spawnSync(process.execPath, [path.join(nativeCi, "release_conductor_init.mjs"), "--spec", specPath, "--recipe", selectedRecipe, "--plan", planPath], { encoding: "utf8" });
    return { operation, planPath, result };
  };
  const conductorFixture = readFileSync(path.join(here, "release_conductor_test.sh"), "utf8");
  const protocol = token => conductorFixture.split(`<<'${token}'\n`)[1].split(`\n${token}\n`)[0] + "\n";
  const protocolBin = path.join(root, "conductor-protocol-bin"), protocolState = path.join(root, "conductor-protocol-state");
  mkdirSync(protocolBin); mkdirSync(protocolState);
  for (const [name, token] of [["fixture", "FIXTURE"], ["gh", "GH"]]) {
    textFile(path.join(protocolBin, name), protocol(token)); chmodSync(path.join(protocolBin, name), 0o755);
  }
  const ids = ["core", "server", "runner", "node", "python"];
  for (const [index, id] of ids.entries()) {
    const task = `RT-${String(index + 1).padStart(4, "0")}`;
    const script = protocol("AIT").replaceAll("LXT-1000", task).replaceAll("SNP-ABCDEF123456", components[id])
      .replace('state=${AIT_CONDUCTOR_FIXTURE_STATE:?}', `state=\${AIT_CONDUCTOR_FIXTURE_STATE:?}/source-${id}\nmkdir -p "$state"`);
    textFile(path.join(protocolBin, `source-${id}`), script); chmodSync(path.join(protocolBin, `source-${id}`), 0o755);
  }
  const selector = `#!/bin/sh\nif [ "$1 $2" = 'config show' ]; then exec '${path.join(protocolBin, "source-core")}' "$@"; fi\ncase "$3" in\n${ids.map((id, index) => `RT-${String(index + 1).padStart(4, "0")}) exec '${path.join(protocolBin, `source-${id}`)}' "$@";;`).join("\n")}\n*) exit 99;;\nesac\n`;
  textFile(path.join(protocolBin, "ait"), selector); chmodSync(path.join(protocolBin, "ait"), 0o755);
  const conductorEnv = { ...process.env, PATH: protocolBin + path.delimiter + process.env.PATH,
    AIT_RELEASE_AIT_BIN: path.join(protocolBin, "ait"), AIT_CONDUCTOR_FIXTURE_STATE: protocolState };
  const isolatedRecipe = read(recipePath);
  for (const phase of isolatedRecipe.phases) {
    if (["source_preflight", "candidate_freeze", "candidate_admission"].includes(phase.id)) {
      const output = `{{records_root}}/${phase.id}-fixture.receipt`;
      phase.actions = [{ id: `${phase.id}-fixture`, type: "command", mutation: false, cwd: root,
        argv: [path.join(protocolBin, "fixture"), "produce", phase.id, output], outputs: [output] }];
    } else if (["public_qualification", "component_receipts", "clean_host_qualification"].includes(phase.id)) {
      const output = `{{records_root}}/${phase.id}-fixture.receipt`;
      phase.actions = [{ id: `${phase.id}-fixture`, type: "github_workflow", cwd: root, repository: "owner/repository", workflow: phase.id,
        head_sha: "a".repeat(40), dispatch_argv: [path.join(protocolBin, "fixture"), "dispatch", phase.id, "{head_sha}"],
        bind: { cwd: root, argv: [path.join(protocolBin, "fixture"), "bind", phase.id, output, "{run_id}"], outputs: [output] } }];
    }
  }
  const isolatedRecipePath = path.join(root, "isolated-conductor-recipe.json"); save(isolatedRecipePath, isolatedRecipe);
  const generated = initialize(isolatedRecipePath); assert.equal(generated.result.status, 0, generated.result.stderr);
  const generatedPlan = read(generated.planPath), generatedWeb = generatedPlan.phases.find(phase => phase.id === "web_admission").actions;
  assert.deepEqual(generatedWeb.slice(0, -1).map(action => path.basename(action.outputs[0])), names);
  assert.equal(generatedWeb.length, 6); assert.equal(generatedWeb.at(-1).id, "run-web-admission");
  assert(!generatedWeb.some(action => action.id === "bind-web-admission-input"));
  assert(generatedWeb.every(action => action.argv.includes(inputPath) && action.argv.includes(cli) && action.argv.includes(server)));
  assert(generatedWeb.at(-1).probe.argv.includes(inputPath));
  assert(generatedPlan.source.control_files[reviewed]); assert(generatedPlan.source.control_files[qZip]);
  assert(generatedPlan.source.control_files[eZip]); assert(generatedPlan.source.control_files[path.join(partial, "first-failure.tsv")]);
  assert(generatedPlan.source.control_files[path.join(history, "native-scenes/explicit-rebase/conductor-logs/component.ait.finish.log")]);
  const qBytes = readFileSync(qRun); save(qRun, { ...read(qRun), id: 2 });
  const stale = initialize(); assert.equal(stale.result.status, 75); assert(!existsSync(stale.planPath)); writeFileSync(qRun, qBytes);
  const carrierBytes = readFileSync(qZip); rmSync(qZip);
  const missing = initialize(); assert.equal(missing.result.status, 75); assert(!existsSync(missing.planPath)); writeFileSync(qZip, carrierBytes);
  const logBytes = readFileSync(qLogs); rmSync(qLogs); symlinkSync(qZip, qLogs);
  const linked = initialize(); assert.equal(linked.result.status, 75); assert.match(linked.result.stderr, /symlink/); assert(!existsSync(linked.planPath));
  rmSync(qLogs); writeFileSync(qLogs, logBytes);
  const dispositionBytes = readFileSync(reviewed); textFile(reviewed, dispositionBytes.toString() + `\n- N: \`${path.join(history, "native-scenes")}\`.\n`);
  const ambiguousInput = initialize(); assert.equal(ambiguousInput.result.status, 75); assert.match(ambiguousInput.result.stderr, /ambiguous reviewed N/); assert(!existsSync(ambiguousInput.planPath));
  writeFileSync(reviewed, dispositionBytes);
  const matched = initialize(); assert.equal(matched.result.status, 0, matched.result.stderr);
  const alternateRoot = path.join(root, "alternate-selection"), alternateComponents = path.join(alternateRoot, "web-components.json");
  save(alternateComponents, { ...components, core: "SNP-999999999999" });
  save(path.join(alternateRoot, "candidate.json"), candidate); save(path.join(alternateRoot, "pre-rc-qualification.json"), qualified);
  const wrongCohort = initialize(recipePath, accepted => {
    accepted.paths.web_components = alternateComponents;
    accepted.component_closeouts.find(row => row.id === "core").snapshot = "SNP-999999999999";
  });
  assert.equal(wrongCohort.result.status, 75); assert.match(wrongCohort.result.stderr, /selected component cohort differs/); assert(!existsSync(wrongCohort.planPath));
  const selectedCandidateFile = path.join(root, "candidate.json"), selectedCandidateBytes = readFileSync(selectedCandidateFile);
  save(selectedCandidateFile, { ...candidate, qualification: { ...qualification, aggregate_artifact_digest: "sha256:" + "9".repeat(64) } });
  const wrongCandidate = initialize(); assert.equal(wrongCandidate.result.status, 75); assert.match(wrongCandidate.result.stderr, /selected candidate\/artifact\/authority differs/); assert(!existsSync(wrongCandidate.planPath));
  writeFileSync(selectedCandidateFile, selectedCandidateBytes);
  const selectedQFile = path.join(root, "pre-rc-qualification.json"), selectedQBytes = readFileSync(selectedQFile);
  save(selectedQFile, { ...qualified, source: { ...qualified.source, git_commit: "9".repeat(40) } });
  const wrongQualification = initialize(); assert.equal(wrongQualification.result.status, 75); assert.match(wrongQualification.result.stderr, /selected source qualification differs/); assert(!existsSync(wrongQualification.planPath));
  writeFileSync(selectedQFile, selectedQBytes);

  // Exercise the actual conductor process. Existing isolated AIT/GitHub
  // protocols satisfy its prior-phase contracts; they produce no real Job,
  // review, remote write or publication. Pins come only from the real init.
  const stateFile = path.join(generated.operation, "conductor-state.json");
  const conductorArgs = [path.join(nativeCi, "release_conductor.mjs"), "run", "--plan", generated.planPath, "--state", stateFile];
  const conduct = () => spawnSync(process.execPath, conductorArgs, { encoding: "utf8", env: conductorEnv });
  const sourceActions = generatedPlan.phases.find(phase => phase.id === "component_release").actions;
  for (const action of sourceActions) {
    const local = spawnSync(conductorEnv.AIT_RELEASE_AIT_BIN, ["task", "finish", action.task, "--local", "--json"],
      { encoding: "utf8", env: { ...conductorEnv, AIT_FIXTURE_REVIEWER: "true" } });
    assert.equal(local.status, 0, local.stderr);
  }
  for (const action of sourceActions) {
    const pending = conduct(); assert.equal(pending.status, 75, pending.stderr);
    assert.match(pending.stderr, /independent Reviewer must finish/);
    const pin = path.join(generated.operation, "conductor-receipts", action.id + ".package-identity.json");
    assert(existsSync(pin), "the conductor must produce the package selector before fixture review capture");
    const finished = spawnSync(conductorEnv.AIT_RELEASE_AIT_BIN, ["workflow", "finish", action.task, "--apply", "--remote", "origin",
      "--expected-identity", pin, "--review-message", "Reviewed files: fixture. Findings: none. Risks: isolated. Tests: protocol. Recommendation: pass.", "--json"],
      { encoding: "utf8", env: { ...conductorEnv, AIT_FIXTURE_REVIEWER: "true" } });
    assert.equal(finished.status, 0, finished.stderr);
    textFile(path.join(generated.operation, "conductor-logs", action.id + ".ait.finish.log"), finished.stdout);
  }
  textFile(path.join(root, "fail-init"), "fixture failure\n");
  const failed = conduct(); assert.equal(failed.status, 75, failed.stderr); assert.match(failed.stderr, /bounded WT batch incomplete/);
  const failedState = read(stateFile).phases.find(phase => phase.id === "web_admission");
  assert.deepEqual(failedState.actions.slice(0, 5).map(action => action.status), ["action_required", "complete", "complete", "complete", "action_required"],
    readFileSync(path.join(generated.operation, "conductor-logs/bounded-web-3.log"), "utf8"));
  assert.equal(readFileSync(path.join(root, "fork-dispatch"), "utf8"), "fork\n", "failed init must not suppress an independent eligible fork");
  assert.equal(readFileSync(path.join(root, "random-dispatch"), "utf8"), "run\n");
  const randomReceipt = readFileSync(path.join(generated.operation, "conductor-receipts/bounded-web-4.json"));
  rmSync(path.join(root, "fail-init"));
  const resumed = conduct(); assert.equal(resumed.status, 75, resumed.stderr); assert.match(resumed.stderr, /Pre-tag operator HOLD/,
    readFileSync(path.join(generated.operation, "conductor-logs/bounded-web-2.log"), "utf8"));
  assert.equal(read(stateFile).phases.find(phase => phase.id === "web_admission").status, "complete");
  assert.equal(read(stateFile).phases.find(phase => phase.id === "tag").status, "action_required");
  for (const action of generatedWeb) assert(existsSync(path.join(generated.operation, "conductor-receipts", action.id + ".json")));
  assert(readFileSync(path.join(generated.operation, "conductor-receipts/bounded-web-4.json")).equals(randomReceipt));
  assert.equal(readFileSync(path.join(root, "random-dispatch"), "utf8"), "run\n", "resume must retain the successful campaign receipt");
  assert.equal(readFileSync(path.join(root, "fork-dispatch"), "utf8"), "fork\n", "resume must retain the successful fork receipt");
  const generatedAdmission = path.join(generated.operation, "web-admission/web-admission.json");
  assert.equal(read(generatedAdmission).status, "admitted");
  assert(existsSync(path.join(generated.operation, "web-admission/receipts/closeout.json")));
  assert(!existsSync(path.join(generated.operation, "web-admission/receipts/solo_local.json")));
  const generatedFork = path.join(generated.operation, "web-admission/results/native.current.reviewed-fork");
  const actualWorktree = read(path.join(generatedFork, "ait.semantic.change.create.worktree.stdout"));
  assert.notEqual(actualWorktree.path, actualWorktree.open_path);
  assert.equal(readFileSync(path.join(generatedFork, "ait.semantic.change.create.argv"), "utf8").split("\n")[1], actualWorktree.path);
  assert(!existsSync(actualWorktree.path) && !existsSync(actualWorktree.open_path), "existing owned cleanup must remove physical and alias paths");
  assert(!existsSync(dispatch), "single-case loading must not execute the solo runner's top level or another CLI leaf");
  const generatedVerified = spawnSync(process.execPath, [path.join(nativeCi, "release_web_admission_verify.mjs"), inputPath, generatedAdmission, repository], { encoding: "utf8" });
  assert.equal(generatedVerified.status, 0, generatedVerified.stderr);
  if (!process.env.AIT_WEB_RECOVERY_ONLY && !process.env.AIT_WEB_APPLICABILITY_ONLY) {
  const good = fixture();
  const coverage = good.verify();
  assert(coverage.has("ait.semantic.init") && coverage.has("ait.semantic.task.show") && coverage.has("native.current.formal-finish") && coverage.has("final.clean-state"));
  assert.equal(coverage.get("ait.semantic.task.show"), "reviewed Q/E assertion equivalence");
  assert(!existsSync(path.join(good.outputRoot, "results/ait.semantic.init/summary.json")));
  assert(!existsSync(path.join(good.outputRoot, "receipts/solo_local.json")));
  const driver = spawnSync("bash", [path.join(here, "release_web_admission.sh"), "run", "--input", inputPath, "--repository", repository, "--ait", cli,
    "--server-bin", server, "--server-data", root, "--server-url", "http://127.0.0.1:8088/", "--output-dir", good.outputRoot], { encoding: "utf8" });
  assert.equal(driver.status, 0, driver.stderr); assert.match(driver.stdout, /reviewed assertion coverage: complete/);
  assert(!existsSync(dispatch), "consumer must not invoke CLI leaves or a listener");
  const admission = path.join(good.outputRoot, "web-admission.json"), admitted = readFileSync(admission);
  const verified = spawnSync(process.execPath, [path.join(here, "release_web_admission_verify.mjs"), inputPath, admission, repository], { encoding: "utf8" });
  assert.equal(verified.status, 0, verified.stderr);
  const repeated = spawnSync("bash", [path.join(here, "release_web_admission.sh"), "run", "--input", inputPath, "--repository", repository, "--ait", cli,
    "--server-bin", server, "--server-data", root, "--server-url", "http://127.0.0.1:8088/", "--output-dir", good.outputRoot], { encoding: "utf8" });
  assert.equal(repeated.status, 0, repeated.stderr); assert(readFileSync(admission).equals(admitted)); assert(!existsSync(dispatch));
  // A bounded journal still fails the unchanged normal complete-result path.
  assert.throws(() => validateWebDirect({ ...inventorySummary, mode: "solo_local", phase: "solo-local" }, input, "solo-local", repository,
    path.join(good.outputRoot, "results/ait.semantic.init")));
  const dualArgv = (source, label, args) => {
    const directory = path.join(source, names[2], "native-two-authorities");
    const file = readdirSync(directory).find(n => n.endsWith(`.${label}.argv`));
    textFile(path.join(directory, file), [cli, ...args].join("\n") + "\n");
  };
  // Every mutation is made before the normal producer creates its receipt.
  // These are contradictory original captures, not post-receipt tampering.
  const cases = [
    ["pending actor", s => { const f = path.join(s, names[0], "ait.semantic.init.stdout"), v = read(f); v.agent_team_bootstrap.session_readiness = "pending"; save(f, v); }],
    ["wrong workspace", s => { const f = path.join(s, names[0], "ait.semantic.init.stdout"), v = read(f); v.repo_root = root; save(f, v); }],
    ["conflicting actor", s => { const f = path.join(s, names[0], "ait.semantic.init.stdout"), v = read(f); v.agent_team_bootstrap.target_session_id = "another-actor"; save(f, v); }],
    ["wrong candidate", s => textFile(path.join(s, names[0], "ait.semantic.init.argv"), `${server}\ninit\n--json\n`)],
    ["duplicate completed execution", s => textFile(path.join(s, names[0], "cases.tsv"), "ait.help.init\tpass\t0\t1:1\t1:1\nait.semantic.init\tpass\t0\t1:1\t1:1\n")],
    ["partial pass-shaped summary", s => save(path.join(s, names[0], "summary.json"), { ...inventorySummary, phase: "solo-local" })],
    ["changed reviewed anchor", s => { const f = path.join(s, names[1], "ait.semantic.change.create.preserved.stdout"), v = read(f); v.message = "overwritten"; save(f, v); }],
    ["wrong fork operation", s => textFile(path.join(s, names[1], "ait.semantic.change.create.original.argv"), `${cli}\ntask\nshow\nLWTT-0001\n--local\n--json\n`)],
    ["foreign fork path", s => { const f = path.join(s, names[1], "ait.semantic.change.create.argv"); textFile(f, readFileSync(f, "utf8").replace(forkWorktree.path, path.join(root, "foreign-worktree"))); }, /foreign worktree path/],
    ["foreign correction path", s => { const f = path.join(s, names[1], "ait.semantic.change.create.correction.argv"); textFile(f, readFileSync(f, "utf8").replace(forkWorktree.path, path.join(root, "foreign-worktree"))); }],
    ["wrong fork Task", s => { const f = path.join(s, names[1], "ait.semantic.change.create.argv"); textFile(f, readFileSync(f, "utf8").replace("\nLWTT-0001\n", "\nLWTT-0002\n")); }],
    ["wrong correction Task", s => { const f = path.join(s, names[1], "ait.semantic.change.create.correction.argv"); textFile(f, readFileSync(f, "utf8").replace("\nLWTT-0001\n", "\nLWTT-0002\n")); }],
    ["wrong equal-ID authority", s => { const f = readdirSync(path.join(s, names[2], "native-two-authorities")).find(n => n.endsWith(".authority-b.stdout")); save(path.join(s, names[2], "native-two-authorities", f), { repository: { repository_index: 8 } }); }],
    ["selected B reads default A", s => dualArgv(s, "selected-b", ["task", "show", "RT-0001", "--remote", "origin", "--json"]), /isolation capture command differs: selected-b/],
    ["selected B reads wrong Task", s => dualArgv(s, "selected-b", ["task", "show", "RT-0002", "--remote", "selected-b", "--json"]), /isolation capture command differs: selected-b/],
    ["B capture uses wrong Task", s => dualArgv(s, "capture-b", ["snapshot", "create", "RT-0002", "--message", "Native authority B only", "--json", "--full"]), /isolation capture command differs: capture-b/],
    ["B cleanup targets A", s => dualArgv(s, "cleanup-selected-b", ["task", "abandon", "RT-0001", "--remote", "origin", "--json"]), /isolation capture command differs: cleanup-selected-b/],
    ["wrong random seed", s => { const f = path.join(s, names[3], "summary.json"), v = read(f); v.seed++; save(f, v); }],
    ["random label hides retirement", s => textFile(path.join(s, names[3], "random.0003.01.argv"), `${cli}\nrepo\nretire\n23\n--remote\norigin\n--json\n`), /contradicts its recorded argv/],
    ["random wrong authority", s => textFile(path.join(s, names[3], "random.0003.01.argv"), `${cli}\ntask\nshow\nRWTT-0001\n--remote\nselected-b\n--json\n`), /random scenario argv or output predicate failed/],
    ["random wrong selected Task", s => textFile(path.join(s, names[3], "random.0003.01.argv"), `${cli}\ntask\nshow\nRWTT-0002\n--remote\norigin\n--json\n`), /random scenario argv or output predicate failed/],
    ["random malformed scenario body", s => textFile(path.join(s, names[3], "random.0003.01.stdout"), "{}\n"), /random scenario argv or output predicate failed/],
    ["missing random group", s => textFile(path.join(s, names[3], "groups.tsv"), readFileSync(path.join(s, names[3], "groups.tsv"), "utf8").split("\n").slice(1).join("\n"))],
    ["wrong random schedule", s => textFile(path.join(s, names[3], "schedule.tsv"), "wrong\n")],
    ["unclean final state", s => save(path.join(s, names[4], "final-status.json"), { ...status, workspace_dirty: true })],
    ["unrestored settings", s => save(path.join(s, names[4], "final-config.json"), { ...config, agent_teams: false })],
  ];
  for (const [name, mutate, expected] of cases) assert.throws(fixture(mutate).verify, expected, name);
  assert.throws(() => validateComposedWebCoverage(inputPath, { ...input, execution: { ...input.execution, random_seed: 1 } }, repository, good.outputRoot), /another input/);
  // Aggregate PASS cannot hide a source assertion which did not execute.
  const logsBytes = readFileSync(qLogs), logsPin = pins[qLogs];
  zip(qLogs, { "Canonical core quality gate/4_Run the canonical core quality module.txt": sourceAssertions.slice(1).map(s => `test ${s} ... ok\n`).join("") }, true);
  pins[qLogs] = sha(qLogs); assert.throws(fixture().verify, /source assertion did not execute/);
  writeFileSync(qLogs, logsBytes); pins[qLogs] = logsPin;
  const nativeFile = path.join(history, "native-scenes/local-finish/conductor-logs/component.ait.finish.log"), nativeBytes = readFileSync(nativeFile), nativePin = pins[nativeFile];
  const wrongJob = read(nativeFile); wrongJob.expected_identity.worker_job_index = 2; save(nativeFile, wrongJob); pins[nativeFile] = sha(nativeFile);
  assert.throws(fixture().verify, /identity/); writeFileSync(nativeFile, nativeBytes); pins[nativeFile] = nativePin;
  assert.throws(fixture(() => {}, "bounded-0").verify, /pending/);
  const corrupt = path.join(good.outputRoot, "results/ait.semantic.init/ait.semantic.init.stdout"), originalBytes = readFileSync(corrupt);
  textFile(corrupt, "{}\n"); assert.throws(good.verify, /output changed/); writeFileSync(corrupt, originalBytes);
  const priorFailure = path.join(partial, "first-failure.tsv"), priorBytes = readFileSync(priorFailure);
  textFile(priorFailure, "relabelled\n"); assert.throws(good.verify, /changed composition evidence/); writeFileSync(priorFailure, priorBytes);
  const planBytes = readFileSync(good.planFile), ambiguous = read(good.planFile);
  ambiguous.phases.find(p => p.id === "web_admission").actions.push({ ...ambiguous.phases.find(p => p.id === "web_admission").actions[0], id: "second-init" });
  save(good.planFile, ambiguous); assert.throws(good.verify); writeFileSync(good.planFile, planBytes);
  save(path.join(good.outputRoot, "receipts/solo_local.json"), { status: "pass" }); assert.throws(good.verify, /synthetic/);
  assert(!existsSync(dispatch));
  }

  // Reviewed assertion correction: retain the old W producer and create a new
  // ordinary input. Existing noncanonical FAIL/dual/random roots are adopted
  // only through generated native transactions. Semantic-init stays HOLD.
  const recoveryBase = path.join(root, "records-stage5-composition-20261101"), recoveryResults = path.join(recoveryBase, "web-admission/results");
  const retainedInit = path.join(recoveryResults, "ait.semantic.init-wt-1"), retainedDual = path.join(recoveryResults, names[2]);
  cpSync(path.join(bank, names[0]), retainedInit, { recursive: true }); cpSync(path.join(bank, names[2]), retainedDual, { recursive: true });
  textFile(path.join(retainedInit, "cases.tsv"), "ait.semantic.init\tfail\t65\t2:2\t3:3\n");
  textFile(path.join(retainedInit, "first-failure.tsv"), "ait.semantic.init\t65\t2:2\t3:3\n");
  const partialBody = read(path.join(retainedInit, "ait.semantic.init.stdout"));
  partialBody.agent_team_bootstrap.status = "pending"; partialBody.agent_team_bootstrap.session_readiness = "pending";
  partialBody.agent_team_bootstrap.detail = "Calling conversation belongs to another Agent; no adoption attempted";
  save(path.join(retainedInit, "ait.semantic.init.stdout"), partialBody);
  const collection = path.join(recoveryResults, "randomized_3000-collection-1"), rawBank = path.join(bank, names[3]);
  const allGroups = readFileSync(path.join(rawBank, "groups.tsv"), "utf8").trim().split("\n").map(line => line.split("\t"));
  const allCommands = readFileSync(path.join(rawBank, "commands.tsv"), "utf8").trim().split("\n").map(line => line.split("\t"));
  const priorRoots = [path.join(repository, ".ait-runtime/stage5-random-HZU51i/run"),
    ...[8, 10, 12, 13].map(index => path.join(repository, `.ait-runtime/stage5-replay-20261101-case${index}`))];
  const metadata = readFileSync(path.join(rawBank, "schedule.tsv")), scheduleMetadata = { schedule_cksum: "fixture-20261029" };
  const materialize = (directory, commands, groups, actualExit = false) => {
    textFile(path.join(directory, "schedule.tsv"), metadata); save(path.join(directory, "schedule-summary.json"), scheduleMetadata);
    textFile(path.join(directory, "commands.tsv"), commands.map(row => row.join("\t")).join("\n") + "\n");
    textFile(path.join(directory, "groups.tsv"), groups.map(row => row.join("\t")).join("\n") + "\n");
    for (const row of commands) {
      for (const name of row.slice(7)) cpSync(path.join(rawBank, name), path.join(directory, name));
      if (actualExit) textFile(path.join(directory, row[2] + ".exit"), row[5] + "\n");
    }
    const failed = groups.find(group => group[3] === "fail");
    if (failed) textFile(path.join(directory, "first-failure.tsv"), [...failed.slice(0, 3), commands.find(row => row[0] === failed[0])[2], failed[4], failed[6], failed[7], "20261029"].join("\t") + "\n");
  };
  const originalCommands = allCommands.filter(row => Number(row[0]) <= 3).map(row => [...row]);
  originalCommands.find(row => row[0] === "3")[5] = "65"; originalCommands.find(row => row[0] === "3")[6] = "fail";
  const originalGroups = allGroups.slice(0, 3).map(row => [...row]); originalGroups[2][3] = "fail"; originalGroups[2][4] = "65";
  materialize(priorRoots[0], originalCommands, originalGroups);
  for (const [offset, index] of [8, 10, 12, 13].entries()) {
    const command = [...allCommands.find(row => Number(row[0]) === index)]; command[5] = "65"; command[6] = "fail";
    const group = [...allGroups[index - 1]]; group[3] = "fail"; group[4] = "65";
    materialize(priorRoots[offset + 1], [command], [group]);
  }
  materialize(collection, allCommands.filter(row => Number(row[0]) > 2), allGroups.slice(2), true);
  textFile(path.join(collection, "collector.argv"), ["sh", "-s", "--", path.join(repository, "acceptance"), repository, cli, collection, inputPath, ...priorRoots].join("\n") + "\n");
  for (const suffix of ["stdin", "stdout", "stderr"]) textFile(path.join(collection, "collector." + suffix), "fixture diagnostic process capture\n");
  for (const number of [1, 2]) for (const suffix of ["argv", "stdout", "stderr"])
    textFile(path.join(original, "conductor-logs", `wt-owner-positive.init-${number}.${suffix}`), suffix === "argv" ? [cli, "init", "--json"].join("\n") + "\n" : suffix === "stdout" ? '{"agent_team_bootstrap":{"status":"ready","work_started":false}}\n' : "");
  const oldPlanBytes = readFileSync(oldPlanFile), oldInputBytes = readFileSync(inputPath),
    correctedSnapshot = correctedCohort.web_test;
  textFile(path.join(root, "corrected-wt"), correctedSnapshot + "\n");
  textFile(path.join(repository, "acceptance/run-randomized-commands.sh"), readFileSync(path.join(repository, "acceptance/run-randomized-commands.sh"), "utf8") + "\n# reviewed assertion-only fixture correction\n");
  for (const file of ["randomized-self-test.sh", "randomized-collection-self-test.sh"]) textFile(path.join(repository, "acceptance", file), "#!/bin/sh\nexit 0\n");
  save(path.join(bank, "closeout/final-status.json"), { ...status, head_snapshot_id: correctedSnapshot });
  const originalAccepted = read(path.join(generated.operation, "accepted-input.json"));
  save(path.join(original, "accepted-input.json"), { ...originalAccepted, records_root: original });
  const runtimeRoot = path.join(recoveryBase, "runtime-wt-" + correctedSnapshot.slice(4));
  const runtimeComponents = path.join(runtimeRoot, "web-components.json"), runtimeInput = path.join(runtimeRoot, "web-admission/qualification-input.json");
  save(runtimeComponents, { ...components, web_test: correctedSnapshot });
  const runtimeAccepted = structuredClone(originalAccepted); runtimeAccepted.records_root = runtimeRoot;
  runtimeAccepted.web.acceptance_snapshot = correctedSnapshot; runtimeAccepted.paths.web_components = runtimeComponents;
  save(path.join(runtimeRoot, "accepted-input.json"), runtimeAccepted);
  const runtimeProducer = spawnSync(process.execPath, [path.join(nativeCi, "release_web_admission_init.mjs"),
    "--candidate", path.join(original, "candidate.json"), "--components", runtimeComponents, "--repository", repository,
    "--ait", cli, "--server-bin", server, "--server-url", "http://127.0.0.1:8088/", "--seed", "20261029", "--output", runtimeInput],
    { encoding: "utf8", env: conductorEnv });
  assert.equal(runtimeProducer.status, 0, runtimeProducer.stderr);
  const retainedFork = path.join(recoveryResults, "native.current.reviewed-fork-1"), retainedFinal = path.join(recoveryResults, "closeout-1");
  cpSync(path.join(bank, names[1]), retainedFork, { recursive: true }); cpSync(path.join(bank, names[4]), retainedFinal, { recursive: true });
  save(path.join(retainedFork, "ait.semantic.setup-mutation-task.setup.json"), { repo_name: "ait-web-test", task_id: change.task_id,
    status: "active", identity_source: "local", publication_state: "local_draft",
    worktree: { ...forkWorktree, bound_change_ref: null, fork_snapshot_id: correctedSnapshot, head_snapshot_id: correctedSnapshot } });
  save(path.join(retainedFork, "ait.semantic.config-original.json"), config);
  const recoveryRecipe = structuredClone(isolatedRecipe);
  for (const phase of recoveryRecipe.phases) if (["public_qualification", "component_receipts", "clean_host_qualification"].includes(phase.id)) {
    // Independent external protocol namespaces keep this unqualified fixture's
    // watermark truthful. Real exact-run Q/E rebinding is covered separately
    // by release_stable_prepare_test; no hosted workflow runs here.
    const action = phase.actions[0], namespace = phase.id + "-recovery-fixture";
    action.workflow = namespace; action.dispatch_argv[2] = namespace;
  }
  recoveryRecipe.phases.find(phase => phase.id === "candidate_freeze").actions = [{ id: "freeze-fixture-candidate", type: "command", mutation: false, cwd: root,
    argv: [process.execPath, copy, path.join(original, "candidate.json"), "{{records_root}}/candidate.json"], outputs: ["{{records_root}}/candidate.json"] }];
  const recoveryRecipePath = path.join(root, "recovery-recipe.json"); save(recoveryRecipePath, recoveryRecipe);
  mkdirSync(path.join(root, "unrelated-public-source"));
  const unrelated = initialize(recoveryRecipePath, accepted => {
    accepted.web.acceptance_snapshot = correctedSnapshot; accepted.paths.public_source = path.join(root, "unrelated-public-source");
    accepted.paths.web_components = path.join(accepted.records_root, "web-components.json");
    save(accepted.paths.web_components, { ...components, web_test: correctedSnapshot });
  });
  assert.equal(unrelated.result.status, 0, unrelated.result.stderr);
  assert.deepEqual(read(unrelated.planPath).phases.find(phase => phase.id === "web_admission").actions.map(action => action.id),
    ["bind-web-admission-input", "run-web-admission"], "an unrelated new source must retain normal direct qualification");
  const recovered = initialize(recoveryRecipePath, accepted => {
    accepted.web.acceptance_snapshot = correctedSnapshot;
    accepted.paths.web_components = path.join(accepted.records_root, "web-components.json");
    save(accepted.paths.web_components, { ...components, web_test: correctedSnapshot });
  });
  assert.equal(recovered.result.status, 0, recovered.result.stderr);
  const recoveredPlan = read(recovered.planPath), recoveredWeb = recoveredPlan.phases.find(phase => phase.id === "web_admission").actions;
  assert.deepEqual(recoveredWeb.map(action => action.id), ["bind-web-admission-input", ...names.map((_, index) => `bounded-web-${index + 1}`), "run-web-admission"]);
  for (const action of recoveredWeb.slice(1, 6)) assert(action.adopt && action.probe);
  for (const [index, source] of [[1, retainedInit], [2, retainedFork], [3, retainedDual], [4, collection], [5, retainedFinal]]) {
    assert(recoveredWeb[index].outputs[0].startsWith(recovered.operation + path.sep)); assert(recoveredWeb[index].adopt.argv.includes(source));
  }
  assert(recoveredPlan.source.control_files[path.join(collection, "collector.stdin")]);
  assert(recoveredPlan.source.control_files[path.join(original, "conductor-logs/wt-owner-positive.init-2.stdout")]);
  assert(recoveredPlan.source.control_files[runtimeInput]);
  const recoveredStatePath = path.join(recovered.operation, "conductor-state.json");
  const conductRecovery = () => spawnSync(process.execPath, [path.join(nativeCi, "release_conductor.mjs"), "run", "--plan", recovered.planPath, "--state", recoveredStatePath],
    { encoding: "utf8", env: conductorEnv });
  // Retained formal acceptance still requires the actual Reviewer's native
  // response for this selector path. Exercise the same guarded recovery used
  // above; never rewrite a prior captured expected_identity_path.
  for (const action of sourceActions) {
    const pending = conductRecovery(); assert.equal(pending.status, 75, pending.stderr);
    assert.match(pending.stderr, /Remote acceptance is retained/);
    const pin = path.join(recovered.operation, "conductor-receipts", action.id + ".package-identity.json");
    const finished = spawnSync(conductorEnv.AIT_RELEASE_AIT_BIN, ["workflow", "finish", action.task, "--apply", "--remote", "origin",
      "--expected-identity", pin, "--review-message", "Reviewed files: fixture. Findings: none. Risks: isolated. Tests: protocol. Recommendation: pass.", "--json"],
      { encoding: "utf8", env: { ...conductorEnv, AIT_FIXTURE_REVIEWER: "true" } });
    assert.equal(finished.status, 0, finished.stderr);
    textFile(path.join(recovered.operation, "conductor-logs", action.id + ".ait.finish.log"), finished.stdout);
  }
  const recoveryRun = conductRecovery();
  const inputLog = path.join(recovered.operation, "conductor-logs/bind-web-admission-input.log");
  assert.equal(recoveryRun.status, 65, recoveryRun.stderr + (existsSync(inputLog) ? readFileSync(inputLog, "utf8") : ""));
  const recoveredState = read(recoveredStatePath).phases.find(phase => phase.id === "web_admission");
  const recoveryLog = path.join(recovered.operation, "conductor-logs/bounded-web-4.log");
  assert(recoveredState.actions.slice(0, 6).every(action => action.status === "complete"), recoveryRun.stderr + (existsSync(recoveryLog) ? readFileSync(recoveryLog, "utf8") : ""));
  const admissionLog = readFileSync(path.join(recovered.operation, "conductor-logs/run-web-admission.log"), "utf8");
  assert.match(admissionLog, /semantic-init admission HOLD/);
  assert(!existsSync(path.join(recovered.operation, "web-admission/web-admission.json")));
  assert(readFileSync(oldPlanFile).equals(oldPlanBytes) && readFileSync(inputPath).equals(oldInputBytes));
  assert(!existsSync(path.join(collection, "summary.json")));
  assert.equal(read(path.join(recovered.operation, "web-admission/qualification-input.json")).components.web_test, correctedSnapshot);
  assert.equal(readFileSync(path.join(root, "random-dispatch"), "utf8"), "run\n", "adoption must not rerun the campaign");
  assert.equal(readFileSync(path.join(root, "fork-dispatch"), "utf8"), "fork\n", "adoption must not rerun the reviewed fork");
  const retainedBytes = readFileSync(path.join(retainedFork, "ait.semantic.change.create.preserved.stdout"));
  textFile(path.join(retainedFork, "ait.semantic.change.create.preserved.stdout"), "{}\n");
  const changedSource = conductRecovery(); assert.notEqual(changedSource.status, 0); assert.match(changedSource.stderr, /control file|changed|digest|drift/i);
  writeFileSync(path.join(retainedFork, "ait.semantic.change.create.preserved.stdout"), retainedBytes);
  await runCorrectedCohortFixture({ root, here, native, nativeCi, repository, original,
    inputPath, cli, server, config, status, queue, retainedInit, collection, recoveryRecipe, protocol, protocolState, protocolBin, zip });
} finally {
  if (process.env.AIT_WEB_KEEP_FIXTURE) console.error("retained isolated fixture: " + root);
  else rmSync(root, { recursive: true, force: true });
}
console.log("release cross-stage composition and fail-closed checks passed");
