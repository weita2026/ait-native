#!/usr/bin/env node
// Isolated Task-start/optional current capture consumer integration. This emits no browser-evidence record
// and cannot qualify B–D, actual host readiness or the Server restart case.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { openNativeBrowser, runPersonalTaskStartBrowserScenario, runPersonalWorkflowBrowserAction } from "./release_web_browser_evidence.mjs";
import { currentControls, explicitAuthority, validateCaptureAuthority, validateCurrentDoctor } from "./release_web_controls.mjs";

const args = process.argv.slice(2);
assert.equal(args.length % 2, 0);
const input = Object.fromEntries(Array.from({ length: args.length / 2 }, (_, i) => args.slice(i * 2, i * 2 + 2)));
const scopes = input["--scope"] ? [input["--scope"]] : ["local", "remote"];
assert(scopes.every(scope => ["local", "remote"].includes(scope)), "scope must be Local or Remote");
for (const key of ["--ait", "--personal", "--server", "--chrome", "--ram-root", "--playwright", "--headless-helper"]) assert(path.isAbsolute(input[key] ?? ""), `explicit absolute ${key} is required`);
const diagnosticsDir = input["--diagnostics-dir"] ?? mkdtempSync(path.join(os.tmpdir(), "ait-native-browser-diagnostics."));
assert(path.isAbsolute(diagnosticsDir), "diagnostics directory must be absolute");
if (input["--diagnostics-dir"]) mkdirSync(diagnosticsDir);
const root = mkdtempSync(path.join(os.tmpdir(), "ait-native-browser-A."));
const ram = mkdtempSync(path.join(input["--ram-root"], "ait-native-browser-A."));
const repo = path.join(root, "repository"); mkdirSync(repo);
const bin = path.join(root, "bin"); mkdirSync(bin);
const fixtureNativeHome = path.join(root, "native-home"); mkdirSync(fixtureNativeHome);
const fixtureUserHome = path.join(root, "user-home"); mkdirSync(fixtureUserHome);
// Prevent real host bootstrap. This component test makes no host/role claim.
writeFileSync(path.join(bin, "codex"), "#!/bin/sh\nexit 69\n"); chmodSync(path.join(bin, "codex"), 0o755);
const env = { ...process.env, PATH: bin + path.delimiter + process.env.PATH, CODEX_HOME: fixtureNativeHome,
  HOME: fixtureUserHome, USERPROFILE: fixtureUserHome, XDG_CONFIG_HOME: path.join(fixtureUserHome, "config"),
  AIT_NATIVE_SERVER_CI_RAM_ROOT: ram, AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" };
delete env.CODEX_THREAD_ID; delete env.CODEX_SESSION_ID;
const children = [];
function launch(executable, argv, childEnv = env) {
  const child = spawn(executable, argv, { cwd: repo, env: childEnv, stdio: ["ignore", "pipe", "pipe"] });
  children.push(child);
  child.diagnostic = "";
  child.stderr.on("data", bytes => { child.diagnostic = (child.diagnostic + bytes).slice(-8000); });
  return child;
}
function cli(argv, json = true, cwd = repo) {
  const result = spawnSync(input["--ait"], argv, { cwd, env, encoding: "utf8", timeout: 30000, maxBuffer: 32 * 1024 * 1024 });
  assert.equal(result.status, 0, `${argv.join(" ")}: ${result.stdout}\n${result.stderr}`);
  return json ? JSON.parse(result.stdout) : result.stdout;
}
function legacyCiBaseline(cwd) {
  // Genuine legacy registration requires committed, repository-authored CI.
  // This disposable protocol baseline creates no review/CI acceptance claim.
  cli(["config", "set", "--default-author-mode", "ai_only_experimental", "--task-review", "automatic", "--json"], true, cwd);
  const card = "docs/sprints/ci-component.md", ref = "native/browser-ci/setup";
  mkdirSync(path.join(cwd, "docs/sprints"), { recursive: true });
  writeFileSync(path.join(cwd, card), `# Component CI baseline [plan-ref: native/browser-ci/root]\n\n- [ ] Configure the explicit component validation command. [ref: ${ref}]\n`);
  const started = cli(["task", "start", "--from", `${card}#${ref}`, "--intent", "Configure the isolated component CI baseline", "--local", "--json"], true, cwd);
  assert.match(started.task_id, /^[A-Z]+T-[0-9]+$/);
  const worktree = cli(["worktree", "show", started.task_id.toLowerCase(), "--json"], true, cwd);
  assert.equal(worktree.bound_task_id, started.task_id);
  mkdirSync(path.join(worktree.open_path, "ci"), { recursive: true });
  writeFileSync(path.join(worktree.open_path, "ci/check.mjs"), 'import assert from "node:assert/strict"; import fs from "node:fs"; assert.equal(fs.readFileSync("ci/fixture.txt", "utf8"), "component baseline\\n");\n');
  writeFileSync(path.join(worktree.open_path, "ci/fixture.txt"), "component baseline\n");
  writeFileSync(path.join(worktree.open_path, "ci/patch_ci.json"), JSON.stringify({ schema_version: 1, suites: [{
    suite_id: "browser_component", display_name: "Browser component fixture", plane: "patchset", mode: "gate", default_blocking: true,
    runner: { kind: "command_bundle", commands: ["node ci/check.mjs"], timeout_seconds: 30 },
  }] }));
  const finished = cli(["task", "finish", started.task_id, "--message", "Configure explicit component validation", "--local", "--json"], true, worktree.open_path);
  assert.equal(finished.task_id, started.task_id);
}
async function port() {
  const listener = net.createServer(); listener.listen(0, "127.0.0.1"); await once(listener, "listening");
  const value = listener.address().port; await new Promise(resolve => listener.close(resolve)); return value;
}
async function startServer(name) {
  const number = await port();
  const child = launch(input["--server"], ["--data", path.join(root, name), "--listen", `127.0.0.1:${number}`, "--init-if-missing", "--defer-ci-admission"]);
  child.stdout.resume();
  const url = `http://127.0.0.1:${number}`;
  for (let i = 0; i < 200; i++) {
    assert.equal(child.exitCode, null, child.diagnostic);
    try { if ((await fetch(url + "/healthz")).ok) return url; } catch {}
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error("isolated Server readiness timeout: " + child.diagnostic);
}
async function readyJson(child) {
  return new Promise((resolve, reject) => {
    let text = "";
    const timer = setTimeout(() => reject(new Error("Personal readiness timeout: " + child.diagnostic)), 30000);
    child.once("exit", code => { clearTimeout(timer); reject(new Error(`Personal exited ${code}: ${child.diagnostic}`)); });
    child.stdout.on("data", bytes => {
      text += bytes;
      try { const value = JSON.parse(text); assert.equal(value.ready, true); clearTimeout(timer); resolve(value); } catch {}
    });
  });
}
let browser, retainFixture = false;
try {
  cli(["init", "--json"]);
  let config = cli(["config", "show", "--json"]);
  const generation = Object.hasOwn(config, "workflow_mode") ? "legacy" : "current";
  const help = cli(["config", "set", "--help"], false);
  if (generation === "current") {
    currentControls(config); assert(help.includes("--agent-teams"));
    cli(["config", "set", "--agent-teams", "disabled", "--sprint", "on", "--json"]);
  } else {
    assert(["solo_local", "solo_remote"].includes(config.workflow_mode?.value));
    assert(help.includes("--workflow-mode"));
    cli(["config", "set", "--workflow-mode", "solo_local", "--sprint", "on", "--json"]);
  }
  cli(["config", "set", "--user-name", "Native browser A component fixture", "--user-email", "native-browser-fixture@example.invalid", "--json"]);
  if (generation === "legacy" && scopes.includes("remote")) {
    assert(help.includes("--default-author-mode") && help.includes("--task-review"));
    legacyCiBaseline(repo);
  }
  const artifact = "docs/sprints/direct_personal_action_cases.md";
  const item = "ait-web-test/direct-personal-action-cases/start-probe-";
  mkdirSync(path.join(repo, "docs/sprints"), { recursive: true });
  writeFileSync(path.join(repo, artifact), "# Browser A component [plan-ref: ait-web-test/direct-personal-action-cases/root]\n\n" +
    ["local", "remote"].map(scope => `- [ ] Start and abandon the ${scope} browser component Task. [ref: ${item}${scope}]\n`).join(""));
  cli(["plan", "sync", artifact, "--local", "--json"]);
  if (scopes.includes("remote")) {
  const primary = await startServer("default-server"), secondary = await startServer("selected-server");
  // Establish the same Repository name/index independently on the second
  // Server before attaching its configured row to the original client.
  const mirror = path.join(root, "mirror", "repository"); mkdirSync(mirror, { recursive: true });
  cli(["init", "--json"], true, mirror);
  if (generation === "current") cli(["config", "set", "--agent-teams", "disabled", "--json"], true, mirror);
  cli(["config", "set", "--user-name", "Native browser A component fixture", "--user-email", "native-browser-fixture@example.invalid", "--json"], true, mirror);
  if (generation === "legacy") legacyCiBaseline(mirror);
  cli(["remote", "add", "selected-B", secondary, "--default", "--json"], true, mirror);
  cli(["remote", "add", "default-A", primary, "--default", "--json"]);
  cli(["remote", "add", "selected-B", secondary, "--json"]);
  cli(["plan", "sync", artifact, "--remote", "selected-B", "--json"]);
  }
  config = cli(["config", "show", "--json"]);
  const doctorRun = spawnSync(input["--personal"], ["doctor", "--repository", repo, "--ait", input["--ait"], "--json"], { env, encoding: "utf8" });
  assert.equal(doctorRun.status, 0, doctorRun.stderr);
  const doctor = JSON.parse(doctorRun.stdout);
  if (generation === "current") validateCurrentDoctor(config, doctor);
  else {
    assert.equal(doctor.contract, "ait.web.personal.doctor.v1"); assert.equal(doctor.ok, true);
    assert.equal(doctor.repository.name, config.repo_name);
    assert.equal(doctor.repository.workflow_mode, config.workflow_mode.value);
    assert.equal(Object.hasOwn(doctor.repository, "workflow_default_scope"), false);
  }
  const app = launch(input["--personal"], ["serve", "--repository", repo, "--ait", input["--ait"], "--json"]);
  const ready = await readyJson(app);
  browser = await openNativeBrowser({ helper: input["--headless-helper"], playwrightModule: input["--playwright"],
    chrome: input["--chrome"], diagnosticsDir });
  const page = await browser.context.newPage();
  await page.goto(ready.launch_url, { waitUntil: "domcontentloaded" });
  const base = new URL(ready.launch_url).origin;
  if (input["--capture-message"]) assert.equal(generation, "current", "current capture cannot fall back to legacy");
  for (const scope of scopes) {
    const authority = explicitAuthority(scope, scope === "remote" ? "selected-B" : undefined);
    const selectedRepositoryIndex = scope === "remote"
      ? cli(["repo", "show", "--remote", "selected-B", "--json"]).repository.repository_index
      : config.repository_index;
    const result = await runPersonalTaskStartBrowserScenario({ page, base, authority,
      ait: input["--ait"], repository: repo, generation, source: `${artifact}#${item}${scope}`, diagnosticsDir, env,
      onCreated: input["--capture-message"] ? async ({ page, task, worktree, receipt }) => {
        writeFileSync(path.join(worktree.open_path, "native-browser-capture.txt"), "Native current workflow consumer fixture\n");
        const request = { contract: "ait.web.personal.workflow-mutation-request.v2", operation: "snapshot_create", task_id: task.task_id,
          scope: authority.scope, remote: authority.remote, change_ref: null, message: input["--capture-message"] };
        const captured = await runPersonalWorkflowBrowserAction({ page, base, request, generation, repositoryIndex: selectedRepositoryIndex,
          worktreeName: receipt.worktree.name, ait: input["--ait"], repository: repo, diagnosticsDir, env });
        const native = cli(["snapshot", "show", captured.body.authority.snapshot_id, "--json"]);
        assert.equal(native.message, request.message);
        const after = cli(["worktree", "show", receipt.worktree.name, "--json"]);
        assert.equal(after.head_snapshot_id, captured.body.authority.snapshot_id); assert.equal(after.bound_change_ref, null);
        console.log(`Native ${scope} current capture UI/readback completed; B–D qualification remains HOLD`);
      } : undefined });
    if (generation === "current") validateCaptureAuthority(result.worktree, authority, selectedRepositoryIndex);
  }
  if (scopes.includes("remote")) {
    assert.equal(cli(["task", "list", "--all", "--remote", "default-A", "--json"]).length, 0);
    const remoteTasks = cli(["task", "list", "--all", "--remote", "selected-B", "--json"]);
    assert.equal(remoteTasks.length, 1); assert.equal(remoteTasks[0].status, "abandoned");
  }
  console.log(`Native A component scenario (${generation}; ${scopes.join(",")}) completed real form/dialog/Inspect/cleanup; full browser qualification remains HOLD`);
} catch (error) {
  retainFixture = error.retainBrowserFixture === true;
  writeFileSync(path.join(diagnosticsDir, "native-browser.stderr"), error.stack + "\n", { mode: 0o600 });
  process.stderr.write(`Original browser diagnostics: ${diagnosticsDir}\n`);
  if (retainFixture) process.stderr.write(`Uncertain Task dispatch/cleanup; retained exact fixture ${root} and ${ram} for reconciliation\n`);
  throw error;
} finally {
  try { await browser?.close(); } finally {
  for (const child of children.reverse()) {
    if (child.exitCode !== null || child.signalCode !== null) continue;
    child.kill("SIGTERM");
    await Promise.race([once(child, "exit"), new Promise(resolve => setTimeout(resolve, 2000))]);
    if (child.exitCode === null && child.signalCode === null) { child.kill("SIGKILL"); await once(child, "exit"); }
  }
  if (!retainFixture) { rmSync(root, { recursive: true, force: true }); rmSync(ram, { recursive: true, force: true }); }
  }
}
