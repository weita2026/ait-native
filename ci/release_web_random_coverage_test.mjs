import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import os from "node:os";
import { generateCollectionSchedule } from "./release_web_random_collection.mjs";
import { validateRecoveredRandom } from "./release_web_random_coverage.mjs";

// Isolated raw-protocol fixture. The CLI must never be executed by reevaluation.
const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-random-reevaluation-")));
const put = (file, bytes) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, bytes); };
const save = (file, value) => put(file, JSON.stringify(value) + "\n");
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const repository = path.join(root, "repository"), acceptance = path.join(repository, "acceptance"), prior = path.join(root, "prior"), collection = path.join(root, "collection");
const cli = path.join(root, "bin/ait"), inputPath = path.join(root, "input.json"), dispatch = path.join(root, "unexpected-dispatch");
try {
  put(cli, `#!/bin/sh\ntouch '${dispatch}'\nexit 99\n`); chmodSync(cli, 0o755);
  const manifests = path.join(acceptance, "manifests");
  put(path.join(manifests, "ait-command-matrix.tsv"), ["help", "policy show", "patchset show", "task list", "task show", "change list", "change show"].map(command => `${command}\tleaf\tpositive\tpositive\tfixture\tnone\n`).join(""));
  for (const name of ["random-command-exclusions.tsv", "retired-command-paths.tsv", "random-command-scenarios.tsv"]) put(path.join(manifests, name), "# fixture\n");
  put(path.join(acceptance, "generate-random-command-schedule.sh"), `#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do case "$1" in --output) output=$2;; esac; shift 2; done
awk 'BEGIN{for(i=1;i<=3000;i++) { scenario=i==1?"review-evidence-read":i==2?"task-change-read":"leaf-contract"; command=i==3?"change list":i>2?"help":"-";print i "\\t" scenario "\\t" command "\\t1\\t1\\tr20261029-" i}}' >"$output"
printf '%s\\n' '{"contract":"ait-web-test.random-command-schedule.v1","status":"pass","groups":3000,"seed":20261029,"schedule_cksum":"fixture"}'
`);
  const unused = ["json_string", "repository_read_one", "repository_read", "line_snapshot_read", "plan_read", "review_evidence_one", "storage_worktree_one", "storage_worktree_read", "boundary_rejection_one", "boundary_rejection"];
  put(path.join(acceptance, "run-randomized-commands.sh"), `#!/bin/sh
exit 99
awt_collect() {
  if "$@"; then return 0; else awt_rc=$?; fi
  [ "$awt_scenario_exit" -ne 0 ] || awt_scenario_exit=$awt_rc
  return 0
}
awt_leaf_contract() {
  if [ "$1" = 'change list' ]; then
    awt_collect awt_success_text 'change list' "$awt_ait_bin" change list --help
    awt_collect awt_expected_failure 'change list' "$awt_ait_bin" change list "--ait-web-test-invalid-$2"
    return 0
  fi
  awt_collect awt_success_text help "$awt_ait_bin" help
  awt_collect awt_expected_failure help "$awt_ait_bin" help "--ait-web-test-invalid-$2"
}
awt_review_evidence_read() {
  awt_collect awt_success_json 'policy show' '.patchset_id == "RWTT-0004/P-01"' "$awt_ait_bin" policy show RWTT-0004/P-01 --remote origin --json
  awt_collect awt_success_json 'patchset show' '.patchset_id == "RWTT-0004/P-01"' "$awt_ait_bin" patchset show RWTT-0004/P-01 --remote origin --json
}
awt_task_change_read() {
  awt_success_json 'task list' 'type == "array" and length > 0' "$awt_ait_bin" task list --all --remote origin --json || return
  awt_task_id=$(jq -r '.[0].task_id' "$awt_last_stdout")
  awt_success_json 'task show' ".task_id == \\"$awt_task_id\\"" "$awt_ait_bin" task show "$awt_task_id" --remote origin --json || return
  awt_success_json 'change list' 'type == "array"' "$awt_ait_bin" change list --all --remote origin --json || return
  awt_change_id=$(jq -r '.[0].task_id // empty' "$awt_last_stdout")
  [ -n "$awt_change_id" ] || return 0
  awt_success_json 'change show' ".task_id == \\"$awt_change_id\\"" "$awt_ait_bin" change show "$awt_change_id" --remote origin --json
}
awt_execute_scenario() {
  awt_scenario_exit=0
  case "$1" in
    review-evidence-read) awt_collect awt_review_evidence_read ;;
    task-change-read) awt_collect awt_task_change_read ;;
    leaf-contract) awt_collect awt_leaf_contract "$2" "$5" ;;
    *) return 99 ;;
  esac
  return "$awt_scenario_exit"
}
${unused.map(name => `awt_${name}() {\n  return 99\n}`).join("\n")}
`);
  const input = { execution: { random_groups: 3000, random_seed: 20261029 }, executables: { ait: { path: cli, sha256: hash(readFileSync(cli)) } } };
  save(inputPath, input);
  const generated = generateCollectionSchedule(repository, input, path.join(root, "schedule.tsv"));
  for (const directory of [prior, collection]) {
    put(path.join(directory, "schedule.tsv"), generated.bytes); save(path.join(directory, "schedule-summary.json"), generated.metadata);
  }
  const command = (directory, group, order, label, expectation, exit, status, argv, body, actual) => {
    const id = `random.${String(group).padStart(4, "0")}.${String(order).padStart(2, "0")}`;
    put(path.join(directory, id + ".argv"), [cli, ...argv].join("\n") + "\n"); put(path.join(directory, id + ".stdout"), body); put(path.join(directory, id + ".stderr"), "");
    if (actual !== undefined) put(path.join(directory, id + ".exit"), actual + "\n");
    return [group, order, id, label, expectation, exit, status, id + ".argv", id + ".stdout", id + ".stderr"].join("\t");
  };
  const oldCommands = [], oldGroups = [];
  for (let group = 1; group <= 3000; group++) {
    if (group === 1) oldCommands.push(command(prior, group, 1, "policy show", "json-success", 0, "pass", ["policy", "show", "RWTT-0004/P-01", "--remote", "origin", "--json"], '{"patchset_id":"RWTT-0004/P-01"}\n'));
    else if (group === 2) {
      oldCommands.push(command(prior, group, 1, "task list", "json-success", 0, "pass", ["task", "list", "--all", "--remote", "origin", "--json"], '[{"task_id":"RWTT-0001"}]\n'));
      oldCommands.push(command(prior, group, 2, "task show", "json-success", 0, "pass", ["task", "show", "RWTT-0001", "--remote", "origin", "--json"], '{"task_id":"RWTT-0001"}\n'));
    } else if (group === 3) {
      oldCommands.push(command(prior, group, 1, "change list", "text-success", 0, "pass", ["change", "list", "--help"], "List open Changes...\n"));
      oldCommands.push(command(prior, group, 2, "change list", "nonzero-fail-closed", 2, "pass", ["change", "list", `--ait-web-test-invalid-r20261029-${group}`], "invalid option\n"));
    } else {
      oldCommands.push(command(prior, group, 1, "help", "text-success", 0, "pass", ["help"], "help\n"));
      oldCommands.push(command(prior, group, 2, "help", "nonzero-fail-closed", 2, "pass", ["help", `--ait-web-test-invalid-r20261029-${group}`], "rejected\n"));
    }
    oldGroups.push([group, generated.schedule[group - 1][1], generated.schedule[group - 1][2], group <= 2 ? "fail" : "pass", group <= 2 ? 65 : 0, group === 1 ? 1 : 2, "3:3", "3:3"].join("\t"));
  }
  put(path.join(prior, "commands.tsv"), oldCommands.join("\n") + "\n"); put(path.join(prior, "groups.tsv"), oldGroups.join("\n") + "\n");
  put(path.join(prior, "first-failure.tsv"), "1\treview-evidence-read\t-\trandom.0001.01\t65\t3:3\t3:3\t20261029\n");
  const fresh = [command(collection, 1, 2, "patchset show", "json-success", 65, "fail", ["patchset", "show", "RWTT-0004/P-01", "--remote", "origin", "--json"], '{"patchset_id":"RWTT-0004/P-01"}\n', 0),
    command(collection, 2, 3, "change list", "json-success", 65, "fail", ["change", "list", "--all", "--remote", "origin", "--json"], '[]\n', 0)];
  put(path.join(collection, "commands.tsv"), fresh.join("\n") + "\n");
  put(path.join(collection, "groups.tsv"), "1\treview-evidence-read\t-\tfail\t65\t1\t4:4\t4:4\n2\ttask-change-read\t-\tfail\t65\t1\t4:4\t4:4\n");
  put(path.join(collection, "first-failure.tsv"), "1\treview-evidence-read\t-\trandom.0001.02\t65\t4:4\t4:4\t20261029\n");
  put(path.join(collection, "collector.argv"), ["sh", "-s", "--", acceptance, repository, cli, collection, inputPath, prior].join("\n") + "\n");
  for (const suffix of ["stdin", "stdout", "stderr"]) put(path.join(collection, "collector." + suffix), "");
  let pins;
  const files = directory => readdirSync(directory).sort().flatMap(name => {
    const file = path.join(directory, name), stat = lstatSync(file); assert(!stat.isSymbolicLink()); return stat.isDirectory() ? files(file) : [file];
  });
  const pin = () => { pins = new Map([...files(prior), ...files(collection)].map(file => [file, hash(readFileSync(file))])); };
  pin();
  const c = { input, priorInputPath: inputPath, repository, pinned: file => { const bytes = readFileSync(file); assert.equal(hash(bytes), pins.get(file), "changed pinned input"); return bytes; },
    pinnedTree: directory => { const selected = files(directory); assert.deepEqual(selected, [...pins.keys()].filter(file => file.startsWith(directory + path.sep)).sort()); for (const file of selected) c.pinned(file); } };
  const selection = { collection, priorRoots: [prior] }, verify = () => validateRecoveredRandom(c, collection, selection);
  assert.deepEqual(verify(), { groups: 3000, invocations: 6001 });
  assert(!existsSync(dispatch)); assert(!existsSync(path.join(collection, "summary.json")));
  const change = path.join(collection, "random.0002.03.stdout"), changeBytes = readFileSync(change);
  put(change, "{}\n"); assert.throws(verify, /changed pinned input/); put(change, changeBytes);
  const commandsFile = path.join(collection, "commands.tsv"), commandsBytes = readFileSync(commandsFile);
  const nonempty = fresh[1].split("\t"); nonempty[5] = "0"; nonempty[6] = "pass";
  put(commandsFile, [fresh[0], nonempty.join("\t")].join("\n") + "\n");
  put(change, '[{"task_id":"RWTT-0001","status":"draft"}]\n'); pin();
  assert.throws(verify, /nonempty change inventory cannot inherit/);
  put(change, "List open Changes...\n"); pin(); assert.throws(verify, SyntaxError, "JSON inventory text must fail closed");
  const groupsFile = path.join(collection, "groups.tsv"), groupsBytes = readFileSync(groupsFile);
  put(change, '[{"task_id":"RWTT-0001","status":"landed"}]\n');
  const shown = command(collection, 2, 4, "change show", "json-success", 0, "pass",
    ["change", "show", "RWTT-0001", "--remote", "origin", "--json"], '{"task_id":"RWTT-0001"}\n', 0);
  put(commandsFile, [fresh[0], nonempty.join("\t"), shown].join("\n") + "\n");
  put(groupsFile, "1\treview-evidence-read\t-\tfail\t65\t1\t4:4\t4:4\n2\ttask-change-read\t-\tpass\t0\t2\t4:4\t4:4\n"); pin();
  assert.deepEqual(verify(), { groups: 3000, invocations: 6002 });
  for (const suffix of ["argv", "stdout", "stderr", "exit"]) rmSync(path.join(collection, "random.0002.04." + suffix));
  put(groupsFile, groupsBytes);
  put(commandsFile, commandsBytes); put(change, changeBytes); pin();
  const omitted = new Map(["argv", "stdout", "stderr", "exit"].map(suffix => {
    const file = path.join(collection, "random.0002.03." + suffix); return [file, readFileSync(file)];
  }));
  for (const file of omitted.keys()) rmSync(file);
  put(commandsFile, fresh[0] + "\n"); put(path.join(collection, "groups.tsv"), "1\treview-evidence-read\t-\tfail\t65\t1\t4:4\t4:4\n"); pin();
  assert.throws(verify, /missing required invocation/); put(commandsFile, commandsBytes);
  for (const [file, bytes] of omitted) put(file, bytes);
  put(path.join(collection, "groups.tsv"), "1\treview-evidence-read\t-\tfail\t65\t1\t4:4\t4:4\n2\ttask-change-read\t-\tfail\t65\t1\t4:4\t4:4\n");
  const bad = path.join(collection, "random.0001.02.stdout"), good = readFileSync(bad); put(bad, '{"patchset_id":"RWTT-0004/C-01/P-01"}\n'); pin();
  assert.throws(verify, /corrected scenario argv\/output failed/); put(bad, good);
  put(path.join(collection, "summary.json"), '{"status":"pass"}\n'); pin(); assert.throws(verify, /synthesize/); rmSync(path.join(collection, "summary.json"));
  put(path.join(collection, "groups.tsv"), "1\treview-evidence-read\t-\tfail\t65\t1\t4:4\t5:5\n2\ttask-change-read\t-\tfail\t65\t1\t4:4\t4:4\n"); pin(); assert.throws(verify, /changed authority/);
  assert(!existsSync(dispatch));
} finally { rmSync(root, { recursive: true, force: true }); }
console.log("release in-memory random reevaluation checks passed");
