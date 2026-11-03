// A bounded applicability decision for the independently admitted 1.2.0 port.
// Original captures retain their original product and executable identities.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import { digest, receiptBindings, recoverReceiptTransaction } from "./release_receipt_transaction.mjs";

export const previousCohort = Object.freeze({ core: "SNP-612D54B7DED2", node: "SNP-35638846CB28", python: "SNP-E03C958836BD",
  server: "SNP-312CF19CF88F", runner: "SNP-4DCDF133F25D", web_test: "SNP-81FBA5F92668" });
export const correctedCohort = Object.freeze({ ...previousCohort, core: "SNP-87EE9B58D854", node: "SNP-23DCE07316AA",
  python: "SNP-5D36FB73A8CA", web_test: "SNP-C6181C6BE8BB" });
const tasks = { core: "RCT-1939", node: "RNT-0096", python: "RPT-0089" };
const paths = {
  core: ["rust/crates/ait-cli/src/agent_teams/activation.rs", "rust/crates/ait-cli/src/agent_teams/mod.rs",
    "rust/crates/ait-cli/src/binding_output.rs", "rust/crates/ait-cli/src/binding_team.rs", "rust/crates/ait-cli/src/config_surface.rs",
    "rust/crates/ait-cli/tests/team_bootstrap_cli.rs", "rust/crates/ait-cli/tests/team_caller_activation_cli.rs"],
  node: ["ait-external.lock", "ait-external.toml", "ci/run.ps1", "ci/run.sh", "lib/npm-payload-contract.json",
    "release/npm-payload-package.mjs", "release/release-adapter.mjs", "scripts/native-build.mjs", "src/runtime.js", "test/package.test.js"],
  python: ["ait-external.lock", "ait-external.toml", "tests/test_package.py"],
};

export function isApprovedReplacement(selected, prior) {
  return JSON.stringify(Object.keys(selected).sort()) === JSON.stringify(Object.keys(correctedCohort).sort()) &&
    JSON.stringify(Object.keys(prior.components).sort()) === JSON.stringify(Object.keys(previousCohort).sort()) &&
    Object.entries(previousCohort).every(([key, value]) => prior.components[key] === value) &&
    Object.entries(correctedCohort).every(([key, value]) => selected[key] === value);
}

export function validateReplacementDiff(component, diff) {
  assert.equal(diff.old_snapshot_id, previousCohort[component]); assert.equal(diff.new_snapshot_id, correctedCohort[component]);
  assert.deepEqual(diff.files.map(row => row.path).sort(), [...paths[component]].sort(), `unapproved ${component} source delta`);
  assert(diff.files.every(row => row.status === "modified"), "replacement adds, removes or renames source");
  for (const key of ["mode_changed", "added", "deleted", "rename_hints", "directory_move_hints"]) assert.deepEqual(diff[key], []);
  if (component !== "core") for (const row of diff.files) {
    assert.equal(row.diff?.status, "text", "companion literal delta is unavailable");
    const lines = row.diff.text.split("\n"), old = lines.filter(line => line.startsWith("-") && !line.startsWith("---")).map(line => line.slice(1));
    const fresh = lines.filter(line => line.startsWith("+") && !line.startsWith("+++")).map(line => line.slice(1));
    assert(old.length > 0 && old.length === fresh.length, "companion changes more than exact guard literals");
    for (const [index, line] of old.entries()) {
      assert(line.includes(previousCohort.core), "companion changes unrelated source");
      assert.equal(line.replaceAll(previousCohort.core, correctedCohort.core), fresh[index], "companion changes more than Core pin/guard literals");
    }
  }
}

export function replacementIdentity(accepted, prior, selected, reader = null) {
  if (!isApprovedReplacement(selected, prior)) return null;
  assert.equal(accepted.release.version, "1.2.0"); assert.equal(accepted.release.channel, "stable");
  const cli = prior.executables.ait;
  const stat = lstatSync(cli.path); assert(stat.isFile() && !stat.isSymbolicLink());
  assert.equal(createHash("sha256").update(readFileSync(cli.path)).digest("hex"), cli.sha256);
  const read = reader ?? ((root, argv) => {
    const result = spawnSync(cli.path, argv, { cwd: root, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
    assert.equal(result.status, 0, `replacement source delta is unavailable: ${result.stderr}`); return JSON.parse(result.stdout);
  });
  const diffs = {};
  for (const component of ["core", "node", "python"]) {
    const rows = accepted.component_closeouts.filter(row => row.id === component); assert.equal(rows.length, 1);
    const row = rows[0]; assert.equal(row.task, tasks[component]); assert.equal(row.patchset, tasks[component] + "/P-01");
    assert.equal(row.snapshot, correctedCohort[component]); assert.equal(row.remote, "origin");
    if (component === "core") assert.equal(row.repository_root, accepted.paths.core_root);
    diffs[component] = read(row.repository_root, ["snapshot", "diff", previousCohort[component], correctedCohort[component],
      "--include-text", "--max-bytes", "1048576", "--json"]);
    validateReplacementDiff(component, diffs[component]);
  }
  return { components: selected, diffs };
}

// The approved Core port changes bootstrap/preservation and explicit recovery.
// Help and invalid-option calls are parsed by unchanged CLI definitions. Actual
// successful mutations cannot inherit old output just because a label matches.
export function requireUnaffectedRandom(captures) {
  for (const [id, capture] of captures) {
    const args = capture.argv.slice(1), expectation = capture.row[4];
    const affected = affectedCommand(args);
    if (!affected) continue;
    const help = expectation === "text-success" && args.includes("--help");
    const invalid = expectation === "nonzero-fail-closed" && args.some(arg => arg.startsWith("--ait-web-test-invalid-"));
    assert(help || invalid, `fresh affected random closure is required: ${id} (${capture.row[3]})`);
  }
}

export const affectedCommand = args => args[0] === "init" || args[0] === "config" && args[1] === "set" ||
  args[0] === "team" && args[1] === "session" && ["ensure", "ack", "acknowledge", "activate"].includes(args[2]);

const regular = file => {
  const stat = lstatSync(file); assert(stat.isFile() && !stat.isSymbolicLink(), `applicability input is not regular: ${file}`);
  return readFileSync(file);
};
const json = file => JSON.parse(regular(file));
const sha = file => createHash("sha256").update(regular(file)).digest("hex");
export const cacheIds = ["cache-web-source-qualification", "cache-web-candidate-qualification"];
export const cacheDirectory = (root, kind) => path.join(root, "web-" + kind + "-qualification-cache");

// Keep the existing producer descriptors and their qualified validator in
// agreement. These are ordinary command actions, with no extra Plan fields.
export function qualificationCacheActions(native, root, repository) {
  const program = "const [module,...args]=process.argv.slice(1); const {pathToFileURL}=await import('node:url'); const {cacheQualificationArtifact}=await import(pathToFileURL(module)); cacheQualificationArtifact(...args);";
  return ["source", "candidate"].map((kind, index) => ({ id: cacheIds[index], type: "command", mutation: false,
    cwd: native, argv: [process.execPath, "--input-type=module", "-e", program, path.join(native, "ci/release_web_applicability.mjs"),
      kind, root, repository, cacheDirectory(root, kind)], outputs: [cacheDirectory(root, kind)] }));
}

export function qualifiedApplicabilityCaches(accepted, phases) {
  const web = phases.find(phase => phase.id === "web_admission");
  const bounded = Array.isArray(web?.actions) && web.actions.some(action => action.id === "bounded-web-1");
  if (!bounded || !isApprovedReplacement(json(accepted.paths.web_components), { components: previousCohort })) return [];
  return qualificationCacheActions(accepted.paths.native_root, accepted.records_root, accepted.github_repository);
}

export function guardedHostMaterialization(row, native, root, version) {
  const same = isDeepStrictEqual;
  return row?.id === "materialize-host-candidate" && row.type === "command" && row.mutation === true &&
    row.cwd === native && row.probe?.cwd === native && same(row.outputs, [path.join(root, "candidate-bin")]) &&
    same(row.argv, [path.join(native, "ci/release_candidate_materialize.sh"), "materialize", version,
      path.join(root, "candidate-cache"), path.join(root, "candidate-bin")]) &&
    same(row.probe.argv, [path.join(native, "ci/release_candidate_materialize.sh"), "probe", version,
      path.join(root, "candidate-cache"), path.join(root, "candidate-bin")]);
}

export function validateQualifiedCleanHostActions(actions, native, root, version, caches) {
  assert.deepEqual(actions.map(action => action.id),
    ["qualify-frozen-candidate", "cache-frozen-candidate", "materialize-host-candidate", ...caches.map(action => action.id)],
    "qualified clean-host action inventory differs");
  assert(guardedHostMaterialization(actions[2], native, root, version), "qualified clean-host actions must retain guarded host materialization");
  assert.deepEqual(actions.slice(3), caches, "qualified applicability cache binding differs");
}

export function verifyQualificationCache(directory, document, kind, repository) {
  const q = kind === "source" ? document.workflow : document.qualification;
  const runId = kind === "source" ? q.run_id : q.workflow_run_id;
  const artifactId = kind === "source" ? q.artifact_id : kind === "distribution" ? q.candidate_artifact_id : q.aggregate_artifact_id;
  const expected = kind === "source" ? q.artifact_digest : kind === "distribution" ? q.candidate_artifact_digest : q.aggregate_artifact_digest;
  const head = kind === "source" ? document.source.git_commit : document.release.source_commit;
  const cache = json(path.join(directory, "cache.json")), run = json(path.join(directory, "run.json")), artifact = json(path.join(directory, "artifact.json"));
  assert.equal(cache.contract, "ait.release.artifact-cache/v1"); assert.equal(cache.status, "complete");
  assert.equal(cache.repository, repository); assert.equal(cache.workflow_run_id, runId); assert.equal(cache.artifact_id, artifactId);
  assert.equal(run.id, runId); assert.equal(run.head_sha, head); assert.equal(run.status, "completed"); assert.equal(run.conclusion, "success");
  assert.equal(run.run_attempt, kind === "source" ? q.run_attempt : q.workflow_run_attempt);
  assert.equal(artifact.id, artifactId); assert.equal(artifact.expired, false); assert.equal(artifact.digest, expected);
  assert.equal("sha256:" + sha(path.join(directory, "artifact.zip")), expected, "fresh qualification artifact bytes differ");
}

// Generated ordinary command actions use the existing artifact-cache producer.
export function cacheQualificationArtifact(kind, root, repository, output) {
  assert(["source", "candidate"].includes(kind));
  const document = json(path.join(root, kind === "source" ? "pre-rc-qualification.json" : "candidate.json"));
  const runId = kind === "source" ? document.workflow.run_id : document.qualification.workflow_run_id;
  const prefix = kind === "source" ? "ait-pre-rc-qualification-" : "ait-pre-tag-clean-host-";
  const result = spawnSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "release_artifact_cache.sh"),
    [repository, String(runId), prefix, output], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  assert.equal(result.status, 0, `fresh qualification cache failed: ${result.stderr}`);
  verifyQualificationCache(output, document, kind, repository);
}

// Future normal producer outputs are bound by their existing transactions,
// rather than pretending they were pre-existing control_files at Plan creation.
export function currentProducedInputs(c) {
  const state = json(path.join(c.root, "conductor-state.json"));
  const planSha = createHash("sha256").update(c.bytes).digest("hex");
  assert.equal(state.plan_sha256, planSha); assert.deepEqual(state.release, c.plan.release);
  const ids = ["qualify-public-source", "qualify-frozen-candidate", "cache-frozen-candidate", "materialize-host-candidate", ...cacheIds, "bind-web-admission-input"];
  const records = [], pins = {};
  const walk = file => {
    const stat = lstatSync(file); assert(!stat.isSymbolicLink(), "fresh producer output has a symlink");
    if (stat.isDirectory()) for (const name of readdirSync(file).sort()) walk(path.join(file, name));
    else { assert(stat.isFile()); pins[file] = sha(file); }
  };
  for (const id of ids) {
    const actions = c.plan.phases.flatMap(phase => phase.actions).filter(action => action.id === id);
    const completed = state.phases.flatMap(phase => phase.actions).filter(action => action.id === id);
    assert.equal(actions.length, 1); assert.equal(completed.length, 1); const action = actions[0], recorded = completed[0];
    assert.equal(recorded.status, "complete", `fresh candidate producer is incomplete: ${id}`);
    const checkpoint = json(path.join(c.root, "conductor-checkpoints", id + ".json"));
    assert.equal(checkpoint.plan_sha256, planSha); assert.deepEqual(checkpoint.action, recorded);
    if (id === "materialize-host-candidate") {
      const helper = path.join(c.accepted.paths.native_root, "ci/release_candidate_materialize.sh");
      const args = [c.input.release.version, path.join(c.root, "candidate-cache"), path.join(c.root, "candidate-bin")];
      assert.equal(action.mutation, true); assert.deepEqual(action.argv, [helper, "materialize", ...args]);
      assert.deepEqual(action.probe?.argv, [helper, "probe", ...args]);
      assert.deepEqual(action.outputs, [path.join(c.root, "candidate-bin")]);
      for (const [file, expected] of Object.entries(recorded.outputs)) { assert.equal(digest(file), expected); walk(file); }
      const probe = () => { const result = spawnSync(helper, ["probe", ...args], { cwd: action.cwd, encoding: "utf8" });
        assert.equal(result.status, 0, `fresh materialized candidate differs: ${result.stderr}`); };
      probe(); records.push({ id, recorded, probe }); continue;
    }
    const proof = json(path.join(c.root, "conductor-receipts", id + ".json"));
    assert.deepEqual(proof.outputs.map(row => row.path).sort(), Object.keys(recorded.outputs).sort());
    let expectedOutputs = action.bind?.outputs ?? action.outputs;
    if (id === "bind-web-admission-input") {
      const input = path.join(c.root, "web-admission/qualification-input.json"); assert.deepEqual(expectedOutputs, [input]);
      expectedOutputs = [input, ...["starting-config.json", "starting-status.json", "starting-queue.json", "starting-worktrees.json",
        "remote-repository.json", "acceptance-set.sha256"].map(name => path.join(c.root, "web-admission/evidence", name))];
    }
    assert.deepEqual([...expectedOutputs].sort(), Object.keys(recorded.outputs).sort());
    const bindings = receiptBindings(c.root, id); assert(bindings && typeof bindings === "object");
    if (action.type === "github_workflow") {
      assert.equal(bindings["{run_id}"], recorded.run_id); assert.equal(bindings["{head_sha}"], recorded.head_sha);
    }
    const context = { plan_sha256: planSha, release: state.release, action, replacements: bindings };
    for (const [file, expected] of Object.entries(recorded.outputs)) { assert.equal(digest(file), expected, `fresh producer output changed: ${id}`);
      if (id !== "cache-frozen-candidate") walk(file); }
    assert(recoverReceiptTransaction(c.root, id, context)); records.push({ id, context, recorded });
  }
  const pinned = file => { const bytes = regular(file); assert.equal(createHash("sha256").update(bytes).digest("hex"), pins[file]); return bytes; };
  const verify = () => { for (const { id, context, recorded, probe } of records) {
    for (const [file, expected] of Object.entries(recorded.outputs)) assert.equal(digest(file), expected, `fresh producer output changed: ${id}`);
    if (probe) probe(); else assert(recoverReceiptTransaction(c.root, id, context));
  } };
  return { pins, pinned, verify };
}
