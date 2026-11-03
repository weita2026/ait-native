#!/usr/bin/env node
// Explicit local protocol rehearsal. CLI/Server/Runner are real processes;
// hosted Actions/publication are fixtures. Remote finish waits for the actual
// independent Reviewer; this does not qualify the separate host/platform matrix.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, closeSync, existsSync, openSync, writeSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { packageIdentity, packageAcceptance, pendingPackageAcceptance, repreparedPackageIdentity, workflowResult, originalLocalSnapshot } from "./release_native_workflow.mjs";

const cli = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => process.argv.slice(2 + i * 2, 4 + i * 2)));
for (const key of ["--ait", "--server", "--runner", "--ram-root"]) {
  if (!path.isAbsolute(cli[key] ?? "")) throw new Error(`required explicit absolute path: ${key}`);
}
for (const key of ["--personal", "--chrome", "--playwright", "--headless-helper"]) {
  assert(!cli[key], "Personal UI is owner-deferred; this rehearsal uses native CLI/HTTP only");
}
const nativeWaitMs = Number(cli["--native-wait-ms"]);
assert(Number.isSafeInteger(nativeWaitMs) && nativeWaitMs > 0, "supply the admitted native wait budget plus transport allowance");
const reviewWaitMs = Number(cli["--review-wait-ms"] ?? 3600000);
assert(Number.isSafeInteger(reviewWaitMs) && reviewWaitMs > 0, "Reviewer wait must remain explicitly bounded");
const laterScene = cli["--later-scene"];
assert(["local-finish", "explicit-rebase"].includes(laterScene), "select one A120-04 scene with --later-scene");
const interruptFinish = laterScene === "explicit-rebase";
const laterNames = laterScene === "local-finish" ? ["later-local"] : ["later-rebase-base", "later-rebase"];
const laterFiles = laterScene === "local-finish" ? [["later.txt", "later Local history\n"]] :
  [["rebase-base.txt", "Local base for explicit rebase\n"], ["rebase.txt", "explicitly rebased Local history\n"]];
const here = path.dirname(fileURLToPath(import.meta.url));
const records = cli["--records-root"] ?? realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-native-control-records.")));
assert(path.isAbsolute(records));
const resume = Boolean(cli["--records-root"] && existsSync(records));
if (cli["--records-root"] && !resume) mkdirSync(records);
chmodSync(records, 0o700);
const planFile = path.join(records, "conductor-plan.json"), stateFile = path.join(records, "conductor-state.json");
const retainedPlan = resume ? JSON.parse(readFileSync(planFile)) : null;
const retainedAction = retainedPlan?.phases.find(phase => phase.id === "component_release")?.actions.find(action => action.id === "component");
if (resume) {
  assert.equal(retainedPlan.contract, "ait.release.conductor-plan/v1");
  assert.equal(realpathSync(retainedPlan.records_root), realpathSync(records));
  assert.equal(retainedAction?.type, "ait_closeout");
  assert.equal(retainedAction.ait_bin, cli["--ait"], "resume the same fixed CLI");
  // A response may represent a native mutation even when the UI rejects its
  // shape. Refuse before starting a Server, which would reset unfinished CI.
  assert(!existsSync(path.join(records, "conductor-logs/component.ait.ready-pinned.stdout")), "a captured request needs its own reconciliation, not another mutation");
  assert(!existsSync(path.join(records, "conductor-logs/component.ait.ready-pinned.log")), "a captured native invocation requires reconciliation, never a reset or replay");
  assert(!existsSync(path.join(records, "conductor-logs/component.ait.finish.log")), "never repeat a captured finish");
  assert.match(readFileSync(path.join(records, "conductor-logs/component.ait.ready-pinned.stderr"), "utf8"), /observed workflow requests=0/);
}
const scratch = resume ? realpathSync(path.dirname(retainedAction.repository_root)) : realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-native-real-control.")));
assert(path.basename(scratch).startsWith("ait-native-real-control."));
const ram = resume ? realpathSync(cli["--ram-root"]) : realpathSync(mkdtempSync(path.join(cli["--ram-root"], "ait-native-real-control.")));
if (resume) assert(path.basename(ram).startsWith("ait-native-real-control."), "resume with the original owned RAM directory");
const repo = path.join(scratch, "fixture-repository"); if (!resume) mkdirSync(repo);
if (resume) assert.equal(realpathSync(retainedAction.repository_root), repo);
const bin = path.join(scratch, "bin"); if (!resume) mkdirSync(bin);
// Preserve the actual caller/host registry. Component-only stub hosts and
// erased thread IDs cannot admit this real Reviewer/CI scenario.
const env = { ...process.env, PATH: bin + path.delimiter + process.env.PATH,
  AIT_NATIVE_SERVER_CI_RAM_ROOT: ram, AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" };
const children = new Set();
function launch(command, args, cwd = scratch) {
  const child = spawn(command, args, { cwd, env }); children.add(child);
  const result = { child, stdout: "", stderr: "", code: null };
  child.stdout.on("data", bytes => { result.stdout += bytes; });
  child.stderr.on("data", bytes => { result.stderr += bytes; });
  result.closed = new Promise(resolve => child.once("close", (code, signal) => { children.delete(child); result.code = code ?? 128; resolve(result); }));
  return result;
}
function ait(args, cwd = repo) {
  const out = spawnSync(cli["--ait"], args, { cwd, env, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  assert.equal(out.status, 0, `${args.join(" ")}\n${out.stdout}\n${out.stderr}`);
  return { json: JSON.parse(out.stdout), stdout: out.stdout };
}
if (resume) {
  const titles = new Set(ait(["task", "list", "--all", "--json"]).json.map(task => task.title));
  assert(laterNames.every(name => titles.has(name)) &&
    ["later-local", "later-rebase-base", "later-rebase"].filter(name => !laterNames.includes(name))
      .every(name => !titles.has(name)), "resume requires the original A120-04 scene");
}
async function waitFor(read, label, timeout = 45000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const result = await read(); if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 30));
  }
  throw new Error(`rehearsal timeout: ${label}`);
}
async function unusedPort() {
  const server = createServer(); await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
const retainedRemote = resume ? ait(["remote", "list", "--json"]).json.find(remote => remote.name === "origin") : null;
const port = resume ? Number(new URL(retainedRemote.url).port) : await unusedPort();
const url = `http://127.0.0.1:${port}`;
if (resume) assert.equal(retainedRemote.url, url, "resume only the original isolated loopback authority");
const dataRoot = path.join(scratch, "server-data");
let server;
async function startServer() {
  server = launch(cli["--server"], ["--data", dataRoot, "--listen", `127.0.0.1:${port}`, "--init-if-missing", "--defer-ci-admission"]);
  await waitFor(async () => {
    if (server.code !== null) throw new Error(`isolated Server exited: ${server.stderr.slice(-2000)}`);
    try { const response = await fetch(url + "/healthz"); return response.ok; } catch { return false; }
  }, "isolated Server ready");
}
async function stop(child) {
  if (!child || child.code !== null) return;
  child.child.kill("SIGTERM");
  await Promise.race([child.closed, new Promise(resolve => setTimeout(resolve, 3000))]);
  if (child.code === null) { child.child.kill("SIGKILL"); await child.closed; }
}
function startTask(name) {
  return ait(["task", "start", "--local", "--title", name, "--intent", name,
    "--edit-root", path.join(scratch, name), "--json"]).json;
}
function finishLocal(task, clean = false) {
  return ait(["task", "finish", task.task_id, "--local",
    ...(clean ? [] : ["--message", "Protocol fixture source"]), "--json"], task.edit_root).json;
}
function completedLocalByTitle(title) {
  const matches = ait(["task", "list", "--all", "--json"]).json.filter(task => task.title === title);
  assert.equal(matches.length, 1, `expected one completed ${title} fixture Task`);
  const task = matches[0];
  return { task_id: task.task_id, snapshot_id: originalLocalSnapshot(task, task.task_id) };
}
function snapshotDescendsFrom(start, base) {
  let cursor = start;
  for (let depth = 0; depth < 256; depth++) {
    if (cursor === base) return true;
    const parents = ait(["snapshot", "show", cursor, "--json"]).json.parent_snapshot_ids;
    assert(Array.isArray(parents) && parents.length <= 1, "Local source ancestry must be linear");
    if (!parents.length) return false;
    cursor = parents[0];
  }
  throw new Error("Local source ancestry exceeded its bounded inventory");
}
let keepFixture = false;

const gate = path.join(scratch, "ci-release");
let runner, controller;
const pin = path.join(records, "conductor-receipts/component.package-identity.json");
const runController = () => launch(process.execPath, [path.join(here, "release_conductor.mjs"), "run", "--plan", planFile, "--state", stateFile], repo);
let config, source, identity, beforeRestart;
try {
  await startServer();
  if (!resume) {
  const callerConfig = ait(["config", "show", "--json"], process.cwd()).json;
  for (const key of ["user_name", "user_email"]) {
    assert(typeof callerConfig[key] === "string" && callerConfig[key].trim(), `real caller config ${key} is required`);
  }
  const initialization = spawnSync(cli["--ait"], ["init", "--json"], { cwd: repo, env, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  const initializationLog = path.join(records, "conductor-logs"); mkdirSync(initializationLog, { recursive: true, mode: 0o700 });
  writeFileSync(path.join(initializationLog, "component.ait.init.stdout"), initialization.stdout ?? "", { mode: 0o600 });
  writeFileSync(path.join(initializationLog, "component.ait.init.stderr"), initialization.stderr ?? String(initialization.error ?? ""), { mode: 0o600 });
  assert.equal(initialization.status, 0, "native fixture admission failed; preserve its original init output");
  const initialized = JSON.parse(initialization.stdout);
  assert.equal(initialized.action, "initialized", "require a newly initialized disposable database");
  assert.equal(realpathSync(initialized.repo_root), repo);
  const admission = initialized.agent_team_bootstrap;
  assert(admission?.status === "ready" || admission?.status === "disabled" ||
    admission?.status === "pending" && admission.detail === "Existing thread identity/workspace mismatch",
  `Native fixture initialization is ${admission?.status ?? "unavailable"}: ${admission?.detail ?? "unknown admission"}; no candidate/CI started`);
  // This disposable component repository uses the existing external-orchestrator
  // route. It never adopts the caller as coordinator or qualifies fresh Teams.
  ait(["config", "set", "--agent-teams", "disabled", "--sprint", "off", "--default-author-mode", "ai_only_experimental",
    "--user-name", callerConfig.user_name, "--user-email", callerConfig.user_email, "--json"]);
  const fixtureConfig = ait(["config", "show", "--json"]);
  writeFileSync(path.join(initializationLog, "component.ait.config.stdout"), fixtureConfig.stdout, { mode: 0o600 });
  assert.equal(realpathSync(fixtureConfig.json.repo_root), repo, "config must read the initialized fixture database");
  assert.equal(fixtureConfig.json.agent_teams, false);
  assert.equal(fixtureConfig.json.sprint?.value, "off");
  assert.equal(fixtureConfig.json.default_author_mode, "ai_only_experimental");
  ait(["remote", "add", "origin", url, "--default", "--json"]);
  config = ait(["config", "show", "--json"]).json;
  assert(Number.isSafeInteger(config.repository_index));
  const base = startTask("baseline");
  mkdirSync(path.join(base.edit_root, "ci"));
  writeFileSync(path.join(base.edit_root, "payload.txt"), "baseline\n");
  writeFileSync(path.join(base.edit_root, "ci/check.sh"), '#!/bin/sh\nset -eu\ntest "$(cat payload.txt)" = candidate\nfixture_gate=${1:-${NATIVE_CONTROL_GATE:?}}\nprintf running > "$fixture_gate.entered"\nwhile test ! -f "$fixture_gate"; do sleep 0.02; done\ntest "$(cat payload.txt)" = candidate\nprintf "verified exact candidate bytes\\n"\n');
  const quotedGate = "'" + gate.replaceAll("'", "'\\''") + "'";
  writeFileSync(path.join(base.edit_root, "ci/run.sh"), '#!/bin/sh\nset -eu\nexec sh "$(dirname "$0")/check.sh" ' + quotedGate + '\n', { mode: 0o755 });
  writeFileSync(path.join(base.edit_root, "ci/patch_ci.json"), JSON.stringify({ schema_version: 1, suites: [{ suite_id: "native_protocol", display_name: "Native protocol fixture", plane: "patchset", mode: "gate", default_blocking: true,
    runner: { kind: "command_bundle", commands: ["sh ci/check.sh"], env: { NATIVE_CONTROL_GATE: gate }, timeout_seconds: 60 } }] }));
  finishLocal(base);
  ait(["push", "--remote", "origin", "--line", "main", "--json"]);
  source = startTask("candidate");
  writeFileSync(path.join(source.edit_root, "payload.txt"), "candidate\n");
  const finished = finishLocal(source);
  originalLocalSnapshot(ait(["task", "show", source.task_id, "--local", "--json"]).json, source.task_id, finished.landed_snapshot_id);

  const fixture = path.join(scratch, "control-fixture.mjs");
  writeFileSync(fixture, `import fs from 'node:fs'; const [op,id,file]=process.argv.slice(2); if(op==='probe')process.exit(1); if(op==='dispatch'){const f=${JSON.stringify(records)}+'/'+id+'.dispatch';if(fs.existsSync(f))process.exit(99);fs.writeFileSync(f,'1');}else fs.writeFileSync(file,id);`);
  const workflows = ["public_qualification", "component_receipts", "clean_host_qualification", "protected_promotion", "endpoint_publication", "latest_alias"];
  const gh = path.join(bin, "gh");
  writeFileSync(gh, `#!/usr/bin/env node\nimport fs from 'node:fs'; const args=process.argv.slice(2); const workflows=${JSON.stringify(workflows)}; if(args[0]==='run'&&args[1]==='list'){const name=args[args.indexOf('--workflow')+1];console.log(JSON.stringify(fs.existsSync(${JSON.stringify(records)}+'/'+name+'.dispatch')?[{databaseId:workflows.indexOf(name)+1,headSha:'${"a".repeat(40)}',status:'completed',conclusion:'success'}]:[]));}else if(args[0]==='run'&&args[1]==='view')console.log(JSON.stringify({status:'completed',conclusion:'success',headSha:'${"a".repeat(40)}',attempt:1}));else process.exit(99);\n`); chmodSync(gh, 0o755);
  const phases = ["source_preflight", "component_release", "candidate_freeze", "public_qualification", "candidate_admission", "component_receipts", "clean_host_qualification", "web_admission", "tag", "protected_promotion", "endpoint_publication", "winget_submission", "latest_alias", "closeout"];
  const action = id => ({ id, type: "command", mutation: ["tag", "winget_submission"].includes(id), cwd: repo,
    argv: [process.execPath, fixture, "produce", id, path.join(records, id + ".fixture")], outputs: [path.join(records, id + ".fixture")],
    ...(["tag", "winget_submission"].includes(id) ? { probe: { cwd: repo, argv: [process.execPath, fixture, "probe", id] } } : {}) });
  const plan = { contract: "ait.release.conductor-plan/v1", records_root: records, release: { prior_version: "1.1.4", version: "1.2.0", tag: "v1.2.0", channel: "stable" },
    phases: phases.map(id => ({ id, actions: id === "component_release" ? [{ id: "component", type: "ait_closeout", task: source.task_id, snapshot: finished.landed_snapshot_id,
      repository_root: repo, edit_root: source.edit_root, ait_bin: cli["--ait"], finish_local_before_ready: true,
      review_message: "Reviewed files: await independent Reviewer. Findings: pending. Risks: fixture. Tests: native protocol. Recommendation: await Reviewer." }] :
      workflows.includes(id) ? [{ id, type: "github_workflow", repository: "fixture/fixture", workflow: id, cwd: repo, head_sha: "a".repeat(40), dispatch_argv: [process.execPath, fixture, "dispatch", id] }] : [action(id)] })) };
  writeFileSync(planFile, JSON.stringify(plan));
  controller = runController();
  const jobUrl = `${url}/v1/native/repository-authorities/${config.repository_index}/worker-jobs/0`;
  const job = await waitFor(async () => {
    if (controller.code !== null) throw new Error(`controller stopped before package CI: ${controller.stderr}`);
    const response = await fetch(jobUrl); if (!response.ok) return false;
    const body = await response.json(); const value = body.data ?? body;
    return value.package_execution?.ci_state === "queued" ? value.package_execution : false;
  }, "actual original queued package Job");
  identity = packageIdentity(job.expected_identity);
  assert.equal(identity.revision_snapshot_id, finished.landed_snapshot_id);
  runner = launch(cli["--runner"], ["run-job", "--server", url, "--repository-index", String(identity.repository_index), "--worker-job-index", String(identity.worker_job_index),
    "--source-root", repo, "--attempt-root", path.join(ram, "runner-attempt")]);
  await waitFor(async () => {
    if (runner.code !== null) throw new Error(`actual Runner exited before CI boundary: ${runner.stdout}\n${runner.stderr}`);
    const response = await fetch(jobUrl); const body = await response.json(); return (body.data ?? body).package_execution?.ci_state === "running" && existsSync(gate + ".entered");
  }, "actual running CI with controlled source gate");
  writeFileSync(gate, "release");
  const run = await runner.closed;
  assert.equal(run.code, 0, `actual Runner failed: ${run.stdout}\n${run.stderr}`);
  const ready = await controller.closed;
  assert.equal(ready.code, 75, ready.stderr);
  assert.match(ready.stderr, /independent Reviewer must finish/);
  const state = JSON.parse(readFileSync(stateFile));
  assert.deepEqual(state.phases[1].actions[0].expected_identity, identity);
  const beforeRestartRaw = await (await fetch(jobUrl)).text();
  writeFileSync(path.join(records, "conductor-logs/component.ait.job-before-restart.stdout"), beforeRestartRaw, { mode: 0o600 });
  const beforeRestartBody = JSON.parse(beforeRestartRaw); beforeRestart = beforeRestartBody.data ?? beforeRestartBody;
  assert.equal(beforeRestart.job.tombstoned, false, "original Job must be live before finish");
  assert.equal(beforeRestart.job.patchset_index, null, "CI must not create formal publication");
  await stop(server); await startServer();
  const afterRestartRaw = await (await fetch(jobUrl)).text();
  writeFileSync(path.join(records, "conductor-logs/component.ait.job-after-restart.stdout"), afterRestartRaw, { mode: 0o600 });
  const afterRestartBody = JSON.parse(afterRestartRaw), afterRestart = afterRestartBody.data ?? afterRestartBody;
  assert.equal(beforeRestart.package_execution.author_mode, "ai_only_experimental", "mode must come from the verified sealed-package getter");
  assert(!(afterRestart.package_execution?.ci_state === "passed" && afterRestart.package_execution.availability === "available" && afterRestart.package_execution.recoverable),
    "unfinished preformal CI must not remain eligible after actual Server reset");
  const oldObservation = spawnSync(cli["--ait"], ["workflow", "finish", source.task_id, "--remote", "origin", "--expected-identity", pin, "--json"],
    { cwd: repo, env, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  writeFileSync(path.join(records, "conductor-logs/component.ait.finish-readback.log"), oldObservation.stdout, { mode: 0o600 });
  const oldResult = JSON.parse(oldObservation.stdout);
  workflowResult(oldResult, "finish", identity, { readonly: true });
  assert.equal(packageAcceptance(oldResult, identity, source.task_id), null, "old-pin finish cannot borrow replacement evidence");
  const rejectedRunner = launch(cli["--runner"], ["run-job", "--server", url, "--repository-index", String(identity.repository_index),
    "--worker-job-index", String(identity.worker_job_index), "--source-root", repo, "--attempt-root", path.join(ram, "stale-runner-attempt")]);
  const rejected = await rejectedRunner.closed;
  writeFileSync(path.join(records, "conductor-logs/component.ait.stale-runner.stdout"), rejected.stdout, { mode: 0o600 });
  writeFileSync(path.join(records, "conductor-logs/component.ait.stale-runner.stderr"), rejected.stderr, { mode: 0o600 });
  assert.notEqual(rejected.code, 0, "invalidated original Job must reject an old execution attempt");
  assert.match(rejected.stderr, /\bHTTP (?:404|409|410)\b|HTTP 400: prepared CI Job has no recoverable managed package association/,
    "require an actual native stale/invalid Job refusal, not a transport failure");
  assert.equal((await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${identity.worker_job_index + 1}`)).status, 404,
    "readonly/finish/old execution probes must create no replacement");

  // Each invocation seals one original package and exercises one A120-04
  // scene. The rebase scene needs its own later Local base to rebase onto.
  if (laterScene === "local-finish") {
    const later = startTask("later-local");
    writeFileSync(path.join(later.edit_root, laterFiles[0][0]), laterFiles[0][1]);
    finishLocal(later);
  } else {
    const rebased = startTask("later-rebase");
    writeFileSync(path.join(rebased.edit_root, laterFiles[1][0]), laterFiles[1][1]);
    ait(["snapshot", "create", rebased.task_id, "--message", "Prepare later rebase", "--json"], rebased.edit_root);
    const localBase = startTask("later-rebase-base");
    writeFileSync(path.join(localBase.edit_root, laterFiles[0][0]), laterFiles[0][1]);
    finishLocal(localBase);
    ait(["worktree", "rebase", rebased.worktree_name, "--onto", "main", "--json"], rebased.edit_root);
    finishLocal(rebased, true);
  }
  for (const [name, content] of laterFiles) assert.equal(readFileSync(path.join(repo, name), "utf8"), content);
  ait(["config", "set", "--default-author-mode", "ai_with_human_review", "--json"]);
  } else {
    // Reuse the original zero-POST failure. Do not repeat fixture Tasks, CI,
    // restart assertions, default changes or any accepted operation.
    identity = packageIdentity(JSON.parse(readFileSync(pin)));
    const state = JSON.parse(readFileSync(stateFile));
    const current = state.phases.find(phase => phase.id === "component_release").actions.find(action => action.id === "component");
    assert.deepEqual(packageIdentity(current.expected_identity), identity);
    assert(!current.complete && !current.completion && !current.patchset_id && !current.closeout, "resume only unaccepted work");
    const originalBody = JSON.parse(readFileSync(path.join(records, "conductor-logs/component.ait.job-before-restart.stdout")));
    beforeRestart = originalBody.data ?? originalBody;
    assert.deepEqual(packageIdentity(beforeRestart.package_execution.expected_identity), identity);
    assert.equal(beforeRestart.package_execution.author_mode, "ai_only_experimental");
    config = ait(["config", "show", "--json"]).json;
    assert.equal(config.repository_index, identity.repository_index);
    assert.equal(config.agent_teams, false); assert.equal(config.sprint?.value, "off");
    assert.equal(retainedAction.edit_root, path.join(scratch, "candidate"),
      "resume requires the original candidate edit root");
    source = { task_id: retainedAction.task, edit_root: retainedAction.edit_root };
    assert.equal(retainedAction.snapshot, identity.revision_snapshot_id);
    const old = await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${identity.worker_job_index}`);
    assert.equal(old.status, 200);
    const oldBody = await old.json(), oldJob = oldBody.data ?? oldBody;
    assert.equal(oldJob.job.patchset_index, null);
    assert.equal(oldJob.package_execution, null, "the original execution remains invalidated");
    assert.equal((await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${identity.worker_job_index + 1}`)).status, 404,
      "a replacement already exists; observe it without another mutation");
  }
  assert.equal(originalLocalSnapshot(ait(["task", "show", source.task_id, "--local", "--json"]).json, source.task_id), identity.revision_snapshot_id);
  const laterTasks = laterNames.map(completedLocalByTitle);
  for (const task of laterTasks) assert.notEqual(task.snapshot_id, identity.revision_snapshot_id);
  const sealedSource = ait(["snapshot", "show", identity.revision_snapshot_id, "--json"]).json;
  assert(!sealedSource.files.some(file => laterFiles.some(([name]) => name === file.path)),
    "later Local finish/rebase must stay outside the original sealed package");
  // Keep the original pinned invocation alive while the real Runner executes.
  // A read-only Server inventory may identify the unique queued Job for CI;
  // only the captured native Ready response selects its final identity.
  assert.deepEqual(packageIdentity(JSON.parse(readFileSync(pin))), identity);
  const readyLog = path.join(records, "conductor-logs/component.ait.ready-pinned.log");
  const stdoutFd = openSync(readyLog, "wx", 0o600), stderrFd = openSync(readyLog + ".stderr", "wx", 0o600);
  const nativeReady = launch(cli["--ait"], ["workflow", "ready", source.task_id, "--apply", "--remote", "origin",
    "--expected-identity", pin, "--json", "--full"], repo);
  nativeReady.child.stdout.on("data", bytes => writeSync(stdoutFd, bytes));
  nativeReady.child.stderr.on("data", bytes => writeSync(stderrFd, bytes));
  nativeReady.closed.then(() => { closeSync(stdoutFd); closeSync(stderrFd); });
  const observed = await waitFor(async () => {
    if (nativeReady.code !== null) throw new Error(`pinned Ready exited before replacement CI: ${nativeReady.stderr}`);
    const listResponse = await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs`);
    assert.equal(listResponse.status, 200, "replacement Job inventory must be readable");
    const inventory = await listResponse.json();
    assert(Array.isArray(inventory.jobs), "replacement Job inventory must be complete");
    const newer = inventory.jobs.filter(job => job.job_kind === 7 &&
      job.worker_job_index > identity.worker_job_index && !job.tombstoned);
    assert(newer.length <= 1, "replacement Job inventory is ambiguous");
    if (!newer.length) return false;
    const jobUrl = `${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${newer[0].worker_job_index}`;
    const jobResponse = await fetch(jobUrl);
    assert.equal(jobResponse.status, 200);
    const body = await jobResponse.json(), job = body.data ?? body;
    if (!job.package_execution) return false;
    const provisional = repreparedPackageIdentity(identity,
      packageIdentity(job.package_execution.expected_identity));
    assert.equal(provisional.worker_job_index, newer[0].worker_job_index);
    assert.equal(job.job.patchset_index, null);
    assert.equal(job.package_execution.author_mode, beforeRestart.package_execution.author_mode);
    assert.equal(job.package_execution.ci_state, "queued");
    assert.equal(job.package_execution.availability, "available");
    return { provisional, jobUrl };
  }, "unique retained replacement Job while pinned Ready waits", nativeWaitMs);
  assert.equal(nativeReady.code, null, "pinned Ready must remain pending until the real replacement Runner executes");
  assert.equal(readFileSync(readyLog).length, 0, "pinned Ready cannot deliver its final response before replacement CI");
  runner = launch(cli["--runner"], ["run-job", "--server", url, "--repository-index", String(observed.provisional.repository_index),
    "--worker-job-index", String(observed.provisional.worker_job_index), "--source-root", repo, "--attempt-root", path.join(ram, "replacement-runner-attempt")]);
  const nextRun = await runner.closed;
  writeFileSync(path.join(records, "conductor-logs/component.ait.replacement-runner.stdout"), nextRun.stdout, { mode: 0o600 });
  writeFileSync(path.join(records, "conductor-logs/component.ait.replacement-runner.stderr"), nextRun.stderr, { mode: 0o600 });
  assert.equal(nextRun.code, 0, `${nextRun.stdout}\n${nextRun.stderr}`);
  const passedRaw = await (await fetch(observed.jobUrl)).text();
  writeFileSync(path.join(records, "conductor-logs/component.ait.job-reprepared.stdout"), passedRaw, { mode: 0o600 });
  const passedBody = JSON.parse(passedRaw), passed = passedBody.data ?? passedBody;
  assert.equal(passed.job.patchset_index, null, "replacement CI must not create formal publication");
  assert.equal(passed.package_execution.author_mode, beforeRestart.package_execution.author_mode);
  assert.equal(passed.package_execution.ci_state, "passed");
  assert.deepEqual(packageIdentity(passed.package_execution.expected_identity), observed.provisional);
  await waitFor(() => nativeReady.code !== null && nativeReady, "original-pin native Ready response after real CI", nativeWaitMs);
  await nativeReady.closed;
  assert.equal(nativeReady.code, 0, `pinned Ready failed after real CI: ${nativeReady.stderr}`);
  const capturedReadyRaw = readFileSync(readyLog, "utf8");
  const response = JSON.parse(capturedReadyRaw);
  const nextIdentity = repreparedPackageIdentity(identity, workflowResult(response, "ready", null, { invocation: {
    task: source.task_id, remote: "origin", pinPath: pin, expectedIdentity: identity,
  } }));
  assert.deepEqual(nextIdentity, observed.provisional, "native Ready must select the exact observed real-CI Job");
  assert.equal(response.package_execution?.author_mode, beforeRestart.package_execution.author_mode);
  assert.equal(response.package_execution?.ci_state, "passed");
  // Reconcile the original capture through the existing controller transaction.
  controller = runController(); const selected = await controller.closed;
  assert.equal(selected.code, 75, selected.stderr);
  assert.deepEqual(JSON.parse(readFileSync(pin)), nextIdentity);
  assert.equal(readFileSync(readyLog, "utf8"), capturedReadyRaw, "retain the original prepare response");
  assert.deepEqual(JSON.parse(readFileSync(stateFile)).phases[1].actions[0].expected_identity, nextIdentity);
  // A passed-result replay joins the same replacement instead of preparing a
  // third Job or borrowing the old execution's evidence.
  const joined = ait(["workflow", "ready", source.task_id, "--apply", "--remote", "origin", "--expected-identity", pin, "--json"]);
  assert.deepEqual(workflowResult(joined.json, "ready"), nextIdentity);
  assert.equal((await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${nextIdentity.worker_job_index + 1}`)).status, 404);
  controller = runController(); const reviewWait = await controller.closed;
  assert.equal(reviewWait.code, 75, reviewWait.stderr); assert.match(reviewWait.stderr, /independent Reviewer must finish/);
  assert.deepEqual(JSON.parse(readFileSync(pin)), nextIdentity);
  assert.equal(readFileSync(readyLog, "utf8"), capturedReadyRaw, "CI resume must retain the original prepare response");
  const finishLog = path.join(records, "conductor-logs/component.ait.finish.log");
  const boundaryLog = path.join(records, "conductor-logs/component.ait.finish-interruption.json");
  process.stdout.write(`Assigned WT Reviewer required: Task=${source.task_id} repository=${repo} remote=origin expected-identity=${pin} original-stdout=${finishLog} records-root=${records} boundary=${interruptFinish ? "native-process-interruption" : "closed-success"}${interruptFinish ? ` driver=${path.join(here, "release_finish_process_loss.mjs")} boundary-log=${boundaryLog}` : ""}\n`);
  if (interruptFinish) process.stdout.write(`Review driver inputs: --ait=${cli["--ait"]} --repository=${repo} --server=${url} --identity=${pin} --stdout=${finishLog} --stderr=${finishLog}.stderr --boundary=${boundaryLog} --repository-index=${nextIdentity.repository_index} --worker-job-index=${nextIdentity.worker_job_index} --task=${source.task_id}; Reviewer supplies --reviewer and --review-message to the native workflow finish after --.\n`);
  // The actual Reviewer independently reads/reviews the candidate and invokes
  // the existing guarded finish. No Developer-supplied name, review or apply.
  await waitFor(() => {
    if (!existsSync(finishLog) || interruptFinish && !existsSync(boundaryLog)) return false;
    try {
      const handles = spawnSync("lsof", ["-nP", "-Ffa", "--", finishLog], { encoding: "utf8" });
      return handles.status === 1 && !handles.stdout.trim() &&
        (interruptFinish || readFileSync(finishLog).length > 0);
    } catch { return false; }
  }, "actual independent Reviewer original finish capture", reviewWaitMs);
  let interruptedFormal = null;
  if (interruptFinish) {
    const boundary = JSON.parse(readFileSync(boundaryLog, "utf8"));
    assert.equal(boundary.status, "interrupted_after_formal_job", "require a pending native CLI interruption");
    assert.equal(boundary.repository_index, nextIdentity.repository_index);
    assert.equal(boundary.worker_job_index, nextIdentity.worker_job_index);
    assert(Number.isSafeInteger(boundary.patchset_index) && boundary.patchset_index >= 0);
    assert.equal(boundary.client_signal, "SIGKILL");
    assert.equal(boundary.stdout_bytes, 0);
    assert.equal(readFileSync(finishLog).length, 0, "interrupted original CLI must not have delivered a response");
    const observed = ait(["workflow", "finish", source.task_id, "--remote", "origin",
      "--expected-identity", pin, "--json"]).json;
    workflowResult(observed, "finish", nextIdentity, { readonly: true });
    interruptedFormal = pendingPackageAcceptance(observed, nextIdentity, source.task_id);
  }
  // Remove the actual temporary CI RAM state across a native Server restart.
  await stop(server);
  rmSync(ram, { recursive: true, force: true });
  assert(!existsSync(ram), "temporary package/CI RAM state must be removed");
  mkdirSync(ram, { mode: 0o700 });
  await startServer();
  const afterTask = startTask("after-acceptance");
  writeFileSync(path.join(afterTask.edit_root, "after.txt"), "Local history after remote acceptance\n");
  finishLocal(afterTask);
  const afterAcceptance = completedLocalByTitle("after-acceptance");
  const acceptedRemoteHead = ait(["line", "show", "main", "--remote", "origin", "--json"]).json.head_snapshot_id;
  assert.match(acceptedRemoteHead, /^SNP-[0-9A-F]{12}$/);
  assert.equal(ait(["line", "show", "main", "--json"]).json.head_snapshot_id,
    completedLocalByTitle("after-acceptance").snapshot_id, "newer Local finish must advance main before read-only retry");
  const beforeRetry = ait(["workflow", "finish", source.task_id, "--remote", "origin",
    "--expected-identity", pin, "--json"]).json;
  workflowResult(beforeRetry, "finish", nextIdentity, { readonly: true });
  const formalBeforeRetry = pendingPackageAcceptance(beforeRetry, nextIdentity, source.task_id);
  const remoteTaskId = formalBeforeRetry.patchset_id.split("/")[0];
  assert(/^[A-Z]+T-[0-9]+$/.test(remoteTaskId));
  const remoteTaskBefore = ait(["task", "show", remoteTaskId, "--remote", "origin", "--json"]).json;
  // AE3's accepted retry is read-only formal recognition. Its pending mapping
  // phase must not be promoted to a complete publication or trigger another
  // finish merely because the original native response was lost.
  const reconciled = [];
  for (const command of ["finish", "ready"]) {
    for (const [selection, args] of [["pinned", ["--expected-identity", pin]], ["unpinned", []]]) {
      const value = ait(["workflow", command, source.task_id, "--remote", "origin", ...args, "--json"]).json;
      workflowResult(value, command, nextIdentity, { readonly: true });
      const completion = pendingPackageAcceptance(value, nextIdentity, source.task_id);
      assert.equal(completion.patchset_id, formalBeforeRetry.patchset_id,
        `${command}/${selection} changed the original Patchset`);
      assert.equal(completion.submission_id, formalBeforeRetry.submission_id,
        `${command}/${selection} changed the original submission`);
      reconciled.push(completion);
    }
  }
  for (const completion of reconciled.slice(1)) {
    assert.deepEqual(completion, reconciled[0], "four accepted retries must retain the same formal acceptance and pending mapping phase");
  }
  assert.equal(reconciled[0].patchset_id, formalBeforeRetry.patchset_id);
  assert.equal(reconciled[0].submission_id, formalBeforeRetry.submission_id);
  assert.deepEqual(ait(["task", "show", remoteTaskId, "--remote", "origin", "--json"]).json,
    remoteTaskBefore, "accepted retries must create no new remote review, approval or apply");
  assert.equal((await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${nextIdentity.worker_job_index + 1}`)).status,
    404, "accepted retries must create no additional package Job or CI");
  assert(!existsSync(source.edit_root), "original Local finish must complete the source Task worktree cleanup");
  const localSource = ait(["task", "show", source.task_id, "--local", "--json"]).json;
  if (interruptFinish) {
    assert.equal(reconciled[0].patchset_id, interruptedFormal.patchset_id);
    assert.equal(reconciled[0].submission_id, interruptedFormal.submission_id);
    assert.equal(localSource.published_task_id, null,
      "lost native response cannot establish original Local publication mappings");
    assert.equal(localSource.publication_state, "local_draft");
  } else {
    const originalFinish = JSON.parse(readFileSync(finishLog, "utf8"));
    workflowResult(originalFinish, "finish", nextIdentity, { invocation: {
      task: source.task_id, remote: "origin", pinPath: pin, expectedIdentity: nextIdentity,
    } });
    const originalAcceptance = packageAcceptance(originalFinish, nextIdentity, source.task_id);
    assert(originalAcceptance?.complete, "matching original Reviewer capture must prove full mappings and cleanup");
    assert.equal(originalAcceptance.completion.patchset_id, formalBeforeRetry.patchset_id);
    assert.equal(originalAcceptance.completion.submission_id, formalBeforeRetry.submission_id);
    assert.deepEqual(originalAcceptance.completion.entries,
      [{ local_task_id: source.task_id, remote_task_id: remoteTaskId, changes: [] }]);
    assert.equal(localSource.published_task_id, remoteTaskId,
      "the original complete response must own its Local publication mapping");
    assert.equal(localSource.publication_state, "published");
  }
  for (const [name, content] of [...laterFiles, ["after.txt", "Local history after remote acceptance\n"]]) {
    assert.equal(readFileSync(path.join(repo, name), "utf8"), content);
  }
  assert.equal(ait(["line", "show", "main", "--remote", "origin", "--json"]).json.head_snapshot_id,
    acceptedRemoteHead, "read-only retry must not advance Remote main");
  if (interruptFinish) {
    keepFixture = true;
    assert.equal(JSON.parse(readFileSync(stateFile)).phases[1].actions[0].status, "action_required",
      "the interrupted original response must retain the component closeout HOLD");
    console.log(`actual restart/reprepare and ${laterScene} accepted retries passed; full mappings/cleanup remain pending and 1.2.0 stays HOLD`);
  } else {
    for (let attempt = 0; attempt < 2; attempt++) {
      controller = runController(); const result = await controller.closed;
      assert.equal(result.code, 75, result.stderr); assert.match(result.stderr, /Pre-tag operator HOLD/);
      assert.equal(existsSync(path.join(records, "tag.fixture")), false);
      assert.equal(existsSync(path.join(records, "endpoint_publication.dispatch")), false);
      assert.equal((await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${nextIdentity.worker_job_index + 1}`)).status, 404);
    }
    // Only a fresh unpublished batch may replay the pending Local suffix on the
    // accepted Remote base. The old sealed package and Local receipts are fixed.
    const nextBatchAuthorMode = ait(["config", "show", "--json"]).json.default_author_mode;
    assert.equal(nextBatchAuthorMode, "ai_with_human_review",
      "the unpinned next batch must use its separately selected current author mode");
    const nextReady = launch(cli["--ait"], ["workflow", "ready", afterAcceptance.task_id,
      "--apply", "--remote", "origin", "--json"], repo);
    const nextJob = await waitFor(async () => {
      if (nextReady.code !== null) throw new Error(`next-batch Ready exited before real CI: ${nextReady.stdout}\n${nextReady.stderr}`);
      const response = await fetch(`${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs`);
      assert.equal(response.status, 200, "next-batch Job inventory must be readable");
      const inventory = await response.json();
      assert(Array.isArray(inventory.jobs), "next-batch Job inventory must be complete");
      const newer = inventory.jobs.filter(job => job.job_kind === 7 &&
        job.worker_job_index > nextIdentity.worker_job_index && !job.tombstoned);
      assert(newer.length <= 1, "next-batch Job inventory is ambiguous");
      if (!newer.length) return false;
      const jobUrl = `${url}/v1/native/repository-authorities/${identity.repository_index}/worker-jobs/${newer[0].worker_job_index}`;
      const jobResponse = await fetch(jobUrl);
      assert.equal(jobResponse.status, 200);
      const body = await jobResponse.json(), job = body.data ?? body;
      if (!job.package_execution) return false;
      const pending = packageIdentity(job.package_execution.expected_identity);
      assert.equal(pending.worker_job_index, nextIdentity.worker_job_index + 1,
        "only the first new Job may serve the next batch");
      assert.equal(pending.base_snapshot_id, acceptedRemoteHead);
      assert.notEqual(pending.package_hash, nextIdentity.package_hash);
      assert.notEqual(pending.revision_snapshot_id, nextIdentity.revision_snapshot_id);
      assert.equal(job.package_execution.author_mode, nextBatchAuthorMode,
        "fresh next-batch mode must follow the new unpinned preparation");
      assert.equal(job.package_execution.ci_state, "queued");
      assert.equal(job.job.patchset_index, null);
      return { identity: pending, jobUrl };
    }, "fresh next-batch Job awaiting actual CI", nativeWaitMs);
    assert.equal(nextReady.code, null, "next-batch Ready must wait for its own CI");
    runner = launch(cli["--runner"], ["run-job", "--server", url,
      "--repository-index", String(nextJob.identity.repository_index),
      "--worker-job-index", String(nextJob.identity.worker_job_index),
      "--source-root", repo, "--attempt-root", path.join(ram, "next-batch-runner-attempt")]);
    const nextRun = await runner.closed;
    assert.equal(nextRun.code, 0, `next-batch Runner failed: ${nextRun.stdout}\n${nextRun.stderr}`);
    await waitFor(() => nextReady.code !== null && nextReady, "next-batch Ready after real CI", nativeWaitMs);
    await nextReady.closed;
    assert.equal(nextReady.code, 0, `next-batch Ready failed after real CI: ${nextReady.stdout}\n${nextReady.stderr}`);
    const nextBatchIdentity = workflowResult(JSON.parse(nextReady.stdout), "ready");
    assert.deepEqual(nextBatchIdentity, nextJob.identity,
      "native Ready must select the exact next-batch Job whose CI ran");
    // Core retains the Local Snapshot for an aligned base and derives a new
    // Snapshot only when the Local suffix diverges from the accepted base.
    const aligned = snapshotDescendsFrom(afterAcceptance.snapshot_id, acceptedRemoteHead);
    assert.equal(nextBatchIdentity.revision_snapshot_id === afterAcceptance.snapshot_id, aligned,
      "aligned suffix must retain its source ID; divergent suffix must replay");
    assert.notEqual(nextBatchIdentity.revision_snapshot_id, nextIdentity.revision_snapshot_id);
    assert.equal(nextBatchIdentity.base_snapshot_id, acceptedRemoteHead,
      "fresh suffix replay must start from the accepted Remote head");
    assert.notEqual(nextBatchIdentity.package_hash, nextIdentity.package_hash);
    assert(nextBatchIdentity.worker_job_index > nextIdentity.worker_job_index);
    const replayed = ait(["snapshot", "show", nextBatchIdentity.revision_snapshot_id, "--json"]).json;
    let cursor = replayed, depth = 0;
    while (cursor.snapshot_id !== acceptedRemoteHead) {
      assert(depth++ < 32, "next-batch ancestry must reach the accepted Remote base");
      assert.equal(cursor.parent_snapshot_ids?.length, 1, "next batch must have one coherent source parent");
      cursor = ait(["snapshot", "show", cursor.parent_snapshot_ids[0], "--json"]).json;
    }
    for (const [name, text] of [["payload.txt", "candidate\n"], ...laterFiles,
      ["after.txt", "Local history after remote acceptance\n"]]) {
      const digest = createHash("sha256").update(text).digest("hex");
      assert(replayed.files.some(file => file.path === name && file.sha256 === digest),
        `fresh suffix replay omitted original or pending Local content: ${name}`);
    }
    assert.equal(originalLocalSnapshot(ait(["task", "show", source.task_id, "--local", "--json"]).json, source.task_id), identity.revision_snapshot_id);
    for (let i = 0; i < laterNames.length; i++) {
      assert.equal(completedLocalByTitle(laterNames[i]).snapshot_id, laterTasks[i].snapshot_id);
    }
    assert.equal(completedLocalByTitle("after-acceptance").snapshot_id, afterAcceptance.snapshot_id);
    assert.equal(ait(["line", "show", "main", "--json"]).json.head_snapshot_id, afterAcceptance.snapshot_id);
    const nextBatchResponse = await fetch(nextJob.jobUrl);
    assert.equal(nextBatchResponse.status, 200, "the first additional Job belongs only to the next batch");
    const nextBatchBody = await nextBatchResponse.json(), nextBatchJob = nextBatchBody.data ?? nextBatchBody;
    assert.deepEqual(packageIdentity(nextBatchJob.package_execution.expected_identity), nextBatchIdentity);
    assert.equal(nextBatchJob.package_execution.author_mode, nextBatchAuthorMode);
    assert.equal(nextBatchJob.package_execution.ci_state, "passed");
    assert.equal(nextBatchJob.job.patchset_index, null);
    assert.equal(ait(["line", "show", "main", "--remote", "origin", "--json"]).json.head_snapshot_id, acceptedRemoteHead,
      "fresh next-batch preparation must not publish or move Remote main");
    console.log(`actual restart/reprepare, ${laterScene}, read-only accepted retry and next-batch preparation passed; 1.2.0 remains pre-tag HOLD`);
  }
} catch (error) {
  keepFixture = true;
  process.stderr.write(`Native rehearsal failed/held; retained fixture=${scratch} RAM=${ram} original records=${records}\n`);
  throw error;
} finally {
  writeFileSync(gate, "release");
  const lock = path.join(records, ".release-operation.lock");
  if (existsSync(lock)) for (const name of readdirSync(lock)) {
    const owner = JSON.parse(readFileSync(path.join(lock, name)));
    if (owner.root === realpathSync(records)) { try { process.kill(-owner.group_pid, "SIGTERM"); } catch {} }
  }
  for (const child of [...children]) { try { child.kill("SIGTERM"); } catch {} }
  await stop(runner); await stop(controller); await stop(server);
  if (!keepFixture) { rmSync(scratch, { recursive: true, force: true }); rmSync(ram, { recursive: true, force: true }); }
}
