#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const REQUIRED_PHASES = [
  "source_preflight",
  "prior_qualification",
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

function validateArgv(argv, label) {
  const invalidIndex = Array.isArray(argv) ? argv.findIndex((row) => typeof row !== "string" || row.length === 0) : -1;
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
  const release = plan.release;
  if (
    release?.channel !== "stable" ||
    !stableAdvance(release.prior_version, release.version) ||
    release.tag !== `v${release.version}`
  ) fail("release conductor currently requires one exact stable patch or minor advance");
  const recordsRoot = requireAbsolute(plan.records_root, "records_root");
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
        validateArgv(action.argv, `${action.id}`);
        if (!Array.isArray(action.outputs) || action.outputs.length === 0) fail(`${action.id} must declare output receipts`);
        for (const output of action.outputs) {
          const resolved = requireAbsolute(output, `${action.id}.output`);
          if (!resolved.startsWith(`${recordsRoot}${path.sep}`)) fail(`${action.id} output must remain below records_root`);
        }
        if (action.mutation === true) {
          requireAbsolute(action.probe?.cwd, `${action.id}.probe.cwd`);
          validateArgv(action.probe?.argv, `${action.id}.probe`);
        } else if (action.probe !== undefined || action.mutation !== false) {
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
    if (!byPhase[id].some((row) => row.type === "github_workflow")) fail(`${id} requires a GitHub workflow action`);
  }
  for (const id of ["tag", "winget_submission"]) {
    if (!byPhase[id].some((row) => row.type === "command" && row.mutation === true)) fail(`${id} requires a probed mutation action`);
  }
  return { recordsRoot, planSha256: sha256Bytes(rawPlan), planPath };
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

function loadState(plan, statePath, planSha256) {
  if (!existsSync(statePath)) return initialState(plan, planSha256);
  const state = JSON.parse(readFileSync(statePath, "utf8"));
  if (state.contract !== "ait.release.conductor-state/v1" || state.plan_sha256 !== planSha256) {
    fail("release state does not bind the exact immutable plan");
  }
  if (JSON.stringify(state.release) !== JSON.stringify(plan.release)) fail("release state identity differs from the plan");
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

function runProcess(argv, cwd, log, replacements = {}) {
  const expanded = argv.map((value) => Object.entries(replacements).reduce(
    (current, [token, replacement]) => current.replaceAll(token, String(replacement)), value,
  ));
  const result = spawnSync(expanded[0], expanded.slice(1), {
    cwd,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  writeFileSync(log, `${result.stdout ?? ""}${result.stderr ?? ""}`, { mode: 0o600 });
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

function completeAction(state, statePath, phaseIndex, actionIndex, extra = {}) {
  Object.assign(actionState(state, phaseIndex, actionIndex), { status: "complete", ...extra });
  atomicJson(statePath, state);
}

function runCommand(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot) {
  if (current.status === "complete") {
    verifyOutputs(current.outputs);
    return;
  }
  const log = logPath(recordsRoot, action.id);
  const replacements = actionReplacements(action);
  if (action.mutation) {
    const probeLog = logPath(recordsRoot, action.id, "probe.log");
    const probe = runProcess(action.probe.argv, action.probe.cwd, probeLog, replacements);
    if (![0, 1].includes(probe.status)) fail(`${action.id} probe failed; see ${probeLog}`);
    if (probe.status === 1) {
      current.status = "prepared";
      atomicJson(statePath, state);
      process.stdout.write(`${action.id}: applying once\n`);
      const result = runProcess(action.argv, action.cwd, log, replacements);
      if (result.status !== 0) fail(`${action.id} failed; see ${log}`);
      const verified = runProcess(action.probe.argv, action.probe.cwd, probeLog, replacements);
      if (verified.status !== 0) fail(`${action.id} post-mutation probe failed; see ${probeLog}`);
    }
  } else {
    if (action.outputs.some((output) => existsSync(output))) fail(`${action.id} found unbound pre-existing output`);
    process.stdout.write(`${action.id}: running\n`);
    const result = runProcess(action.argv, action.cwd, log, replacements);
    if (result.status !== 0) fail(`${action.id} failed; see ${log}`);
  }
  completeAction(state, statePath, phaseIndex, actionIndex, { outputs: recordOutputs(action) });
}

function readJsonCommand(argv, cwd, log) {
  const result = runProcess(argv, cwd, log);
  if (result.status !== 0) return { result };
  try {
    return { result, json: JSON.parse(result.stdout) };
  } catch {
    fail(`command returned invalid JSON; see ${log}`);
  }
}

function runAitCloseout(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot) {
  const prefix = logPath(recordsRoot, action.id, "ait");
  let patchset = current.patchset ?? action.patchset ?? null;
  const remote = action.remote ?? "origin";
  const shownTask = patchset ? patchset.split("/")[0] : action.task;
  const taskShowArgv = ["ait", "task", "show", shownTask];
  if (patchset) taskShowArgv.push("--remote", remote);
  taskShowArgv.push("--json");
  const taskResult = readJsonCommand(taskShowArgv, action.repository_root, `${prefix}.task.json`);
  if (taskResult.result.status !== 0) fail(`${action.id} Task status failed; see ${prefix}.task.json`);
  const taskStatus = taskResult.json?.status ?? taskResult.json?.task?.status;
  if (taskStatus === "completed" && (patchset || action.finish_local_before_ready !== true)) {
    completeAction(state, statePath, phaseIndex, actionIndex, { patchset });
    process.stdout.write(`${shownTask}: already completed\n`);
    return;
  }
  if (!patchset) {
    if (action.finish_local_before_ready === true && taskStatus !== "completed") {
      const localFinish = runProcess(
        ["ait", "task", "finish", action.task, "--local", "--json"],
        action.edit_root,
        `${prefix}.local-finish.log`,
      );
      if (localFinish.status !== 0) fail(`${action.id} local Task finish failed; see ${prefix}.local-finish.log`);
      current.status = "local_finished";
      atomicJson(statePath, state);
      process.stdout.write(`${action.task}: local finish complete\n`);
    }
    const delays = [0, 5, 10, 20, 40, 60];
    for (const delay of delays) {
      if (delay > 0) sleep(delay);
      const ready = runProcess(
        ["ait", "workflow", "ready", action.task, "--apply", "--remote", remote],
        action.finish_local_before_ready === true ? action.repository_root : action.edit_root,
        `${prefix}.ready.log`,
      );
      if (ready.status === 0) {
        patchset = ready.stdout.match(/[A-Z]+T-[0-9]+\/P-[0-9]+/g)?.at(-1) ?? null;
        if (!patchset) fail(`${action.id} ready omitted the public Patchset reference`);
        current.patchset = patchset;
        current.status = "remote_ready";
        atomicJson(statePath, state);
        process.stdout.write(`${patchset}: ready\n`);
        break;
      }
      if (!/503|capacity is exhausted/i.test(`${ready.stdout}${ready.stderr}`)) fail(`${action.id} ready failed; see ${prefix}.ready.log`);
      process.stdout.write(`${action.task}: upload capacity busy; retrying the same Snapshot\n`);
    }
    if (!patchset) fail(`${action.id} upload capacity remained unavailable`, 75);
  }
  for (let attempt = 1; attempt <= 480; attempt += 1) {
    const ci = readJsonCommand(
      ["ait", "patchset", "ci-status", patchset, "--remote", remote, "--json"],
      action.repository_root,
      `${prefix}.ci.json`,
    );
    if (ci.result.status !== 0) fail(`${action.id} CI status failed; see ${prefix}.ci.json`);
    const overall = ci.json?.overall_status ?? "pending";
    const blocking = ci.json?.blocking_failure_count ?? 0;
    if (overall === "pass" && blocking === 0) break;
    if (overall === "fail" || blocking !== 0) fail(`${patchset}: CI failed; see ${prefix}.ci.json`);
    if (attempt === 480) fail(`${patchset}: CI wait expired`, 75);
    if (attempt % 4 === 0) process.stdout.write(`${patchset}: CI ${overall}\n`);
    sleep(15);
  }
  const remoteTask = patchset.split("/")[0];
  let completed = false;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const attestationReady = runProcess(
      ["ait", "workflow", "ready", remoteTask, "--apply", "--remote", remote],
      action.repository_root,
      `${prefix}.attestation-ready-${attempt}.log`,
    );
    if (attestationReady.status !== 0) {
      fail(`${action.id} post-CI ready failed; see ${prefix}.attestation-ready-${attempt}.log`);
    }
    const finish = runProcess(
      ["ait", "workflow", "finish", remoteTask, "--apply", "--remote", remote, "--review-message", action.review_message],
      action.repository_root,
      `${prefix}.finish-${attempt}.log`,
    );
    if (finish.status !== 0) fail(`${action.id} finish failed; see ${prefix}.finish-${attempt}.log`);
    const after = readJsonCommand(
      ["ait", "task", "show", remoteTask, "--remote", remote, "--json"],
      action.repository_root,
      `${prefix}.task-after-finish-${attempt}.json`,
    );
    if (after.result.status !== 0) fail(`${action.id} post-finish Task status failed; see ${prefix}.task-after-finish-${attempt}.json`);
    const afterStatus = after.json?.status ?? after.json?.task?.status;
    if (afterStatus === "completed") {
      completed = true;
      break;
    }
  }
  if (!completed) fail(`${action.id} did not reach completed Task state after bounded closeout`, 75);
  completeAction(state, statePath, phaseIndex, actionIndex, { patchset });
  process.stdout.write(`${patchset}: completed\n`);
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

function approvePendingDeployments(action, runId, recordsRoot) {
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
  const approvalLog = logPath(recordsRoot, action.id, "environment-approval.log");
  const approved = spawnSync("gh", ["api", "--method", "POST", endpoint, "--input", "-"], {
    cwd: action.cwd,
    encoding: "utf8",
    input: `${JSON.stringify({ environment_ids: environmentIds, state: "approved", comment: "Approved by the bound AIT stable release conductor." })}\n`,
    maxBuffer: 16 * 1024 * 1024,
  });
  writeFileSync(approvalLog, `${approved.stdout ?? ""}${approved.stderr ?? ""}`, { mode: 0o600 });
  if (approved.status !== 0) fail(`${action.id} protected environment approval failed; see ${approvalLog}`);
  process.stdout.write(`${action.id}: protected environment approved\n`);
}

function runGithubWorkflow(action, current, state, statePath, phaseIndex, actionIndex, recordsRoot) {
  if (current.status === "complete") {
    verifyOutputs(current.outputs);
    return;
  }
  const headSha = githubHeadSha(action);
  if (current.head_sha && current.head_sha !== headSha) fail(`${action.id} recorded workflow head SHA drifted`);
  let runId = current.run_id ?? null;
  if (!runId) {
    const runs = ghRuns(action, recordsRoot);
    const watermark = current.watermark ?? Math.max(0, ...runs.map((row) => row.databaseId));
    if (current.status === "pending") {
      Object.assign(current, { status: "prepared", watermark });
      atomicJson(statePath, state);
      const dispatchLog = logPath(recordsRoot, action.id, "dispatch.log");
      const dispatched = runProcess(action.dispatch_argv, action.cwd, dispatchLog, { "{head_sha}": headSha });
      if (dispatched.status !== 0) fail(`${action.id} dispatch failed; see ${dispatchLog}`);
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
    if (!runId) fail(`${action.id} dispatched workflow was not found`, 75);
  }
  const viewLog = logPath(recordsRoot, action.id, "view.json");
  for (let attempt = 1; attempt <= (action.max_wait_minutes ?? 180) * 4; attempt += 1) {
    approvePendingDeployments(action, runId, recordsRoot);
    const view = readJsonCommand(
      ["gh", "run", "view", String(runId), "--repo", action.repository, "--json", "status,conclusion,url"],
      action.cwd,
      viewLog,
    );
    if (view.result.status !== 0) fail(`${action.id} workflow status failed; see ${viewLog}`);
    if (view.json.status === "completed") {
      if (view.json.conclusion !== "success") fail(`${action.id} workflow run ${runId} concluded ${view.json.conclusion}`);
      break;
    }
    if (attempt % 4 === 0) process.stdout.write(`${action.id}: workflow run ${runId} is ${view.json.status}\n`);
    if (attempt === (action.max_wait_minutes ?? 180) * 4) {
      Object.assign(current, { status: "action_required", run_id: runId });
      atomicJson(statePath, state);
      fail(`${action.id}: workflow run ${runId} still requires attention; rerun after approval`, 75);
    }
    sleep(15);
  }
  let outputs = {};
  if (action.bind) {
    const bindLog = logPath(recordsRoot, action.id, "bind.log");
    const bound = runProcess(action.bind.argv, action.bind.cwd, bindLog, { "{run_id}": runId, "{head_sha}": headSha });
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
const rawPlan = readFileSync(planPath, "utf8");
let plan;
try { plan = JSON.parse(rawPlan); } catch { fail("release conductor plan is invalid JSON"); }
const validated = validatePlan(plan, rawPlan, planPath, statePath);
mkdirSync(validated.recordsRoot, { recursive: true });
mkdirSync(path.join(validated.recordsRoot, "conductor-logs"), { recursive: true, mode: 0o700 });
const state = loadState(plan, statePath, validated.planSha256);
if (!existsSync(statePath)) atomicJson(statePath, state);

if (cli.command === "status") {
  printStatus(state);
  process.exit(0);
}

for (let phaseIndex = 0; phaseIndex < plan.phases.length; phaseIndex += 1) {
  const phase = plan.phases[phaseIndex];
  const phaseState = state.phases[phaseIndex];
  if (phaseState.status === "complete") {
    for (const action of phaseState.actions) verifyOutputs(action.outputs);
    continue;
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
