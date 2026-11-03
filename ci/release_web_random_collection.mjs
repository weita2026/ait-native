// Private diagnostic execution boundary. Original captures remain in place.
// This does not produce an admission result or a full-run PASS summary.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { appendFileSync, closeSync, existsSync, lstatSync, mkdirSync, mkdtempSync, openSync,
  readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const read = file => {
  for (let item = file; item !== path.dirname(item); item = path.dirname(item))
    assert(!lstatSync(item).isSymbolicLink(), `collection input has a symlink: ${item}`);
  assert(lstatSync(file).isFile(), `collection input is not regular: ${file}`);
  return readFileSync(file);
};
const rows = file => read(file).toString().trim().split(/\r?\n/).filter(Boolean).map(line => line.split("\t"));
const json = file => JSON.parse(read(file));
const idFor = (group, order) => `random.${String(group).padStart(4, "0")}.${String(order).padStart(2, "0")}`;
const digestPattern = /^(0|[1-9][0-9]*):(0|[1-9][0-9]*)$/;
const scenarioFunctions = ["json_string", "leaf_contract", "repository_read_one", "repository_read", "line_snapshot_read",
  "task_change_read", "plan_read", "review_evidence_one", "review_evidence_read", "storage_worktree_one",
  "storage_worktree_read", "boundary_rejection_one", "boundary_rejection", "execute_scenario"];

export function randomDeclarations(file) {
  const source = read(file).toString(), declarations = [...source.matchAll(/^awt_[a-z_]+\(\) \{\n[\s\S]*?^\}/gm)].map(match => match[0]);
  for (const name of scenarioFunctions)
    assert.equal(declarations.filter(body => body.startsWith(`awt_${name}() {`)).length, 1, `missing or ambiguous scenario: ${name}`);
  return declarations.join("\n");
}

export function generateCollectionSchedule(repository, input, output) {
  assert.equal(input.execution.random_groups, 3000); assert.equal(input.execution.random_seed, 20261029);
  const acceptance = path.join(repository, "acceptance"), manifests = path.join(acceptance, "manifests");
  const generated = spawnSync("sh", [path.join(acceptance, "generate-random-command-schedule.sh"),
    "--matrix", path.join(manifests, "ait-command-matrix.tsv"), "--exclusions", path.join(manifests, "random-command-exclusions.tsv"),
    "--retired", path.join(manifests, "retired-command-paths.tsv"), "--scenarios", path.join(manifests, "random-command-scenarios.tsv"),
    "--groups", "3000", "--seed", "20261029", "--output", output], { encoding: "utf8" });
  assert.equal(generated.status, 0, `collection schedule generation failed: ${generated.stderr}`);
  const metadata = JSON.parse(generated.stdout), schedule = rows(output);
  assert.equal(schedule.length, 3000);
  assert(schedule.every((row, index) => row.length === 6 && row[0] === String(index + 1)));
  return { metadata, schedule, bytes: read(output) };
}

// A complete dependent branch may retain its original state. A partial branch
// cannot borrow a list from one state and its selected-ID reads from another.
function reusablePartial(group, commands) {
  const passed = commands.filter(command => command.row[6] === "pass"), labels = passed.map(command => command.row[3]);
  if (["leaf-contract", "repository-read", "review-evidence-read", "storage-worktree-read", "boundary-rejection"].includes(group[1])) return passed;
  const branches = {
    "line-snapshot-read": [["line list", "line show"], ["snapshot list", "snapshot show", "snapshot ancestry", "snapshot is-ancestor"]],
    "task-change-read": [["task list", "task show"], ["change list", "change show"]],
    "plan-read": [["plan list", "plan show", "plan revisions", "plan items", "plan inspect"]],
  }[group[1]];
  assert(branches, `unknown state dependency: group ${group[0]} ${group[1]}`);
  for (const branch of branches) {
    const observed = labels.filter(label => branch.includes(label));
    assert(observed.length === 0 || JSON.stringify(observed) === JSON.stringify(branch),
      `group ${group[0]} needs the complete ${branch.join(" -> ")} dependency closure; prior PASS branch is incomplete`);
  }
  return passed;
}

export function inspectCollectionInputs(roots, input, generated) {
  assert.equal(new Set(roots).size, roots.length, "duplicate collection root");
  const pins = new Map(), passed = new Map(), original = new Map();
  const pin = file => { const bytes = read(file); pins.set(file, sha(bytes)); return bytes; };
  const inventories = new Map();
  for (const root of roots) {
    assert(path.isAbsolute(root) && path.resolve(root) === root, "collection root is not canonical");
    assert(lstatSync(root).isDirectory() && !lstatSync(root).isSymbolicLink());
    const inventory = readdirSync(root).sort(); inventories.set(root, inventory);
    for (const name of inventory) pin(path.join(root, name));
    assert(pin(path.join(root, "schedule.tsv")).equals(generated.bytes), "prior schedule differs from selected schedule");
    assert.deepEqual(json(path.join(root, "schedule-summary.json")), generated.metadata, "prior schedule identity differs");
    const groups = rows(path.join(root, "groups.tsv")), commands = rows(path.join(root, "commands.tsv"));
    assert.equal(new Set(groups.map(group => group[0])).size, groups.length, "duplicate prior group");
    assert.equal(new Set(commands.map(row => row[2])).size, commands.length, "duplicate prior invocation");
    if (groups.some(group => group[3] === "fail")) {
      const failure = rows(path.join(root, "first-failure.tsv")), first = groups.find(group => group[3] === "fail");
      assert.equal(failure.length, 1); assert.equal(failure[0].length, 8);
      assert.deepEqual(failure[0].slice(0, 3), first.slice(0, 3));
      assert(commands.some(row => row[0] === first[0] && row[2] === failure[0][3]));
      assert.deepEqual(failure[0].slice(4), [first[4], first[6], first[7], "20261029"]);
    } else {
      assert(!existsSync(path.join(root, "first-failure.tsv")), "conflicting PASS summary and failure");
      const summary = json(path.join(root, "summary.json"));
      assert.equal(summary.contract, "ait-web-test.random-command-run.v1"); assert.equal(summary.status, "pass");
      assert.equal(summary.mode, "solo_remote"); assert.equal(summary.repository_index, 23); assert.equal(summary.namespace, "WT");
      assert.equal(summary.seed, 20261029); assert.equal(summary.requested_groups, 3000);
      assert.equal(summary.executed_groups, groups.length); assert.equal(summary.command_invocations, commands.length);
      assert.equal(summary.schedule_cksum, generated.metadata.schedule_cksum);
      if (summary.replay_case === null) assert.equal(groups.length, 3000);
      else { assert.equal(groups.length, 1); assert.equal(String(summary.replay_case), groups[0][0]); }
    }
    const allowedFiles = new Set(["schedule.tsv", "schedule-summary.json", "groups.tsv", "commands.tsv", "first-failure.tsv", "summary.json",
      "scenario-counts.tsv", "scenario-counts.json"]);
    const byGroup = new Map();
    for (const row of commands) {
      assert.equal(row.length, 10); assert(/^[1-9][0-9]*$/.test(row[0]) && /^[1-9][0-9]*$/.test(row[1]));
      assert.equal(row[2], idFor(row[0], row[1])); assert(["pass", "fail"].includes(row[6]));
      assert(/^(0|[1-9][0-9]*)$/.test(row[5]));
      const files = row.slice(7).map((name, index) => {
        assert.equal(name, row[2] + [".argv", ".stdout", ".stderr"][index]); allowedFiles.add(name);
        return path.join(root, name);
      });
      const argv = pin(files[0]).toString().trimEnd().split("\n");
      assert.equal(argv[0], input.executables.ait.path, "prior invocation uses another candidate");
      assert(row[3].split(" ").every((token, index) => argv[index + 1] === token), "prior invocation label contradicts argv");
      assert(!/[A-Z]+T-[0-9]+\/C-[0-9]+/.test(argv.join("\n")), "prior invocation uses an internal Change identity");
      const command = { row, argv, stdout: files[1], stderr: files[2], root };
      byGroup.set(row[0], [...(byGroup.get(row[0]) ?? []), command]);
      original.set(row[2], [...(original.get(row[2]) ?? []), command]);
    }
    assert(inventory.every(name => allowedFiles.has(name)), "prior collection has an unrecognized file inventory");
    for (const group of groups) {
      assert.equal(group.length, 8); const selected = generated.schedule[Number(group[0]) - 1];
      assert(selected); assert.deepEqual(group.slice(0, 3), selected.slice(0, 3));
      assert(digestPattern.test(group[6]) && group[6] === group[7], `prior group ${group[0]} changed authority`);
      assert(["pass", "fail"].includes(group[3])); assert.equal(group[3] === "pass", group[4] === "0");
      const captures = byGroup.get(group[0]) ?? []; assert.equal(String(captures.length), group[5]);
      assert(captures.every((capture, index) => capture.row[1] === String(index + 1)), "prior invocation order has a gap");
      if (group[3] === "pass") assert(captures.every(capture => capture.row[6] === "pass"));
      for (const capture of reusablePartial(group, captures)) {
        const prior = passed.get(capture.row[2]);
        if (prior) assert.deepEqual([prior.row, prior.argv, pin(prior.stdout), pin(prior.stderr)],
          [capture.row, capture.argv, pin(capture.stdout), pin(capture.stderr)], `conflicting prior PASS: ${capture.row[2]}`);
        else passed.set(capture.row[2], capture);
      }
      byGroup.delete(group[0]);
    }
    assert.equal(byGroup.size, 0, "invocation lacks its original group");
  }
  const recheck = () => {
    for (const [root, inventory] of inventories) assert.deepEqual(readdirSync(root).sort(), inventory, "prior collection inventory changed");
    for (const [file, digest] of pins) assert.equal(sha(read(file)), digest, `prior collection changed: ${file}`);
  };
  return { passed, original, pins, recheck };
}

function verdict(expectation, rc, stdout, stderr, filter) {
  if (expectation === "nonzero-fail-closed") return rc !== 0 && (read(stdout).length > 0 || read(stderr).length > 0);
  if (rc !== 0 || read(stdout).length === 0) return false;
  if (expectation === "text-success") return true;
  assert.equal(expectation, "json-success");
  return spawnSync("jq", ["-e", filter, stdout], { stdio: "ignore" }).status === 0;
}

function execute(argv, repository, stdout, stderr) {
  return new Promise((resolve, reject) => {
    const out = openSync(stdout, "wx", 0o600), err = openSync(stderr, "wx", 0o600);
    const child = spawn(argv[0], argv.slice(1), { cwd: repository, env: process.env, stdio: ["ignore", out, err] });
    closeSync(out); closeSync(err);
    child.once("error", reject); child.once("close", code => resolve(code ?? 65));
  });
}

const bridge = `
awt_native_call() {
  awt_invocation_command=$1; awt_invocation_expectation=$2; awt_native_filter=$3; shift 3
  awt_group_invocations=$((awt_group_invocations + 1))
  awt_group_ordinal=$((awt_group_ordinal + 1))
  awt_total_invocations=$((awt_total_invocations + 1))
  awt_last_invocation_id=$(printf 'random.%04d.%02d' "$awt_current_group" "$awt_group_ordinal")
  jq -cn --argjson group "$awt_current_group" --argjson order "$awt_group_ordinal" \\
    --arg label "$awt_invocation_command" --arg expectation "$awt_invocation_expectation" \\
    --arg filter "$awt_native_filter" --args \\
    '{kind:"invoke",group:$group,order:$order,label:$label,expectation:$expectation,filter:$filter,argv:$ARGS.positional}' -- "$@" >&3 || return 70
  IFS="$awt_tab" read -r awt_last_invocation_exit awt_last_stdout awt_last_stderr <&4 || return 70
  return "$awt_last_invocation_exit"
}
awt_success_json() {
  awt_native_label=$1; awt_native_predicate=$2; shift 2
  awt_native_call "$awt_native_label" json-success "$awt_native_predicate" "$@"
}
awt_success_text() {
  awt_native_label=$1; shift
  awt_native_call "$awt_native_label" text-success '' "$@"
}
awt_expected_failure() {
  awt_native_label=$1; shift
  awt_native_call "$awt_native_label" nonzero-fail-closed '' "$@"
}
`;

// Called by the runtime owner with a reviewed collector and a finite explicit
// original inventory. This is an internal JS API, not an AIT command or option.
export async function runDiagnosticRandomCollection({ repository, inputPath, runnerPath, outputRoot, priorRoots }) {
  for (const file of [repository, inputPath, runnerPath, outputRoot])
    assert(path.isAbsolute(file) && path.resolve(file) === file && !/[\t\r\n]/.test(file));
  assert(!existsSync(outputRoot), "collection output exists; retain it and select a fresh owned output");
  const inputBytes = read(inputPath), input = JSON.parse(inputBytes), cli = input.executables.ait.path;
  assert.equal(input.contract, "ait.release.web-admission-input/v1");
  assert.equal(input.web_test.repository_index, 23); assert.equal(input.web_test.namespace, "WT");
  assert.equal(input.release.version, "1.2.0"); assert.equal(input.release.channel, "stable");
  assert.equal(sha(read(cli)), input.executables.ait.sha256, "collection candidate changed");
  const runnerBytes = read(runnerPath), declarations = randomDeclarations(runnerPath);
  for (const name of ["collect", "blocked", "preflight"])
    assert(declarations.includes(`awt_${name}() {`), `reviewed independent collector is missing awt_${name}`);
  const scratch = realpathSync(mkdtempSync(path.join(tmpdir(), "ait-random-collection-")));
  try {
    const generated = generateCollectionSchedule(repository, input, path.join(scratch, "schedule.tsv"));
    const prior = inspectCollectionInputs(priorRoots, input, generated), consumed = new Set();
    const manifests = path.join(repository, "acceptance/manifests");
    const matrix = rows(path.join(manifests, "ait-command-matrix.tsv"));
    const excluded = new Set(rows(path.join(manifests, "random-command-exclusions.tsv")).filter(row => !row[0].startsWith("#")).map(row => row[0]));
    const allowed = new Set(matrix.filter(row => row[1] === "leaf" && !excluded.has(row[0])).map(row => row[0]));
    const sourcePins = new Map([inputPath, cli, runnerPath, fileURLToPath(import.meta.url),
      path.join(path.dirname(runnerPath), "lib.sh"), path.join(path.dirname(runnerPath), "workflow-controls.jq"),
      ...["ait-command-matrix.tsv", "random-command-exclusions.tsv", "retired-command-paths.tsv", "random-command-scenarios.tsv"]
        .map(name => path.join(manifests, name)), path.join(repository, "acceptance/generate-random-command-schedule.sh")]
      .map(file => [file, sha(read(file))]));
    mkdirSync(outputRoot, { recursive: true, mode: 0o700 });
    writeFileSync(path.join(outputRoot, "schedule.tsv"), generated.bytes);
    writeFileSync(path.join(outputRoot, "schedule-summary.json"), JSON.stringify(generated.metadata) + "\n");
    for (const file of ["groups.tsv", "commands.tsv", "collector.stdout", "collector.stderr"]) writeFileSync(path.join(outputRoot, file), "");
    const script = `set -eu
awt_script_dir=$1; awt_repository=$2; awt_ait_bin=$3; awt_output_dir=$4
export AIT_WEB_TEST_RESULTS_DIR="$awt_output_dir" AIT_WEB_TEST_REPOSITORY_ROOT="$awt_repository" AIT_WEB_TEST_AIT_BIN="$awt_ait_bin"
export AIT_WEB_TEST_EXPECTED_MODE=solo_remote
. "$awt_script_dir/lib.sh"
awt_schedule=$awt_output_dir/schedule.tsv
awt_mode=solo_remote; awt_seed=20261029; awt_groups=3000; awt_replay_case=
awt_total_invocations=0; awt_last_stdout=; awt_last_stderr=; awt_last_invocation_id=; awt_last_invocation_exit=0
awt_tab=$(printf '\\t')
${declarations}
${bridge}
cd "$awt_repository"
awt_require_tools jq cksum awk sed
awt_validate_repository "$awt_repository" "$awt_ait_bin" solo_remote
awt_preflight || exit 71
while IFS="$awt_tab" read -r awt_current_group awt_case_scenario awt_case_candidate awt_case_a awt_case_b awt_case_token; do
  awt_group_invocations=0; awt_group_ordinal=0; awt_first_failed_invocation=
  awt_before=$(awt_state_digest) || exit 72
  if awt_execute_scenario "$awt_case_scenario" "$awt_case_candidate" "$awt_case_a" "$awt_case_b" "$awt_case_token"; then awt_rc=0; else awt_rc=$?; fi
  awt_after=$(awt_state_digest) || awt_after=digest-error
  jq -cn --argjson group "$awt_current_group" --argjson exit "$awt_rc" --arg before "$awt_before" --arg after "$awt_after" \\
    '{kind:"finish",group:$group,exit:$exit,before:$before,after:$after}' >&3
  IFS= read -r awt_reply <&4 || exit 73
  [ "$awt_reply" = continue ] || exit 74
done <"$awt_schedule"
awt_preflight || exit 75
`;
    writeFileSync(path.join(outputRoot, "collector.stdin"), script);
    const argv = ["sh", "-s", "--", path.dirname(runnerPath), repository, cli, outputRoot, inputPath, ...priorRoots];
    writeFileSync(path.join(outputRoot, "collector.argv"), argv.join("\n") + "\n");
    let newInvocations = 0, reusedInvocations = 0, finishedGroups = 0, failedGroups = 0;
    const fresh = new Map(), seen = new Set();
    const child = spawn(argv[0], argv.slice(1), { cwd: repository, env: process.env, stdio: ["pipe", "pipe", "pipe", "pipe", "pipe"] });
    const childClosed = new Promise((resolve, reject) => { child.once("error", reject); child.once("close", code => resolve(code)); });
    child.stdout.on("data", bytes => appendFileSync(path.join(outputRoot, "collector.stdout"), bytes));
    child.stderr.on("data", bytes => appendFileSync(path.join(outputRoot, "collector.stderr"), bytes));
    let buffered = "", requests = Promise.resolve(), bridgeError;
    child.stdio[4].on("error", error => { if (!bridgeError) { bridgeError = error; child.kill("SIGTERM"); } });
    const handle = async request => {
      assert(Number.isSafeInteger(request.group) && request.group >= 1 && request.group <= 3000);
      const schedule = generated.schedule[request.group - 1];
      if (request.kind === "finish") {
        assert.equal(request.group, finishedGroups + 1, "collection group order differs"); finishedGroups++;
        const commands = fresh.get(request.group) ?? [];
        assert(digestPattern.test(request.before));
        const stable = request.before === request.after;
        const passed = request.exit === 0 && stable;
        if (!passed) failedGroups++;
        if (commands.length) {
          const group = [...schedule.slice(0, 3), passed ? "pass" : "fail", String(passed ? 0 : request.exit || 67),
            String(commands.length), request.before, request.after];
          appendFileSync(path.join(outputRoot, "groups.tsv"), group.join("\t") + "\n");
          if (!passed && !existsSync(path.join(outputRoot, "first-failure.tsv")))
            writeFileSync(path.join(outputRoot, "first-failure.tsv"), [...schedule.slice(0, 3),
              commands.find(command => command[6] === "fail")?.[2] ?? commands.at(-1)[2], group[4], request.before, request.after, "20261029"].join("\t") + "\n");
        }
        if (!stable) appendFileSync(path.join(outputRoot, "collector.stderr"),
          `authority changed or became unavailable in group ${request.group}; groups ${request.group + 1}..3000 are blocked\n`);
        child.stdio[4].write(stable ? "continue\n" : "blocked-authority\n"); return;
      }
      assert.equal(request.kind, "invoke"); assert.equal(request.group, finishedGroups + 1);
      assert(Number.isSafeInteger(request.order) && request.order > 0);
      const id = idFor(request.group, request.order); assert(!seen.has(id), `duplicate invocation: ${id}`); seen.add(id);
      assert(allowed.has(request.label), `invocation is outside the selected matrix: ${request.label}`);
      assert(Array.isArray(request.argv) && request.argv.every(arg => typeof arg === "string" && !/[\r\n\0]/.test(arg)));
      assert.equal(request.argv[0], cli);
      const starts = command => command.split(" ").every((token, index) => request.argv[index + 1] === token);
      assert(starts(request.label), "collector label contradicts argv");
      for (const command of [...excluded, "task land", "change land", "repo retire", "repo restore"]) assert(!starts(command), "excluded collection argv");
      const old = prior.passed.get(id);
      let stdout, stderr, rc;
      if (old) {
        assert.deepEqual([old.row[3], old.row[4], old.argv], [request.label, request.expectation, request.argv], `prior invocation differs: ${id}`);
        assert(verdict(request.expectation, Number(old.row[5]), old.stdout, old.stderr, request.filter), `original PASS no longer satisfies the preserved predicate: ${id}`);
        stdout = old.stdout; stderr = old.stderr; rc = 0; consumed.add(id); reusedInvocations++;
      } else {
        stdout = path.join(outputRoot, id + ".stdout"); stderr = path.join(outputRoot, id + ".stderr");
        writeFileSync(path.join(outputRoot, id + ".argv"), request.argv.join("\n") + "\n", { flag: "wx" });
        const actual = await execute(request.argv, repository, stdout, stderr);
        writeFileSync(path.join(outputRoot, id + ".exit"), actual + "\n", { flag: "wx" });
        const passed = verdict(request.expectation, actual, stdout, stderr, request.filter);
        rc = passed ? 0 : actual || (request.expectation === "nonzero-fail-closed" ? 66 : 65);
        const row = [String(request.group), String(request.order), id, request.label, request.expectation, String(passed ? actual : rc),
          passed ? "pass" : "fail", id + ".argv", id + ".stdout", id + ".stderr"];
        appendFileSync(path.join(outputRoot, "commands.tsv"), row.join("\t") + "\n");
        fresh.set(request.group, [...(fresh.get(request.group) ?? []), row]); newInvocations++;
      }
      child.stdio[4].write([rc, stdout, stderr].join("\t") + "\n");
    };
    child.stdio[3].setEncoding("utf8");
    child.stdio[3].on("data", chunk => {
      buffered += chunk;
      for (let end; (end = buffered.indexOf("\n")) >= 0;) {
        const line = buffered.slice(0, end); buffered = buffered.slice(end + 1);
        requests = requests.then(() => handle(JSON.parse(line))).catch(error => {
          if (!bridgeError) { bridgeError = error; child.stdio[4].end(); child.kill("SIGTERM"); }
        });
      }
    });
    child.stdin.end(script);
    const status = await childClosed; await requests;
    if (bridgeError) throw bridgeError;
    assert.equal(buffered, "", "collector request was truncated");
    prior.recheck();
    for (const [file, digest] of sourcePins) assert.equal(sha(read(file)), digest, `collector source/input changed: ${file}`);
    assert(read(runnerPath).equals(runnerBytes) && read(inputPath).equals(inputBytes));
    if (status === 0) {
      assert.equal(finishedGroups, 3000, "collector omitted selected groups");
      assert.equal(consumed.size, prior.passed.size, "collector omitted previously valid PASS invocations");
    }
    // Runtime owner inspects ordinary raw results and blocked diagnostics.
    // No direct-run summary or admission/transaction receipt is manufactured.
    return { status: status || (failedGroups ? 1 : 0), finishedGroups, newInvocations, reusedInvocations, failedGroups };
  } finally { rmSync(scratch, { recursive: true, force: true }); }
}
