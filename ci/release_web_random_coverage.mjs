// Compose complete original captures in memory. Never execute the CLI, rewrite
// a result, emit a full-run PASS summary or maintain a segment ledger.
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, realpathSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { regular } from "./release_web_composition.mjs";
import { generateCollectionSchedule, inspectCollectionInputs, randomDeclarations } from "./release_web_random_collection.mjs";
import { requireUnaffectedRandom } from "./release_web_applicability.mjs";

const rows = file => regular(file).toString().trim().split(/\r?\n/).filter(Boolean).map(line => line.split("\t"));
const idFor = (group, order) => `random.${String(group).padStart(4, "0")}.${String(order).padStart(2, "0")}`;
const digest = /^(0|[1-9][0-9]*):(0|[1-9][0-9]*)$/;
const reevaluated = new Set(["patchset list", "patchset show", "patchset ci-status", "attest show", "change list"]);
const quote = value => "'" + String(value).replaceAll("'", "'\\''") + "'";

export function validateRecoveredRandom(c, directory, selection) {
  const scratch = realpathSync(mkdtempSync(path.join(tmpdir(), "ait-random-coverage-")));
  try {
    for (const root of [...selection.priorRoots, selection.collection]) c.pinnedTree(root);
    const generated = generateCollectionSchedule(c.repository, c.input, path.join(scratch, "schedule.tsv"));
    const original = inspectCollectionInputs(selection.priorRoots, c.input, generated);
    const selected = new Map(original.passed), fresh = rows(path.join(directory, "commands.tsv"));
    assert(!existsSync(path.join(directory, "summary.json")), "recovery must not synthesize a full-run PASS summary");
    assert(regular(path.join(directory, "schedule.tsv")).equals(generated.bytes));
    assert.deepEqual(JSON.parse(regular(path.join(directory, "schedule-summary.json"))), generated.metadata);
    const allowedFiles = new Set(["groups.tsv", "commands.tsv", "schedule.tsv", "schedule-summary.json", "first-failure.tsv"]);
    const byGroup = new Map();
    let previous = [0, 0];
    for (const row of fresh) {
      assert.equal(row.length, 10); const group = Number(row[0]), order = Number(row[1]);
      assert(Number.isSafeInteger(group) && group >= 1 && group <= 3000 && Number.isSafeInteger(order) && order > 0);
      assert.equal(row[2], idFor(group, order)); assert(!selected.has(row[2]), `duplicate recovered invocation: ${row[2]}`);
      assert(group > previous[0] || group === previous[0] && order > previous[1], "recovered invocation order differs"); previous = [group, order];
      assert(["pass", "fail"].includes(row[6])); assert(/^(0|[1-9][0-9]*)$/.test(row[5]));
      for (const [index, suffix] of ["argv", "stdout", "stderr"].entries()) {
        assert.equal(row[index + 7], row[2] + "." + suffix); allowedFiles.add(row[index + 7]); regular(path.join(directory, row[index + 7]));
      }
      const exitFile = row[2] + ".exit"; allowedFiles.add(exitFile);
      const exit = regular(path.join(directory, exitFile)).toString(); assert(/^(0|[1-9][0-9]*)\n$/.test(exit));
      const actualExit = Number(exit.trim()), argv = regular(path.join(directory, row[7])).toString().trimEnd().split("\n");
      assert.equal(argv[0], c.input.executables.ait.path); assert(row[3].split(" ").every((token, index) => argv[index + 1] === token));
      if (row[6] === "pass") assert.equal(Number(row[5]), actualExit);
      else {
        assert(reevaluated.has(row[3]) && actualExit === 0 && row[5] === "65", `unresolved real or unreviewed failure: ${row[2]}`);
        assert.equal(regular(path.join(directory, row[9])).length, 0);
        if (row[3] === "change list") assert.deepEqual(JSON.parse(regular(path.join(directory, row[8]))), []);
      }
      selected.set(row[2], { row, argv, stdout: path.join(directory, row[8]), stderr: path.join(directory, row[9]), actualExit, root: directory });
      byGroup.set(row[0], [...(byGroup.get(row[0]) ?? []), row]);
    }
    const groups = rows(path.join(directory, "groups.tsv"));
    assert.equal(new Set(groups.map(group => group[0])).size, groups.length);
    let previousGroup = 0;
    for (const group of groups) {
      assert.equal(group.length, 8); assert.deepEqual(group.slice(0, 3), generated.schedule[Number(group[0]) - 1]?.slice(0, 3));
      assert(Number(group[0]) > previousGroup); previousGroup = Number(group[0]);
      assert(["pass", "fail"].includes(group[3])); assert.equal(group[3] === "pass", group[4] === "0");
      assert(digest.test(group[6]) && group[6] === group[7], `recovered group ${group[0]} changed authority`);
      const count = (byGroup.get(group[0]) ?? []).length; assert(count > 0); assert.equal(group[5], String(count)); byGroup.delete(group[0]);
    }
    assert.equal(byGroup.size, 0, "recovered invocation lacks its group/state evidence");
    const first = groups.find(group => group[3] === "fail");
    if (first) {
      const failures = rows(path.join(directory, "first-failure.tsv")); assert.equal(failures.length, 1); const failure = failures[0];
      assert.equal(failure.length, 8); assert.deepEqual(failure.slice(0, 3), first.slice(0, 3));
      assert(fresh.some(row => row[0] === first[0] && row[2] === failure[3]));
      assert.deepEqual(failure.slice(4), [first[4], first[6], first[7], "20261029"]);
    } else assert(!existsSync(path.join(directory, "first-failure.tsv")));
    for (const name of readdirSync(directory)) {
      if (name.startsWith("collector.")) assert(["collector.argv", "collector.stdin", "collector.stdout", "collector.stderr"].includes(name));
      else assert(allowedFiles.has(name), `unrecognized recovered case file: ${name}`);
      assert(regular(path.join(directory, name)).equals(c.pinned(path.join(selection.collection, name))), `copied raw differs: ${name}`);
    }
    const invocation = regular(path.join(selection.collection, "collector.argv")).toString().trimEnd().split("\n");
    for (const suffix of ["stdin", "stdout", "stderr"]) c.pinned(path.join(selection.collection, "collector." + suffix));
    assert.equal(invocation[5], c.input.executables.ait.path); assert.equal(invocation[7], c.priorInputPath);
    const matrix = rows(path.join(c.repository, "acceptance/manifests/ait-command-matrix.tsv"));
    const excluded = new Set(rows(path.join(c.repository, "acceptance/manifests/random-command-exclusions.tsv")).filter(row => !row[0].startsWith("#")).map(row => row[0]));
    const permitted = new Set(matrix.filter(row => row[1] === "leaf" && !excluded.has(row[0])).map(row => row[0]));
    const branches = [];
    if (c.applicability) requireUnaffectedRandom(selected);
    for (const [id, capture] of selected) {
      if (capture.row[3] === "change list" && capture.row[4] === "json-success") {
        assert.deepEqual(capture.argv, [c.input.executables.ait.path, "change", "list", "--all", "--remote", "origin", "--json"],
          "conditional Change inventory argv differs");
        const inventory = JSON.parse(regular(capture.stdout));
        assert(Array.isArray(inventory), "conditional change-show requires a valid inventory");
        if (inventory.length) {
          const dependent = selected.get(idFor(Number(capture.row[0]), Number(capture.row[1]) + 1));
          assert.equal(dependent?.row[3], "change show", "nonempty change inventory cannot inherit empty-inventory N/A");
        }
      }
      assert(permitted.has(capture.row[3]), "recovered command is outside the selected matrix");
      for (const command of [...excluded, "repo retire", "repo restore", "task land", "change land"])
        assert(!command.split(" ").every((token, index) => capture.argv[index + 1] === token), "excluded recovered argv");
      const values = [capture.row[3], capture.row[4], capture.actualExit ?? Number(capture.row[5]), path.join(capture.root, capture.row[7]), capture.stdout, capture.stderr];
      branches.push(`${id}) awt_saved_label=${quote(values[0])}; awt_saved_expectation=${quote(values[1])}; awt_saved_exit=${quote(values[2])}; awt_saved_argv=${quote(values[3])}; awt_last_stdout=${quote(values[4])}; awt_saved_stderr=${quote(values[5])} ;;`);
    }
    const declarations = randomDeclarations(path.join(c.repository, "acceptance/run-randomized-commands.sh"));
    const script = `set -eu
awt_ait_bin=$1; awt_schedule=$2; awt_tab=$(printf '\\t')
${declarations}
awt_error() { printf '%s\\n' "$*" >&2; }
awt_lookup() {
  case "$awt_last_invocation_id" in
${branches.join("\n")}
    *) printf 'missing required invocation: %s\\n' "$awt_last_invocation_id" >&2; return 1 ;;
  esac
}
awt_consume() {
  awt_label=$1; awt_expectation=$2; shift 2
  awt_group_invocations=$((awt_group_invocations + 1)); awt_group_ordinal=$((awt_group_ordinal + 1))
  awt_last_invocation_id=$(printf 'random.%04d.%02d' "$awt_current_group" "$awt_group_ordinal")
  awt_lookup || return
  [ "$awt_saved_label" = "$awt_label" ] && [ "$awt_saved_expectation" = "$awt_expectation" ] || return 1
  printf '%s\\n' "$@" | cmp -s - "$awt_saved_argv" || return 1
  printf '%s\\n' "$awt_last_invocation_id"
}
awt_success_json() {
  awt_label=$1; awt_filter=$2; shift 2
  awt_consume "$awt_label" json-success "$@" || return
  [ "$awt_saved_exit" = 0 ] && jq -e "$awt_filter" "$awt_last_stdout" >/dev/null 2>&1
}
awt_success_text() {
  awt_label=$1; shift; awt_consume "$awt_label" text-success "$@" || return
  [ "$awt_saved_exit" = 0 ] && [ -s "$awt_last_stdout" ]
}
awt_expected_failure() {
  awt_label=$1; shift; awt_consume "$awt_label" nonzero-fail-closed "$@" || return
  [ "$awt_saved_exit" -ne 0 ] && { [ -s "$awt_last_stdout" ] || [ -s "$awt_saved_stderr" ]; }
}
while IFS="$awt_tab" read -r awt_current_group awt_scenario awt_candidate awt_a awt_b awt_token; do
  awt_group_invocations=0; awt_group_ordinal=0; awt_first_failed_invocation=
  awt_execute_scenario "$awt_scenario" "$awt_candidate" "$awt_a" "$awt_b" "$awt_token" || exit 72
  printf 'finished\\t%s\\n' "$awt_current_group"
done <"$awt_schedule"
`;
    const checked = spawnSync("sh", ["-s", "--", c.input.executables.ait.path, path.join(scratch, "schedule.tsv")],
      { input: script, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    assert.equal(checked.status, 0, `corrected scenario argv/output failed: ${checked.stderr}`);
    const lines = checked.stdout.trim().split("\n"), finished = lines.filter(line => line.startsWith("finished\t"));
    assert.deepEqual(finished, generated.schedule.map(row => "finished\t" + row[0]));
    const seen = lines.filter(line => !line.startsWith("finished\t"));
    assert.equal(new Set(seen).size, seen.length, "duplicate required invocation");
    assert.deepEqual(new Set(seen), new Set(selected.keys()), "missing or extra recovered invocation");
    original.recheck(); for (const root of [...selection.priorRoots, selection.collection]) c.pinnedTree(root);
    return { groups: finished.length, invocations: seen.length };
  } finally { rmSync(scratch, { recursive: true, force: true }); }
}
