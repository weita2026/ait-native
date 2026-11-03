import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, renameSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import os from "node:os";
import { generateCollectionSchedule, inspectCollectionInputs, runDiagnosticRandomCollection } from "./release_web_random_collection.mjs";

// Isolated CLI protocol. These are source tests, never release evidence.
const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-random-boundary-")));
const put = (file, text) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, text); };
const save = (file, value) => put(file, JSON.stringify(value) + "\n");
const sha = file => createHash("sha256").update(readFileSync(file)).digest("hex");
const repository = path.join(root, "repository"), acceptance = path.join(repository, "acceptance");
const cli = path.join(root, "bin/ait"), inputPath = path.join(root, "input.json"), priorRoot = path.join(root, "prior");
const runner = path.join(acceptance, "run-randomized-commands.sh"), output = path.join(root, "new-captures");
const dispatch = path.join(root, "dispatch");
try {
  put(cli, `#!/bin/sh
printf '%s\\n' "$*" >>'${dispatch}'
case "$*" in
  'help --ait-web-test-invalid-r20261029-1') printf 'rejected\\n' >&2; exit 2 ;;
  'help --ait-web-test-invalid-r20261029-3000') printf 'unexpected success\\n'; exit 0 ;;
  help) exit 0 ;;
  *) exit 99 ;;
esac
`); chmodSync(cli, 0o755);
  put(path.join(acceptance, "lib.sh"), `awt_require_tools() { :; }
awt_validate_repository() { :; }
awt_state_digest() { printf '3:3\\n'; }
awt_error() { printf '%s\\n' "$*" >&2; }
`);
  put(path.join(acceptance, "workflow-controls.jq"), "# fixture\n");
  const unused = ["json_string", "repository_read_one", "repository_read", "line_snapshot_read", "task_change_read", "plan_read",
    "review_evidence_one", "review_evidence_read", "storage_worktree_one", "storage_worktree_read", "boundary_rejection_one", "boundary_rejection"];
  put(runner, `#!/bin/sh
exit 99
awt_preflight() {
  return 0
}
awt_collect() {
  if "$@"; then return 0; else awt_collect_exit=$?; fi
  [ "$awt_scenario_exit" -ne 0 ] || awt_scenario_exit=$awt_collect_exit
  return 0
}
awt_blocked() {
  awt_group_ordinal=$((awt_group_ordinal + $1)); shift
  printf '%s\\n' "$*" >&2
}
awt_leaf_contract() {
  awt_collect awt_success_text help "$awt_ait_bin" help
  awt_collect awt_expected_failure help "$awt_ait_bin" help "--ait-web-test-invalid-$2"
}
awt_execute_scenario() {
  awt_scenario_exit=0
  awt_leaf_contract "$2" "$5"
  return "$awt_scenario_exit"
}
${unused.map(name => `awt_${name}() {\n  return 99\n}`).join("\n")}
`);
  const manifests = path.join(acceptance, "manifests");
  put(path.join(manifests, "ait-command-matrix.tsv"), "help\tleaf\tpositive\tpositive\thelp\tnone\n");
  for (const name of ["random-command-exclusions.tsv", "retired-command-paths.tsv", "random-command-scenarios.tsv"])
    put(path.join(manifests, name), "# fixture\n");
  put(path.join(acceptance, "generate-random-command-schedule.sh"), `#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do case "$1" in --output) output=$2;; esac; shift 2; done
awk 'BEGIN{for(i=1;i<=3000;i++)print i "\\tleaf-contract\\thelp\\t1\\t1\\tr20261029-" i}' >"$output"
printf '%s\\n' '{"contract":"ait-web-test.random-command-schedule.v1","status":"pass","groups":3000,"seed":20261029,"schedule_cksum":"fixture"}'
`);
  const input = { contract: "ait.release.web-admission-input/v1", release: { version: "1.2.0", channel: "stable" },
    web_test: { repository_index: 23, namespace: "WT" }, executables: { ait: { path: cli, sha256: sha(cli) } },
    execution: { random_groups: 3000, random_seed: 20261029 } };
  save(inputPath, input);
  const generated = generateCollectionSchedule(repository, input, path.join(root, "schedule.tsv"));
  mkdirSync(priorRoot); put(path.join(priorRoot, "schedule.tsv"), generated.bytes); save(path.join(priorRoot, "schedule-summary.json"), generated.metadata);
  const groups = [], commands = [];
  for (let group = 1; group < 3000; group++) {
    groups.push([group, "leaf-contract", "help", group <= 2 ? "fail" : "pass", group === 1 ? 66 : group === 2 ? 65 : 0, 2, "3:3", "3:3"].join("\t"));
    for (let order = 1; order <= 2; order++) {
      const id = `random.${String(group).padStart(4, "0")}.${String(order).padStart(2, "0")}`;
      const failed = (group === 1 && order === 2) || (group === 2 && order === 1);
      const argv = [cli, "help", ...(order === 2 ? [`--ait-web-test-invalid-r20261029-${group}`] : [])];
      const row = [group, order, id, "help", order === 1 ? "text-success" : "nonzero-fail-closed",
        failed ? group === 1 ? 66 : 65 : order === 1 ? 0 : 2, failed ? "fail" : "pass", id + ".argv", id + ".stdout", id + ".stderr"];
      commands.push(row.join("\t")); put(path.join(priorRoot, id + ".argv"), argv.join("\n") + "\n");
      put(path.join(priorRoot, id + ".stdout"), failed && order === 1 ? "" : "help or rejection\n"); put(path.join(priorRoot, id + ".stderr"), "");
    }
  }
  put(path.join(priorRoot, "groups.tsv"), groups.join("\n") + "\n"); put(path.join(priorRoot, "commands.tsv"), commands.join("\n") + "\n");
  put(path.join(priorRoot, "first-failure.tsv"), "1\tleaf-contract\thelp\trandom.0001.02\t66\t3:3\t3:3\t20261029\n");
  const selected = inspectCollectionInputs([priorRoot], input, generated);
  assert.equal(selected.passed.size, 5996);
  const result = await runDiagnosticRandomCollection({ repository, inputPath, runnerPath: runner, outputRoot: output, priorRoots: [priorRoot] });
  assert.deepEqual(result, { status: 1, finishedGroups: 3000, newInvocations: 4, reusedInvocations: 5996, failedGroups: 2 });
  assert.deepEqual(readFileSync(dispatch, "utf8").trim().split("\n"), ["help --ait-web-test-invalid-r20261029-1", "help", "help", "help --ait-web-test-invalid-r20261029-3000"]);
  assert(!existsSync(path.join(output, "summary.json"))); assert(!existsSync(path.join(output, "random.0001.01.stdout")));
  assert.equal(readFileSync(path.join(output, "random.0002.01.exit"), "utf8"), "0\n");
  assert.equal(readFileSync(path.join(output, "groups.tsv"), "utf8").trim().split("\n").length, 3);
  selected.recheck();
  const conflict = path.join(root, "conflict"); mkdirSync(conflict);
  put(path.join(conflict, "schedule.tsv"), generated.bytes); save(path.join(conflict, "schedule-summary.json"), generated.metadata);
  put(path.join(conflict, "groups.tsv"), "1\tleaf-contract\thelp\tfail\t66\t2\t3:3\t3:3\n");
  put(path.join(conflict, "commands.tsv"), commands.slice(0, 2).join("\n") + "\n");
  put(path.join(conflict, "first-failure.tsv"), readFileSync(path.join(priorRoot, "first-failure.tsv")));
  for (const order of [1, 2]) for (const suffix of ["argv", "stdout", "stderr"]) {
    const file = `random.0001.0${order}.${suffix}`; put(path.join(conflict, file), readFileSync(path.join(priorRoot, file)));
  }
  put(path.join(conflict, "random.0001.01.stdout"), "different successful help\n");
  assert.throws(() => inspectCollectionInputs([priorRoot, conflict], input, generated), /conflicting prior PASS/);
  const missing = path.join(priorRoot, "random.0001.01.stderr"); renameSync(missing, missing + ".saved");
  assert.throws(() => inspectCollectionInputs([priorRoot], input, generated)); renameSync(missing + ".saved", missing);
  const link = path.join(priorRoot, "link"); symlinkSync(inputPath, link);
  assert.throws(() => inspectCollectionInputs([priorRoot], input, generated), /symlink/); rmSync(link);
  const badArgv = path.join(priorRoot, "random.0001.01.argv"), originalArgv = readFileSync(badArgv);
  put(badArgv, "/another/candidate\nhelp\n");
  assert.throws(() => inspectCollectionInputs([priorRoot], input, generated), /another candidate/); put(badArgv, originalArgv);
  const branch = path.join(root, "incomplete-branch"); mkdirSync(branch);
  const schedule = generated.bytes.toString().replace("1\tleaf-contract\thelp", "1\ttask-change-read\t-");
  put(path.join(branch, "schedule.tsv"), schedule); save(path.join(branch, "schedule-summary.json"), generated.metadata);
  put(path.join(branch, "groups.tsv"), "1\ttask-change-read\t-\tfail\t65\t1\t3:3\t3:3\n");
  put(path.join(branch, "commands.tsv"), "1\t1\trandom.0001.01\ttask list\tjson-success\t0\tpass\trandom.0001.01.argv\trandom.0001.01.stdout\trandom.0001.01.stderr\n");
  put(path.join(branch, "random.0001.01.argv"), cli + "\ntask\nlist\n--all\n--remote\norigin\n--json\n");
  put(path.join(branch, "random.0001.01.stdout"), '[{"task_id":"RWTT-0001"}]\n'); put(path.join(branch, "random.0001.01.stderr"), "");
  put(path.join(branch, "first-failure.tsv"), "1\ttask-change-read\t-\trandom.0001.01\t65\t3:3\t3:3\t20261029\n");
  const different = { ...generated, bytes: Buffer.from(schedule), schedule: generated.schedule.map((row, index) => index ? row : ["1", "task-change-read", "-", ...row.slice(3)]) };
  assert.throws(() => inspectCollectionInputs([branch], input, different), /complete task list -> task show dependency closure/);
  put(path.join(acceptance, "lib.sh"), `awt_require_tools() { :; }
awt_validate_repository() { :; }
awt_error() { printf '%s\\n' "$*" >&2; }
awt_state_digest() {
  if [ -e '${root}/digest-count' ]; then printf '4:4\\n'; else : >'${root}/digest-count'; printf '3:3\\n'; fi
}
`);
  const stoppedRoot = path.join(root, "state-changed");
  const stopped = await runDiagnosticRandomCollection({ repository, inputPath, runnerPath: runner, outputRoot: stoppedRoot, priorRoots: [priorRoot] });
  assert.equal(stopped.status, 74); assert.equal(stopped.finishedGroups, 1); assert.equal(stopped.newInvocations, 1);
  assert.match(readFileSync(path.join(stoppedRoot, "collector.stderr"), "utf8"), /groups 2\.\.3000 are blocked/);
  const changed = path.join(priorRoot, "first-failure.tsv"); put(changed, "changed\n"); assert.throws(selected.recheck, /changed/);
} finally { rmSync(root, { recursive: true, force: true }); }
console.log("release diagnostic random collection boundary checks passed");
