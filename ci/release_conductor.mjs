#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import { isDeepStrictEqual } from "node:util";
import { runReceiptTransaction, recoverReceiptTransaction, receiptBindings } from "./release_receipt_transaction.mjs";
import { enterOperation } from "./release_operation_owner.mjs";
import { requireProductAcceptance } from "./release_product_acceptance.mjs";
import { runBoundedWebBatch } from "./release_web_bounded.mjs";
import { currentWorkflowReceipt } from "./release_web_workflow.mjs";
import { completedRemoteAdmission, inspectPreparedOperation, readAdmissionJson, qualifiedOperationInputs } from "./release_stable_prepare.mjs";
import { qualifiedApplicabilityCaches, guardedHostMaterialization, validateQualifiedCleanHostActions } from "./release_web_applicability.mjs";

import { originalLocalSnapshot, packageIdentity, samePackage, repreparedPackageIdentity, workflowResult, packageReadiness, packageAcceptance } from "./release_native_workflow.mjs";

const REQUIRED_PHASES = [
  "source_preflight",
  "component_release",
  "candidate_freeze",
  "public_qualification",
  "candidate_admission",
  "component_receipts",
  "clean_host_qualification",
  "web_admission",
  "tag",
  "protected_promotion",
  "endpoint_publication",
  "winget_submission",
  "latest_alias",
  "closeout",
];
const ACTION_TYPES = new Set(["command", "ait_closeout", "github_workflow", "evidence_gate"]);

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail("usage: release_conductor.mjs {run|status} --plan <absolute-json> --state <absolute-json>", 64);
}

function parseCli(argv) {
  const command = argv.shift();
  if (!new Set(["run", "status"]).has(command)) usage();
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) usage();
    options[key] = value;
  }
  if (!options["--plan"] || !options["--state"]) usage();
  return { command, planPath: options["--plan"], statePath: options["--state"] };
}

function requireAbsolute(value, label) {
  if (typeof value !== "string" || !path.isAbsolute(value)) fail(`${label} must be an absolute path`);
  return path.resolve(value);
}

function sha256Bytes(value) {
  return createHash("sha256").update(value).digest("hex");
}

function treeDigest(target) {
  if (!existsSync(target)) fail(`recorded output is missing: ${target}`);
  const root = lstatSync(target);
  if (root.isSymbolicLink()) fail(`recorded output must not be a symlink: ${target}`);
  const hash = createHash("sha256");
  const walk = (current, relative) => {
    const stat = lstatSync(current);
    if (stat.isSymbolicLink()) fail(`recorded output tree contains a symlink: ${current}`);
    if (stat.isDirectory()) {
      hash.update(`d\0${relative}\0`);
      for (const name of readdirSync(current).sort()) walk(path.join(current, name), path.join(relative, name));
      return;
    }
    if (!stat.isFile()) fail(`recorded output tree contains an unsupported entry: ${current}`);
    hash.update(`f\0${relative}\0${stat.mode & 0o777}\0`);
    hash.update(readFileSync(current));
  };
  walk(target, ".");
  return hash.digest("hex");
}

function atomicJson(target, value) {
  const temporary = `${target}.new`;
  writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  renameSync(temporary, target);
}

function sleep(seconds) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, seconds * 1000);
}

function stableAdvance(prior, next) {
  const oldParts = /^(\d+)\.(\d+)\.(\d+)$/.exec(prior);
  const newParts = /^(\d+)\.(\d+)\.(\d+)$/.exec(next);
  if (!oldParts || !newParts) return false;
  const old = oldParts.slice(1).map(Number);
  const current = newParts.slice(1).map(Number);
  return (
    (current[0] === old[0] && current[1] === old[1] && current[2] === old[2] + 1) ||
    (current[0] === old[0] && current[1] === old[1] + 1 && current[2] === 0)
  );
}

function validateArgv(argv, label, retainedWeb = false) {
  const companions = ["--personal-root", "--personal-bin", "--community-root", "--community-bin", "--community-cli-bin", "--chrome"];
  const optionalEmpty = (row, index) => retainedWeb && row === "" && index > 0 && companions.includes(argv[index - 1]);
  const invalidIndex = Array.isArray(argv) ? argv.findIndex((row, index) => typeof row !== "string" || (row.length === 0 && !optionalEmpty(row, index))) : -1;
  if (!Array.isArray(argv) || argv.length === 0 || invalidIndex >= 0) {
    const suffix = invalidIndex >= 0 ? ` at index ${invalidIndex}` : "";
    fail(`${label} argv must be a non-empty string array${suffix}`);
  }
}

function validReviewMessage(value) {
  return typeof value === "string" &&
    /Reviewed files:\s+\S[\s\S]*Findings:\s+\S[\s\S]*Risks:\s+\S[\s\S]*Tests:\s+\S[\s\S]*Recommendation:\s+\S/.test(value);
}

function validateBindings(action, recordsRoot) {
  if (action.bindings === undefined) return;
  if (action.bindings === null || typeof action.bindings !== "object" || Array.isArray(action.bindings)) {
    fail(`${action.id} bindings must be an object`);
  }
  for (const [name, binding] of Object.entries(action.bindings)) {
    if (!/^[a-z][a-z0-9_]*$/.test(name)) fail(`${action.id} binding name is invalid: ${name}`);
    const resolved = requireAbsolute(binding?.path, `${action.id}.bindings.${name}.path`);
    if (!resolved.startsWith(`${recordsRoot}${path.sep}`)) fail(`${action.id} binding must remain below records_root`);
    if (!new Set(["text", "json"]).has(binding?.format)) fail(`${action.id} binding format is invalid: ${name}`);
    if (binding.format === "json" && !/^[a-zA-Z0-9_-]+(?:\.[a-zA-Z0-9_-]+)*$/.test(binding.selector ?? "")) {
      fail(`${action.id} JSON binding selector is invalid: ${name}`);
    }
    if (binding.format === "text" && binding.selector !== undefined) fail(`${action.id} text binding cannot use a selector: ${name}`);
  }
}

function validatePlan(plan, rawPlan, planPath, statePath) {
  if (plan.contract !== "ait.release.conductor-plan/v1") fail("release conductor plan contract is invalid");
  for (const [file, digest] of Object.entries(plan.source?.control_files ?? {})) {
    if (!path.isAbsolute(file) || !existsSync(file) || sha256Bytes(readFileSync(file)) !== digest) fail(`release control source drifted: ${file}`);
  }
  const release = plan.release;
  if (
    release?.channel !== "stable" ||
    !stableAdvance(release.prior_version, release.version) ||
    release.tag !== `v${release.version}`
  ) fail("release conductor currently requires one exact stable patch or minor advance");
  const recordsRoot = requireAbsolute(plan.records_root, "records_root");
  let qualified = null;
  let qualifiedCaches = [];
  let nativeRoot = null;
  const completedSources = plan.phases?.flatMap(phase => phase.actions ?? []).filter(action => action.type === "ait_closeout") ?? [];
  if (plan.source?.control_files?.[path.join(recordsRoot, "preparation-state.json")] ||
      (completedSources.length === 5 && completedSources.every(action => action.finish_local_before_ready === false && action.patchset))) {
    const preparationPath = path.join(recordsRoot, "preparation-state.json");
    const acceptedPath = path.join(recordsRoot, "accepted-input.json");
    try {
      if (!plan.source?.control_files?.[preparationPath] || !plan.source.control_files[acceptedPath]) throw new Error("completed preparation is not pinned by the plan");
      if (completedSources.length !== 5 || !completedSources.every(action => action.finish_local_before_ready === false && action.patchset)) {
        throw new Error("completed source plan inventory differs");
      }
      const accepted = JSON.parse(readFileSync(acceptedPath, "utf8"));
      nativeRoot = accepted.paths.native_root;
      const ait = process.env.AIT_RELEASE_AIT_BIN ?? path.join(accepted.paths.core_root, ".ait/cargo-target/release/ait-cli");
      const proof = inspectPreparedOperation(preparationPath, acceptedPath, (cwd, argv, missing) => readAdmissionJson(ait, cwd, argv, missing));
      qualified = qualifiedOperationInputs(accepted, true);
      if (qualified) qualifiedCaches = qualifiedApplicabilityCaches(accepted, plan.phases);
      if (qualified && !Object.entries(qualified.files).every(([file, hash]) => plan.source.control_files[file] === hash)) {
        throw new Error("qualified inputs are not pinned by the current Plan");
      }
      if (!plan.source.control_files[accepted.paths.web_components] || !completedSources.every(action => proof.closeouts.some(row =>
        ["id", "task", "snapshot", "patchset", "repository_root", "edit_root", "remote", "finish_local_before_ready"].every(key =>
          (key === "id" ? `closeout-${row.id}` : row[key]) === action[key])))) throw new Error("plan source closeouts differ from the prepared cohort");
    } catch (error) { fail(error.message, 75); }
  }
  if (!statePath.startsWith(`${recordsRoot}${path.sep}`)) fail("state must remain below records_root");
  if (rawPlan.match(/[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/)) fail("plan exposes an internal Change reference");
  if (!Array.isArray(plan.phases) || JSON.stringify(plan.phases.map((row) => row.id)) !== JSON.stringify(REQUIRED_PHASES)) {
    fail("release conductor plan must contain the complete ordered phase inventory");
  }
  const actionIds = new Set();
  for (const phase of plan.phases) {
    if (!Array.isArray(phase.actions) || phase.actions.length === 0) fail(`phase has no actions: ${phase.id}`);
    for (const action of phase.actions) {
      if (!/^[a-z0-9][a-z0-9_-]*$/.test(action.id ?? "") || actionIds.has(action.id)) {
        fail(`release action ID is invalid or duplicate: ${action.id ?? ""}`);
      }
      actionIds.add(action.id);
      if (!ACTION_TYPES.has(action.type)) fail(`release action type is invalid: ${action.id}`);
      validateBindings(action, recordsRoot);
      if (action.type === "command") {
        requireAbsolute(action.cwd, `${action.id}.cwd`);
        const retainedWeb = release.version === "1.2.0" && phase.id === "web_admission" &&
          ["bind-web-admission-input", "run-web-admission"].includes(action.id) &&
          (action.id === "bind-web-admission-input" ? ["--personal-root", "--personal-bin"] : ["--personal-bin"])
            .every(flag => action.argv?.filter(value => value === flag).length === 1 &&
            action.argv[action.argv.indexOf(flag) + 1] === "");
        if (retainedWeb) {
          // Only the existing optional companion values may be empty. Keep
          // every command, authority, output and Server argument mandatory.
          const optional = action.id === "bind-web-admission-input"
            ? ["--personal-root", "--personal-bin", "--community-root", "--community-bin", "--community-cli-bin"]
            : ["--personal-bin", "--community-bin", "--community-cli-bin", "--chrome"];
          action.argv.forEach((value, index) => {
            if (value === "" && !optional.includes(action.argv[index - 1])) fail(`${action.id} has an empty required argument`);
          });
        }
        validateArgv(action.argv, `${action.id}`, retainedWeb);
        if (!Array.isArray(action.outputs) || action.outputs.length === 0) fail(`${action.id} must declare output receipts`);
        for (const output of action.outputs) {
          const resolved = requireAbsolute(output, `${action.id}.output`);
          if (!resolved.startsWith(`${recordsRoot}${path.sep}`)) fail(`${action.id} output must remain below records_root`);
        }
        for (const verifier of [action.adopt, action.probe].filter(Boolean)) {
          requireAbsolute(verifier.cwd, `${action.id}.verifier.cwd`);
          validateArgv(verifier.argv, `${action.id}.verifier`);
        }
        if (action.mutation === true) {
          requireAbsolute(action.probe?.cwd, `${action.id}.probe.cwd`);
          validateArgv(action.probe?.argv, `${action.id}.probe`);
        } else if (action.mutation !== false) {
          fail(`${action.id} must explicitly select mutation true or false`);
        }
      } else if (action.type === "ait_closeout") {
        if (!/^[A-Z]+T-[0-9]{4}$/.test(action.task ?? "")) fail(`${action.id} Task ID is invalid`);
        if (!/^SNP-[0-9A-F]{12}$/.test(action.snapshot ?? "")) fail(`${action.id} Snapshot ID is invalid`);
        requireAbsolute(action.repository_root, `${action.id}.repository_root`);
        requireAbsolute(action.edit_root, `${action.id}.edit_root`);
        if (action.patchset !== null && action.patchset !== undefined) {
          const publicPatchset = /^[A-Z]+T-[0-9]{4}\/P-[0-9]{2}$/.test(action.patchset);
          const sameTask = action.patchset.startsWith(`${action.task}/`);
          if (!publicPatchset || (action.finish_local_before_ready !== true && !sameTask)) {
            fail(`${action.id} Patchset reference is invalid`);
          }
        }
        if (!validReviewMessage(action.review_message)) fail(`${action.id} review must contain the complete review sections`);
        if (![undefined, true, false].includes(action.finish_local_before_ready)) fail(`${action.id} local finish flag is invalid`);
      } else if (action.type === "github_workflow") {
        if (!/^[^/]+\/[^/]+$/.test(action.repository ?? "")) fail(`${action.id} GitHub repository is invalid`);
        if (typeof action.workflow !== "string" || action.workflow.length === 0) fail(`${action.id} workflow is invalid`);
        const fixedHead = /^[0-9a-f]{40}$/.test(action.head_sha ?? "");
        const dynamicHead = action.head_sha_path !== undefined;
        if (fixedHead === dynamicHead) fail(`${action.id} requires exactly one fixed or recorded head SHA`);
        if (dynamicHead) {
          const resolved = requireAbsolute(action.head_sha_path, `${action.id}.head_sha_path`);
          if (!resolved.startsWith(`${recordsRoot}${path.sep}`)) fail(`${action.id} head SHA must remain below records_root`);
        }
        requireAbsolute(action.cwd, `${action.id}.cwd`);
        validateArgv(action.dispatch_argv, `${action.id}.dispatch`);
        if (action.approve_environments !== undefined && (
          !Array.isArray(action.approve_environments) || action.approve_environments.length === 0 ||
          action.approve_environments.some((row) => typeof row !== "string" || !/^[a-zA-Z0-9._-]+$/.test(row)) ||
          new Set(action.approve_environments).size !== action.approve_environments.length
        )) fail(`${action.id} protected environment approval inventory is invalid`);
        if (action.bind !== undefined) {
          requireAbsolute(action.bind.cwd, `${action.id}.bind.cwd`);
          validateArgv(action.bind.argv, `${action.id}.bind`);
          if (!Array.isArray(action.bind.outputs) || action.bind.outputs.length === 0) fail(`${action.id} bind outputs are missing`);
          for (const output of action.bind.outputs) {
            const resolved = requireAbsolute(output, `${action.id}.bind.output`);
            if (!resolved.startsWith(`${recordsRoot}${path.sep}`)) {
              fail(`${action.id} bind output must remain below records_root`);
            }
          }
        }
      } else {
        requireAbsolute(action.path, `${action.id}.path`);
        if (typeof action.message !== "string" || action.message.length === 0) fail(`${action.id} gate message is missing`);
      }
    }
  }
  const byPhase = Object.fromEntries(plan.phases.map((row) => [row.id, row.actions]));
  if (!byPhase.component_release.some((row) => row.type === "ait_closeout")) fail("component_release requires AIT closeout actions");
  for (const id of ["public_qualification", "component_receipts", "clean_host_qualification", "protected_promotion", "endpoint_publication", "latest_alias"]) {
    if (qualified && ["public_qualification", "component_receipts", "clean_host_qualification"].includes(id)) {
      const hostMaterialization = row => guardedHostMaterialization(row, nativeRoot, recordsRoot, release.version);
      if (id === "clean_host_qualification") {
        try { validateQualifiedCleanHostActions(byPhase[id], nativeRoot, recordsRoot, release.version, qualifiedCaches); }
        catch (error) { fail(error.message); }
      }
      const expected = { public_qualification: ["qualify-public-source", "bind-qualification", qualified.qualification.workflow.run_id],
        component_receipts: ["collect-component-receipts", "bind-receipts", qualified.receipts.source_dossier.workflow_run_id],
        clean_host_qualification: ["qualify-frozen-candidate", "bind-candidate", qualified.candidate.qualification.workflow_run_id] }[id];
      const action = byPhase[id].find(row => row.id === expected[0]);
      if (!action || action.type !== "command" || action.mutation !== false ||
        path.basename(action.argv[0]) !== "release_operator.sh" || action.argv[1] !== expected[1] ||
        action.argv.filter(arg => arg === "--run-id").length !== 1 ||
        action.argv[action.argv.indexOf("--run-id") + 1] !== String(expected[2]) ||
        (id === "public_qualification" && (action.argv.filter(arg => arg === "--source-root").length !== 1 ||
          action.argv[action.argv.indexOf("--source-root") + 1] !== qualified.qualificationRoot)) ||
        byPhase[id].some(row => row.type === "github_workflow" ||
          (row.mutation === true && !(id === "clean_host_qualification" && hostMaterialization(row))))) fail(`${id} must rebind its exact qualified provider run`);
    } else if (!byPhase[id].some((row) => row.type === "github_workflow")) fail(`${id} requires a GitHub workflow action`);
  }
  for (const id of ["tag", "winget_submission"]) {
    if (!byPhase[id].some((row) => row.type === "command" && row.mutation === true)) fail(`${id} requires a probed mutation action`);
  }
  return { recordsRoot, planSha256: sha256Bytes(rawPlan), planPath, qualified };
}

function initialState(plan, planSha256) {
  return {
    contract: "ait.release.conductor-state/v1",
    plan_sha256: planSha256,
    release: plan.release,
    status: "running",
    phases: plan.phases.map((phase) => ({
      id: phase.id,
      status: "pending",
      actions: phase.actions.map((action) => ({ id: action.id, status: "pending" })),
    })),
  };
}

function loadState(plan, statePath, planSha256, qualified) {
  if (!existsSync(statePath)) {
    const restored = initialState(plan, planSha256);
    for (const phase of restored.phases) for (let index = 0; index < phase.actions.length; index++) {
      const file = path.join(plan.records_root, "conductor-checkpoints", phase.actions[index].id + ".json");
      if (!existsSync(file)) continue;
      const checkpoint = JSON.parse(readFileSync(file, "utf8"));
      if (checkpoint.plan_sha256 !== planSha256 || checkpoint.action.id !== phase.actions[index].id || checkpoint.action.status !== "complete") fail("checkpoint identity drifted");
      phase.actions[index] = checkpoint.action;
      verifyOutputs(checkpoint.action.outputs);
    }
    return restored;
  }
  const state = JSON.parse(readFileSync(statePath, "utf8"));
  if (state.contract !== "ait.release.conductor-state/v1" || state.plan_sha256 !== planSha256) {
    fail("release state does not bind the exact immutable plan");
  }
  if (JSON.stringify(state.release) !== JSON.stringify(plan.release)) fail("release state identity differs from the plan");
  if (qualified) {
    if (!isDeepStrictEqual(state.phases.map(phase => ({ id: phase.id, actions: phase.actions.map(action => action.id) })),
      plan.phases.map(phase => ({ id: phase.id, actions: phase.actions.map(action => action.id) })))) fail("qualified operation state inventory differs");
    for (const phase of state.phases) for (const action of phase.actions) if (action.status === "complete") {
      const file = path.join(plan.records_root, "conductor-checkpoints", `${action.id}.json`);
      if (!existsSync(file)) fail("qualified operation rejects completion without its native checkpoint");
      const checkpoint = JSON.parse(readFileSync(file, "utf8"));
      if (checkpoint.plan_sha256 !== planSha256 || !isDeepStrictEqual(checkpoint.action, action)) fail("qualified operation checkpoint identity drifted");
      verifyOutputs(action.outputs ?? {});
      if (action.digest && treeDigest(action.path) !== action.digest) fail("qualified operation evidence drifted");
    }
  }
  return state;
}

function actionState(state, phaseIndex, actionIndex) {
  return state.phases[phaseIndex].actions[actionIndex];
}

function recordOutputs(action) {
  return Object.fromEntries(action.outputs.map((output) => [output, treeDigest(output)]));
}

function verifyOutputs(outputs) {
  for (const [output, digest] of Object.entries(outputs ?? {})) {
    if (treeDigest(output) !== digest) fail(`completed release receipt drifted: ${output}`);
  }
}

function logPath(recordsRoot, actionId, suffix = "log") {
  return path.join(recordsRoot, "conductor-logs", `${actionId}.${suffix}`);
}

function runProcess(argv, cwd, log, replacements = {}, jsonOutput = false) {
  const expanded = argv.map((value) => Object.entries(replacements).reduce(
    (current, [token, replacement]) => current.replaceAll(token, String(replacement)), value,
  ));
  const result = spawnSync(expanded[0], expanded.slice(1), {
    cwd,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  writeFileSync(log, jsonOutput ? (result.stdout ?? "") : `${result.stdout ?? ""}${result.stderr ?? ""}`, { mode: 0o600 });
  if (jsonOutput && result.stderr) writeFileSync(`${log}.stderr`, result.stderr, { mode: 0o600 });
  return result;
}

function actionReplacements(action) {
  const replacements = {};
  for (const [name, binding] of Object.entries(action.bindings ?? {})) {
    if (!existsSync(binding.path)) fail(`${action.id} binding input is missing: ${binding.path}`);
    const stat = lstatSync(binding.path);
    if (!stat.isFile() || stat.isSymbolicLink()) fail(`${action.id} binding input must be a regular file: ${binding.path}`);
    let value;
    if (binding.format === "text") {
      value = readFileSync(binding.path, "utf8").trim();
    } else {
      let document;
      try { document = JSON.parse(readFileSync(binding.path, "utf8")); } catch { fail(`${action.id} binding input is invalid JSON: ${binding.path}`); }
      value = binding.selector.split(".").reduce((current, part) => current?.[part], document);
      if (!["string", "number", "boolean"].includes(typeof value)) fail(`${action.id} binding value is not scalar: ${name}`);
      value = String(value);
    }
    if (value.length === 0 || value.length > 1024 || value.includes("\0")) fail(`${action.id} binding value is invalid: ${name}`);
    replacements[`{${name}}`] = value;
  }
  return replacements;
}

function pauseAction(current, state, statePath, status, message) {
  current.status = status;
  state.status = status;
  atomicJson(statePath, state);
  fail(message, 75);
}

function receiptContext(action, state, replacements) {
  return { plan_sha256: state.plan_sha256, release: state.release, action, replacements };
}

function completeAction(state, statePath, phaseIndex, actionIndex, extra = {}) {
  const current = actionState(state, phaseIndex, actionIndex);
  Object.assign(current, { status: "complete", ...extra });
  const directory = path.join(path.dirname(statePath), "conductor-checkpoints");
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const file = path.join(directory, current.id + ".json");
  const checkpoint = { plan_sha256: state.plan_sha256, action: current };
  if (!existsSync(file)) atomicJson(file, checkpoint);
  else if (JSON.stringify(JSON.parse(readFileSync(file, "utf8"))) !== JSON.stringify(checkpoint)) fail(`${current.id} checkpoint drifted`);
  atomicJson(statePath, state);
}

function runCommand(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot) {
  const expected = { "qualify-public-source": "pre-rc-qualification.json", "admit-version-delta": "pre-tag-admission.json",
    "prepare-component-receipts": "prepare.json", "collect-component-receipts": "receipts.json", "qualify-frozen-candidate": "candidate.json" }[action.id];
  const verifyQualified = () => {
    if (validated.qualified && expected && (action.outputs.length !== 1 ||
      sha256Bytes(readFileSync(action.outputs[0])) !== validated.qualified.files[path.join(validated.qualified.root, expected)])) {
      fail(`${action.id} differs from the exact qualified evidence`);
    }
    if (validated.qualified && action.id === "cache-frozen-candidate") {
      const cache = action.outputs[0], qualification = validated.qualified.candidate.qualification;
      const record = JSON.parse(readFileSync(path.join(cache, "cache.json"), "utf8"));
      if (record.artifact_id !== qualification.candidate_artifact_id ||
        `sha256:${sha256Bytes(readFileSync(path.join(cache, "artifact.zip")))}` !== qualification.candidate_artifact_digest) fail("qualified candidate artifact changed");
    }
  };
  if (current.status === "complete") {
    verifyOutputs(current.outputs);
    if (validated.qualified && action.mutation === false && !recoverReceiptTransaction(recordsRoot, action.id,
      receiptContext(action, state, actionReplacements(action)))) fail(`${action.id} has no native producer receipt`);
    verifyQualified();
    return;
  }
  const log = logPath(recordsRoot, action.id);
  const replacements = actionReplacements(action);
  const context = receiptContext(action, state, replacements);
  let execution = action;
  if (action.id === "bind-web-admission-input") {
    // The producer emits the input and six immutable entry-authority files.
    // Stage and receipt all seven together; staging only the JSON strands its
    // evidence beside the temporary output and cannot bootstrap a fresh root.
    const input = action.outputs[0];
    if (action.outputs.length !== 1 || action.argv.filter(value => value === "--output").length !== 1 ||
        action.argv[action.argv.indexOf("--output") + 1] !== input) fail("Web input producer/output binding differs");
    const names = ["starting-config.json", "starting-status.json", "starting-queue.json", "starting-worktrees.json",
      "remote-repository.json", "acceptance-set.sha256"];
    const outputs = [input, ...names.map(name => path.join(path.dirname(input), "evidence", name))];
    for (const file of outputs) mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
    const wrapper = `const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const argv=JSON.parse(process.argv[1]),outputs=process.argv.slice(2);argv[argv.indexOf('--output')+1]=outputs[0];
const result=cp.spawnSync(argv[0],argv.slice(1),{stdio:'inherit'});if(result.status!==0)process.exit(result.status??65);
for(let i=1;i<outputs.length;i++)fs.renameSync(path.join(path.dirname(outputs[0]),'evidence',${JSON.stringify(names)}[i-1]),outputs[i]);`;
    execution = { ...action, outputs, argv: [process.execPath, "-e", wrapper, JSON.stringify(action.argv), ...outputs] };
  }
  if (!action.mutation && recoverReceiptTransaction(recordsRoot, action.id, context)) {
    verifyQualified();
    completeAction(state, statePath, phaseIndex, actionIndex, { outputs: recordOutputs(execution) });
    return;
  }
  if (action.mutation) {
    const probeLog = logPath(recordsRoot, action.id, "probe.log");
    const probe = runProcess(action.probe.argv, action.probe.cwd, probeLog, replacements);
    if (![0, 1].includes(probe.status)) fail(`${action.id} probe failed; see ${probeLog}`);
    if (probe.status === 1) {
      current.status = "prepared";
      atomicJson(statePath, state);
      process.stdout.write(`${action.id}: applying once\n`);
      const result = runProcess(action.argv, action.cwd, log, replacements);
      if (result.status === 75) pauseAction(current, state, statePath, "waiting_external", `${action.id}: waiting; see ${log}`);
      if (result.status !== 0) fail(`${action.id} failed; see ${log}`);
      const verified = runProcess(action.probe.argv, action.probe.cwd, probeLog, replacements);
      if (verified.status !== 0) fail(`${action.id} post-mutation probe failed; see ${probeLog}`);
    }
  } else {
    process.stdout.write(`${action.id}: running\n`);
    const result = runReceiptTransaction(recordsRoot, action.id, context, execution, replacements, log);
    if (result.status === 75) pauseAction(current, state, statePath, "waiting_external", `${action.id}: waiting; see ${log}`);
    if (result.status !== 0) fail(`${action.id} failed; see ${log}`);
  }
  verifyQualified();
  completeAction(state, statePath, phaseIndex, actionIndex, { outputs: recordOutputs(execution) });
}

function readJsonCommand(argv, cwd, log, tolerateInvalid = false) {
  const result = runProcess(argv, cwd, log, {}, true);
  try {
    return { result, json: JSON.parse(result.stdout) };
  } catch {
    if (result.status !== 0 || tolerateInvalid) return { result };
    fail(`command returned invalid JSON; see ${log}`);
  }
}

function requireClosedFinishCapture(file, hold, label = "original finish capture") {
  let stat;
  try { stat = lstatSync(file); }
  catch { hold(`${label} is unavailable; retain the selected native identity`); }
  if (!stat.isFile() || stat.isSymbolicLink()) hold(`${label} must be a regular file`);
  const probe = spawnSync("lsof", ["-nP", "-Ffa", "--", file], { encoding: "utf8", timeout: 10000, maxBuffer: 1024 * 1024 });
  if (probe.error || probe.signal || ![0, 1].includes(probe.status) || probe.stderr?.trim()) {
    hold(`cannot verify the ${label} is closed; restore the control host's lsof readback`);
  }
  if (probe.status === 1 && !probe.stdout.trim()) return;
  const accesses = probe.stdout.split("\n").filter(line => line.startsWith("a")).map(line => line.slice(1));
  if (probe.status !== 0 || accesses.length === 0 || accesses.some(access => access !== "r")) {
    hold(`${label} still has a writer or unverifiable access; wait for its native invocation/capture to finish`);
  }
}

function runAitCloseout(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot) {
  const prefix = logPath(recordsRoot, action.id, "ait");
  const finishLog = prefix + ".finish.log";
  const hold = detail => pauseAction(current, state, statePath, "action_required", action.id + ": " + detail);
  const remote = action.remote ?? "origin";
  const ait = action.ait_bin ?? process.env.AIT_RELEASE_AIT_BIN ?? "ait";
  if (action.finish_local_before_ready === false && action.patchset) {
    const receipt = prefix + ".admission-readback.json";
    const patchsetReceipt = prefix + ".patchset-readback.json";
    const retained = current.status === "complete" || current.closeout_status === "complete";
    if (retained) {
      verifyOutputs(current.outputs);
      if (current.outputs?.[receipt] !== treeDigest(receipt) || current.outputs?.[patchsetReceipt] !== treeDigest(patchsetReceipt) || current.closeout_status !== "complete" ||
          current.patchset !== action.patchset || current.expected_identity || current.completion) hold("saved formal source evidence differs");
    }
    const config = readJsonCommand([ait, "config", "show", "--json"], action.repository_root, prefix + ".config.json");
    if (config.result.status !== 0 || !Number.isSafeInteger(config.json?.repository_index) ||
        typeof config.json.repo_name !== "string") hold("native Repository identity is unavailable");
    const observed = readJsonCommand([ait, "task", "show", action.task, "--remote", remote, "--json"],
      action.repository_root, retained ? prefix + ".admission-probe.json" : receipt);
    try {
      if (observed.result.status !== 0) throw new Error("formal source readback is unavailable");
      const proof = completedRemoteAdmission(observed.json, action.task, action.snapshot, config.json.repo_name, config.json.repository_index,
        reference => {
          const resolved = readJsonCommand([ait, "patchset", "show", reference, "--remote", remote, "--json"], action.repository_root,
            retained ? prefix + ".patchset-probe.json" : patchsetReceipt);
          if (resolved.result.status !== 0) throw new Error("public Patchset readback is unavailable");
          return resolved.json;
        });
      if (proof.patchset !== action.patchset) throw new Error("formal source Patchset differs");
      if (retained) {
        const original = JSON.parse(readFileSync(receipt, "utf8"));
        completedRemoteAdmission(original, action.task, action.snapshot, config.json.repo_name, config.json.repository_index,
          () => JSON.parse(readFileSync(patchsetReceipt, "utf8")));
        if (observed.json.current_patchset_id !== original.current_patchset_id ||
            observed.json.successful_land.submission_id !== original.successful_land.submission_id) {
          throw new Error("recorded formal Change/Patchset/Submission identity drifted");
        }
      }
    } catch (error) { hold(error.message); }
    if (current.status !== "complete") completeAction(state, statePath, phaseIndex, actionIndex, {
      patchset: action.patchset, closeout_status: "complete", outputs: { [receipt]: treeDigest(receipt), [patchsetReceipt]: treeDigest(patchsetReceipt) },
    });
    return;
  }
  if (current.status === "complete") {
    requireClosedFinishCapture(finishLog, hold);
    verifyOutputs(current.outputs);
    packageIdentity(current.expected_identity);
    if (!current.completion || current.closeout_status !== "complete") fail(action.id + ": missing original native closeout evidence");
    return;
  }
  const config = readJsonCommand([ait, "config", "show", "--json"], action.repository_root, prefix + ".config.json");
  if (config.result.status !== 0 || !Number.isSafeInteger(config.json?.repository_index)) hold("native Repository identity is unavailable");
  const pinPath = path.join(recordsRoot, "conductor-receipts", action.id + ".package-identity.json");
  if (!current.expected_identity && existsSync(pinPath)) {
    current.expected_identity = packageIdentity(JSON.parse(readFileSync(pinPath, "utf8")));
    atomicJson(statePath, state);
  }
  const select = result => {
    const selected = workflowResult(result, "ready", current.expected_identity ?? null);
    if (selected.repository_index !== config.json.repository_index || selected.revision_snapshot_id !== action.snapshot) {
      hold("ready selected another Repository or original finished Snapshot");
    }
    current.expected_identity = selected;
    mkdirSync(path.dirname(pinPath), { recursive: true, mode: 0o700 });
    if (existsSync(pinPath) && !samePackage(JSON.parse(readFileSync(pinPath, "utf8")), selected)) hold("saved package selector conflicts with operation state");
    if (!existsSync(pinPath)) atomicJson(pinPath, selected);
    atomicJson(statePath, state);
    return selected;
  };
  let ready, readyWasReadbackVerified = false;
  if (!current.expected_identity) {
    if (current.patchset || action.patchset) hold("retain the original formal identity; exact native package recovery is required");
    const taskArgs = [ait, "task", "show", action.task, "--local", "--json"];
    const task = readJsonCommand(taskArgs, action.repository_root, prefix + ".task.json");
    if (task.result.status !== 0) hold("original Local source Task is unavailable");
    if (task.json?.status === "active") hold("independent Reviewer must finish the original Local source Task before package preparation");
    originalLocalSnapshot(task.json, action.task, action.snapshot);
    // Preparation is sent once. A lost response never permits a fresh Job.
    if (current.prepare_attempted) {
      try { ready = { json: JSON.parse(readFileSync(prefix + ".ready.log", "utf8")), result: { status: 0 } }; select(ready.json); }
      catch { hold("prepare outcome is unknown; recover its exact native package identity before resuming"); }
    } else {
      current.prepare_attempted = true;
      current.status = "preparing_package";
      atomicJson(statePath, state);
      ready = readJsonCommand([ait, "workflow", "ready", action.task, "--apply", "--remote", remote, "--json"], action.repository_root, prefix + ".ready.log");
      if (!ready.json) hold("prepare outcome is unknown; retain the original source and read back its exact package/Job");
      select(ready.json);
    }
  }
  // A Server reset never authorizes this controller to prepare another Job.
  // Reconcile only an explicit pinned ready-apply invocation captured in the
  // existing pinned-ready log, with native readback for both old and new pins.
  const captures = [];
  const reprepareLog = prefix + ".ready-pinned.log", browserLog = prefix + ".ready-pinned.stdout";
  if (existsSync(reprepareLog)) {
    requireClosedFinishCapture(reprepareLog, hold, "explicit ready capture");
    try {
      const result = JSON.parse(readFileSync(reprepareLog, "utf8"));
      captures.push({ result, identity: workflowResult(result, "ready", null, { invocation: {
        task: action.task, remote, pinPath, expectedIdentity: current.expected_identity,
      } }) });
    } catch { hold("explicit ready capture is incomplete or incompatible; observe the original authority without another prepare"); }
  }
  if (existsSync(browserLog)) {
    requireClosedFinishCapture(browserLog, hold, "explicit browser ready capture");
    const headers = prefix + ".ready-pinned.headers", confirmation = prefix + ".ready-pinned.confirmation.stdout";
    requireClosedFinishCapture(headers, hold, "explicit browser ready headers");
    requireClosedFinishCapture(confirmation, hold, "explicit browser ready confirmation");
    try {
      const status = /^HTTP (200|202)\n$/.exec(readFileSync(headers, "utf8"));
      if (!status) throw new Error("missing original HTTP status");
      const text = readFileSync(confirmation, "utf8"), start = "Run this exact operation once?\n\n",
        end = "\n\nKeep the original pin for observation and recovery.",
        retainedNotice = "\n\nIf Native verifies a reset, Ready may return a new Job for this same sealed package. New CI and required review apply.";
      if (!text.startsWith(start) || !text.endsWith(end)) throw new Error("missing original confirmation");
      const confirmed = text.slice(start.length, -end.length);
      const request = JSON.parse(confirmed.endsWith(retainedNotice) ? confirmed.slice(0, -retainedNotice.length) : confirmed);
      if (request.operation !== "workflow_ready" || request.task_id !== action.task || request.scope !== "remote" ||
          request.remote !== remote || request.candidate_kind !== "package" || request.apply !== true || !request.expected_identity)
        throw new Error("retained-source ready requires its original operation pin");
      const receipt = currentWorkflowReceipt({ status: Number(status[1]), body: JSON.parse(readFileSync(browserLog, "utf8")) }, request, config.json.repository_index);
      captures.push({ result: receipt.authority, identity: packageIdentity(receipt.authority.expected_identity) });
    } catch { hold("explicit browser ready capture or original HTTP status is incompatible; nothing is retried"); }
  }
  if (captures.length) {
    const original = packageIdentity(current.expected_identity);
    for (const capture of captures) for (const key of ["repository_index", "package_hash", "base_snapshot_id", "revision_snapshot_id"]) {
      if (capture.identity[key] !== original[key]) hold("explicit ready changed the sealed candidate");
    }
    const { identity: selected } = captures.reduce((latest, row) =>
      row.identity.worker_job_index > latest.identity.worker_job_index ? row : latest);
    const saved = existsSync(pinPath) ? packageIdentity(JSON.parse(readFileSync(pinPath, "utf8"))) : null;
    const selectors = [packageIdentity(current.expected_identity), saved].filter(Boolean);
    if (selectors.some(value => !samePackage(value, selected))) {
      if (!saved || current.completion || current.patchset || action.patchset || current.closeout_status) {
        hold("retain known formal acceptance or missing selector evidence; reprepare cannot replace it");
      }
      const previous = selectors.filter(value => !samePackage(value, selected));
      if (previous.length === 2 && !samePackage(previous[0], previous[1])) hold("operation selectors contradict the explicit ready capture");
      try { repreparedPackageIdentity(previous[0], selected); }
      catch { hold("explicit ready changed the sealed candidate or did not select a new Job"); }
      const temporary = mkdtempSync(path.join(tmpdir(), "ait-release-reprepare."));
      let reprepareError, currentReady;
      try {
        const priorPin = path.join(temporary, "prior.json"), nextPin = path.join(temporary, "next.json");
        atomicJson(priorPin, previous[0]); atomicJson(nextPin, selected);
        const prior = readJsonCommand([ait, "workflow", "finish", action.task, "--remote", remote,
          "--expected-identity", priorPin, "--json"], action.repository_root, prefix + ".finish-readback.log", true);
        try {
          workflowResult(prior.json, "finish", previous[0], { readonly: true });
          if (packageAcceptance(prior.json, previous[0], action.task)) throw new Error("original formal acceptance is retained; a new Job cannot replace it");
        } catch (error) { throw new Error("old execution/formal outcome does not permit reprepare: " + error.message); }
        const observed = readJsonCommand([ait, "workflow", "ready", action.task, "--remote", remote,
          "--expected-identity", nextPin, "--json"], action.repository_root, prefix + ".ready-readback.log", true);
        if (observed.result.status !== 0) throw new Error("new ready readback is unavailable; retain the previous selector");
        try { workflowResult(observed.json, "ready", selected, { readonly: true }); }
        catch { throw new Error("new ready capture and native selected execution disagree"); }
        currentReady = observed;
      } catch (error) { reprepareError = error.message; }
      finally { rmSync(temporary, { recursive: true, force: true }); }
      if (reprepareError) hold(reprepareError);
      // A crash between these existing two writes is recovered above only
      // when the same capture agrees with one selector and verifies the other.
      atomicJson(pinPath, selected);
      current.expected_identity = selected;
      current.review_pending = false;
      atomicJson(statePath, state);
      ready = currentReady;
      readyWasReadbackVerified = true;
    }
    // Preserve the original apply response even when it returned pending CI.
    // A resume observes the returned pin; it never overwrites that response
    // with a second ready-apply invocation.
    if (!readyWasReadbackVerified) {
      ready = readJsonCommand([ait, "workflow", "ready", action.task, "--remote", remote,
        "--expected-identity", pinPath, "--json"], action.repository_root, prefix + ".ready-readback.log", true);
      try { workflowResult(ready.json, "ready", selected, { readonly: true }); }
      catch { hold("saved ready capture and native selected execution disagree"); }
      readyWasReadbackVerified = true;
    }
  }
  const identity = packageIdentity(current.expected_identity);
  if (identity.repository_index !== config.json.repository_index || identity.revision_snapshot_id !== action.snapshot) hold("frozen source or Repository identity conflicts");
  if (!existsSync(pinPath)) atomicJson(pinPath, identity);
  if (!samePackage(JSON.parse(readFileSync(pinPath, "utf8")), identity)) hold("package selector drifted");
  const pinned = ["--remote", remote, "--expected-identity", pinPath, "--json"];
  const accept = response => {
    if (!response.json) hold("native outcome is unknown; replay only the same package selector");
    const accepted = packageAcceptance(response.json, identity, action.task);
    if (!accepted) return false;
    current.review_pending = true;
    current.completion = accepted.completion;
    current.patchset = accepted.completion.patchset_id;
    current.closeout_status = accepted.complete ? "complete" : "partial";
    atomicJson(statePath, state);
    if (!accepted.complete || response.result.status !== 0) hold("Remote acceptance is retained; native Local mappings/closeout remain pending");
    requireClosedFinishCapture(finishLog, hold);
    completeAction(state, statePath, phaseIndex, actionIndex, {
      expected_identity: identity, completion: accepted.completion, closeout_status: "complete",
      patchset: accepted.completion.patchset_id, remote_task_id: accepted.remote_task_id,
      outputs: { [finishLog]: treeDigest(finishLog), [pinPath]: treeDigest(pinPath) },
    });
    process.stdout.write(accepted.completion.patchset_id + ": exact package closeout complete\n");
    return true;
  };
  // Reuse the actual Reviewer's original native output in place. Formal GET
  // alone intentionally cannot reconstruct full original source membership.
  let recorded;
  if (existsSync(finishLog)) {
    requireClosedFinishCapture(finishLog, hold);
    try {
      const json = JSON.parse(readFileSync(finishLog, "utf8"));
      workflowResult(json, "finish", identity, { invocation: {
        task: action.task, remote, pinPath, expectedIdentity: identity,
      } });
      if (packageAcceptance(json, identity, action.task)?.complete) recorded = json;
    } catch { /* A failed/partial command is not a successful original result. */ }
    if (!recorded) hold("original finish capture is partial, truncated or mismatched; only the actual Reviewer may recover the same pin into " + finishLog);
  }
  if (recorded) {
    const observed = readJsonCommand([ait, "workflow", "finish", action.task, ...pinned], action.repository_root, prefix + ".finish-readback.log");
    if (!observed.json) hold("retain the original finish output; exact formal readback is unavailable");
    workflowResult(observed.json, "finish", identity, { readonly: true });
    const accepted = packageAcceptance(observed.json, identity, action.task);
    if (!accepted || accepted.completion.patchset_id !== recorded.completion.patchset_id ||
        accepted.completion.submission_id !== recorded.completion.submission_id) {
      hold("original finish output and current exact formal acceptance do not agree");
    }
    accept({ json: recorded, result: { status: 0 } });
    return;
  }
  if (!current.review_pending) {
    ready ??= readJsonCommand([ait, "workflow", "ready", action.task, "--apply", ...pinned], action.repository_root, prefix + ".ready-pinned.log");
    if (!readyWasReadbackVerified) {
      let returned;
      try { returned = workflowResult(ready.json, "ready"); }
      catch { hold("pinned ready outcome is incomplete or incompatible; retain its original pin/capture"); }
      if (!samePackage(returned, identity)) {
        try { repreparedPackageIdentity(identity, returned); }
        catch { hold("pinned ready changed the original sealed candidate"); }
        // The existing explicit ready-apply call returned a retained-source
        // replacement. Reconcile its closed original output before continuing.
        return runAitCloseout(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot);
      }
    }
    const alreadyAccepted = packageAcceptance(ready.json, identity, action.task);
    if (alreadyAccepted) {
      current.completion = alreadyAccepted.completion;
      current.review_pending = true;
      atomicJson(statePath, state);
      hold("Remote acceptance is retained; recover the actual Reviewer's original finish output in " + finishLog);
    }
    const execution = ready.json.package_execution;
    const verifiedPassed = readyWasReadbackVerified && ready.result.status === 0 && !ready.json.completion &&
      execution?.ci_state === "passed" && execution.availability === "available" && execution.recoverable === true;
    if (!verifiedPassed && (ready.result.status !== 0 || !packageReadiness(ready.json))) {
      pauseAction(current, state, statePath, "waiting_external", action.id + ": native " + ready.json.next_action.code + "; retain the same package/Job");
    }
  }
  current.review_pending = true;
  atomicJson(statePath, state);
  hold("independent Reviewer must finish the original source with the saved package selector and capture its native JSON in " + finishLog);
}

function ghRuns(action, recordsRoot) {
  const log = logPath(recordsRoot, action.id, "runs.json");
  const result = readJsonCommand([
    "gh", "run", "list", "--repo", action.repository, "--workflow", action.workflow,
    "--event", "workflow_dispatch", "--limit", "50",
    "--json", "databaseId,headSha,status,conclusion,createdAt",
  ], action.cwd, log);
  if (result.result.status !== 0 || !Array.isArray(result.json)) fail(`${action.id} could not list workflow runs; see ${log}`);
  return result.json;
}

function githubHeadSha(action) {
  if (action.head_sha) return action.head_sha;
  if (!existsSync(action.head_sha_path)) fail(`${action.id} recorded head SHA is missing: ${action.head_sha_path}`);
  const stat = lstatSync(action.head_sha_path);
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`${action.id} recorded head SHA must be a regular file`);
  const value = readFileSync(action.head_sha_path, "utf8").trim();
  if (!/^[0-9a-f]{40}$/.test(value)) fail(`${action.id} recorded head SHA is invalid`);
  return value;
}

function approvePendingDeployments(action, runId, recordsRoot, current, state, statePath) {
  if (action.approve_environments === undefined) return;
  const endpoint = `repos/${action.repository}/actions/runs/${runId}/pending_deployments`;
  const pendingLog = logPath(recordsRoot, action.id, "pending-deployments.json");
  const pending = readJsonCommand(["gh", "api", endpoint], action.cwd, pendingLog);
  if (pending.result.status !== 0 || !Array.isArray(pending.json)) {
    fail(`${action.id} protected environment read failed; see ${pendingLog}`);
  }
  const expected = new Set(action.approve_environments);
  const unexpected = pending.json.filter((row) => !expected.has(row.environment?.name));
  if (unexpected.length > 0) fail(`${action.id} found an unexpected protected environment`);
  const environmentIds = pending.json.map((row) => row.environment?.id);
  if (environmentIds.length === 0) return;
  if (environmentIds.some((row) => !Number.isSafeInteger(row) || row <= 0)) {
    fail(`${action.id} protected environment identity is invalid`);
  }
  if (action.auto_approve !== true || pending.json.some((row) => row.current_user_can_approve === false)) {
    current.pending_environments = pending.json.map((row) => row.environment.name);
    pauseAction(current, state, statePath, "waiting_approval", `${action.id}: waiting for protected environment approval`);
  }
  const approvalLog = logPath(recordsRoot, action.id, "environment-approval.log");
  const approved = spawnSync("gh", ["api", "--method", "POST", endpoint, "--input", "-"], {
    cwd: action.cwd,
    encoding: "utf8",
    input: `${JSON.stringify({ environment_ids: environmentIds, state: "approved", comment: "Approved by the bound AIT stable release conductor." })}\n`,
    maxBuffer: 16 * 1024 * 1024,
  });
  writeFileSync(approvalLog, `${approved.stdout ?? ""}${approved.stderr ?? ""}`, { mode: 0o600 });
  if (approved.status !== 0) pauseAction(current, state, statePath, "waiting_approval", `${action.id}: approval unavailable; see ${approvalLog}`);
  process.stdout.write(`${action.id}: protected environment approved\n`);
}

function runGithubWorkflow(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot) {
  if (current.status === "complete") {
    verifyOutputs(current.outputs);
    return;
  }
  const headSha = githubHeadSha(action);
  const replacements = { ...actionReplacements(action), "{head_sha}": headSha };
  if (current.head_sha && current.head_sha !== headSha) fail(`${action.id} recorded workflow head SHA drifted`);
  const recoveredBindings = receiptBindings(recordsRoot, action.id);
  if (action.bind && recoveredBindings) {
    const recoveredRun = recoveredBindings["{run_id}"];
    if (!Number.isSafeInteger(recoveredRun) || recoveredRun <= 0) fail(`${action.id}: recovered run identity is invalid`);
    const bindings = { ...replacements, "{run_id}": recoveredRun };
    if (recoverReceiptTransaction(recordsRoot, action.id, receiptContext(action, state, bindings))) {
      completeAction(state, statePath, phaseIndex, actionIndex, { run_id: recoveredRun, head_sha: headSha, outputs: recordOutputs({ outputs: action.bind.outputs }) });
      return;
    }
  }
  let runId = current.run_id ?? null;
  if (!runId) {
    const runs = ghRuns(action, recordsRoot);
    const watermark = current.watermark ?? Math.max(0, ...runs.map((row) => row.databaseId));
    if (current.status === "pending") {
      Object.assign(current, { status: "prepared", watermark, head_sha: headSha });
      atomicJson(statePath, state);
      const dispatchLog = logPath(recordsRoot, action.id, "dispatch.log");
      const dispatched = runProcess(action.dispatch_argv, action.cwd, dispatchLog, replacements);
      if (dispatched.status !== 0) process.stdout.write(`${action.id}: dispatch acknowledgement uncertain; reconciling the same head/watermark\n`);
    }
    for (let attempt = 1; attempt <= 60; attempt += 1) {
      const matches = ghRuns(action, recordsRoot).filter(
        (row) => row.databaseId > watermark && row.headSha === headSha,
      );
      if (matches.length > 1) fail(`${action.id} workflow dispatch identity is ambiguous`);
      if (matches.length === 1) {
        runId = matches[0].databaseId;
        Object.assign(current, { status: "dispatched", run_id: runId, watermark, head_sha: headSha });
        atomicJson(statePath, state);
        process.stdout.write(`${action.id}: workflow run ${runId}\n`);
        break;
      }
      sleep(2);
    }
    if (!runId) pauseAction(current, state, statePath, "waiting_external", `${action.id}: dispatch outcome unknown; resolve identity before retry`);
  }
  const viewLog = logPath(recordsRoot, action.id, "view.json");
  for (let attempt = 1; attempt <= (action.max_wait_minutes ?? 180) * 4; attempt += 1) {
    approvePendingDeployments(action, runId, recordsRoot, current, state, statePath);
    const view = readJsonCommand(
      ["gh", "run", "view", String(runId), "--repo", action.repository, "--json", "status,conclusion,url,attempt,headSha"],
      action.cwd,
      viewLog,
    );
    if (view.result.status !== 0) fail(`${action.id} workflow status failed; see ${viewLog}`);
    if (view.json.headSha && view.json.headSha !== headSha) fail(`${action.id} workflow head drifted`);
    if (view.json.status === "completed") {
      if (view.json.conclusion !== "success") {
        if (!["failure", "timed_out", "cancelled"].includes(view.json.conclusion)) fail(`${action.id} non-retryable workflow conclusion: ${view.json.conclusion}`);
        const failedAttempt = view.json.attempt ?? 1;
        if (current.retry_attempt === failedAttempt) pauseAction(current, state, statePath, "waiting_external", `${action.id}: rerun acknowledgement pending`);
        if ((current.retry_count ?? 0) >= (action.max_retries ?? 2)) fail(`${action.id} workflow retry budget exhausted`);
        Object.assign(current, { retry_attempt: failedAttempt, retry_count: (current.retry_count ?? 0) + 1, status: "retry_prepared" });
        atomicJson(statePath, state);
        const retried = runProcess(["gh", "run", "rerun", String(runId), "--repo", action.repository, "--failed"], action.cwd, logPath(recordsRoot, action.id, "retry.log"));
        pauseAction(current, state, statePath, "waiting_external", `${action.id} workflow run ${runId} concluded ${view.json.conclusion}; failed-job retry ${retried.status === 0 ? "requested" : "requires readback"}`);
      }
      break;
    }
    if (attempt % 4 === 0) process.stdout.write(`${action.id}: workflow run ${runId} is ${view.json.status}\n`);
    if (attempt === (action.max_wait_minutes ?? 180) * 4) {
      pauseAction(current, state, statePath, "waiting_external", `${action.id}: workflow run ${runId} still running; resume later`);
    }
    sleep(15);
  }
  let outputs = {};
  if (action.bind) {
    const bindLog = logPath(recordsRoot, action.id, "bind.log");
    const bindings = { ...replacements, "{run_id}": runId };
    const context = receiptContext(action, state, bindings);
    const bound = runReceiptTransaction(recordsRoot, action.id, context, action.bind, bindings, bindLog);
    if (bound.status !== 0) fail(`${action.id} binding failed; see ${bindLog}`);
    outputs = recordOutputs({ outputs: action.bind.outputs });
  }
  completeAction(state, statePath, phaseIndex, actionIndex, { run_id: runId, head_sha: headSha, outputs });
}

function runEvidenceGate(action, current, state, statePath, phaseIndex, actionIndex) {
  if (!existsSync(action.path)) {
    current.status = "action_required";
    atomicJson(statePath, state);
    fail(`${action.id}: ${action.message}`, 75);
  }
  const digest = treeDigest(action.path);
  if (current.status === "complete" && current.digest !== digest) fail(`${action.id} evidence drifted`);
  completeAction(state, statePath, phaseIndex, actionIndex, { path: action.path, digest });
}

function printStatus(state) {
  for (const phase of state.phases) {
    const complete = phase.actions.filter((row) => row.status === "complete").length;
    process.stdout.write(`${phase.id}: ${phase.status} (${complete}/${phase.actions.length})\n`);
  }
  process.stdout.write(`release: ${state.release.tag} ${state.status}\n`);
}

const cli = parseCli(process.argv.slice(2));
const planPath = requireAbsolute(cli.planPath, "plan");
const statePath = requireAbsolute(cli.statePath, "state");
await enterOperation(JSON.parse(readFileSync(planPath, "utf8")).records_root);
const rawPlan = readFileSync(planPath, "utf8");
let plan;
try { plan = JSON.parse(rawPlan); } catch { fail("release conductor plan is invalid JSON"); }
const validated = validatePlan(plan, rawPlan, planPath, statePath);
mkdirSync(validated.recordsRoot, { recursive: true });
mkdirSync(path.join(validated.recordsRoot, "conductor-logs"), { recursive: true, mode: 0o700 });
const state = loadState(plan, statePath, validated.planSha256, validated.qualified);
if (cli.command === "run" && !existsSync(statePath)) atomicJson(statePath, state);

if (cli.command === "status") {
  printStatus(state);
  process.exit(0);
}

state.status = "running";
for (let phaseIndex = 0; phaseIndex < plan.phases.length; phaseIndex += 1) {
  const phase = plan.phases[phaseIndex];
  const phaseState = state.phases[phaseIndex];
  if (phase.id === "tag") {
    try { requireProductAcceptance(state.release.version); }
    catch (error) {
      phaseState.status = "action_required";
      for (const action of phaseState.actions) action.status = "action_required";
      state.status = "action_required";
      atomicJson(statePath, state);
      fail(error.message, 75);
    }
  }
  if (phaseState.status === "complete") {
    for (const [actionIndex, action] of phaseState.actions.entries()) {
      if (phase.actions[actionIndex].type === "ait_closeout") {
        runAitCloseout(phase.actions[actionIndex], action, state, statePath, phaseIndex, actionIndex, validated.recordsRoot);
      }
      verifyOutputs(action.outputs);
      if (action.path && treeDigest(action.path) !== action.digest) fail(`${action.id} evidence drifted`);
    }
    continue;
  }
  const boundedOffset = phase.actions[0]?.id === "bind-web-admission-input" ? 1 : 0;
  if (phase.id === "web_admission" && phase.actions.slice(boundedOffset, boundedOffset + 5).length === 5 &&
    phase.actions.slice(boundedOffset, boundedOffset + 5).every((action, index) => action.id === `bounded-web-${index + 1}`)) {
    if (boundedOffset) runCommand(phase.actions[0], phaseState.actions[0], state, statePath, phaseIndex, 0, validated.recordsRoot);
    const outcomes = await runBoundedWebBatch(validated.recordsRoot, state.plan_sha256, state.release, phase.actions.slice(boundedOffset, boundedOffset + 5), (relativeIndex, outcome) => {
      const index = relativeIndex + boundedOffset;
      if (outcome === "pass") {
        if (phaseState.actions[index].status === "complete") verifyOutputs(phaseState.actions[index].outputs);
        else completeAction(state, statePath, phaseIndex, index, { outputs: recordOutputs(phase.actions[index]) });
      }
      else { phaseState.actions[index].status = "action_required"; atomicJson(statePath, state); }
    });
    if (outcomes.some(outcome => outcome !== "pass")) {
      phaseState.status = "action_required"; state.status = "action_required"; atomicJson(statePath, state);
      fail(`bounded WT batch incomplete: ${outcomes.map((outcome, index) => `${phase.actions[index + boundedOffset].id}=${outcome}`).join(", ")}; independent results retained`, 75);
    }
  }
  phaseState.status = "running";
  atomicJson(statePath, state);
  process.stdout.write(`${phase.id}: start\n`);
  for (let actionIndex = 0; actionIndex < phase.actions.length; actionIndex += 1) {
    const action = phase.actions[actionIndex];
    const current = actionState(state, phaseIndex, actionIndex);
    if (action.type === "command") runCommand(action, current, state, statePath, phaseIndex, actionIndex, validated.recordsRoot);
    else if (action.type === "ait_closeout") runAitCloseout(action, current, state, statePath, phaseIndex, actionIndex, validated.recordsRoot);
    else if (action.type === "github_workflow") runGithubWorkflow(action, current, state, statePath, phaseIndex, actionIndex, validated.recordsRoot);
    else runEvidenceGate(action, current, state, statePath, phaseIndex, actionIndex);
  }
  phaseState.status = "complete";
  atomicJson(statePath, state);
  process.stdout.write(`${phase.id}: complete\n`);
}
state.status = "complete";
atomicJson(statePath, state);
printStatus(state);
