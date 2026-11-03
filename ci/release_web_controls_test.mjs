#!/usr/bin/env node
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { currentControls, unchangedSettings, validateCurrentDoctor, renderedControls, explicitAuthority,
  authorityArgv, scopedReadUrl, taskStartRequest, taskStartInspectLocation, taskStartOutcome, validateCaptureAuthority, webPersonalInScope, webCommunityInScope, validateWebOperation, webAcceptanceTree, validateBrowserRuntimeHead, validateWebBrowser, validateWebDirect } from "./release_web_controls.mjs";
import { Cdp, openNativeBrowser } from "./release_web_browser_evidence.mjs";

const browserSource = readFileSync(new URL("./release_web_browser_evidence.mjs", import.meta.url), "utf8");
assert(!/window\.(?:confirm|fetch)\s*=|\.requestSubmit\s*\(/.test(browserSource), "UI qualification must use actual controls and dialogs");
assert(!/runPersonalTaskStartBrowserScenario\(\{\s*cdp:/.test(browserSource), "the live adapter requires WT's Playwright page");
await assert.rejects(openNativeBrowser({ helper: "relative", playwrightModule: "relative" }), /absolute path/);

const config = { repo_name: "ait-web-test", repository_index: 23,
  workflow_default_scope: { workflow: { value: "local", source: "core_route" }, task: { value: "remote", source: "fixture_explicit" }, change: { value: "local", source: "core_route" } },
  agent_teams: true, sprint: { value: "on", enabled: true, source: "repo_config" },
  default_author_mode: "ai_with_human_review", default_model: null, task_review: { value: "automatic", source: "staged_default" } };
const copy = value => structuredClone(value);
assert.equal(currentControls(config).workflow_default_scope.task.value, "remote");
unchangedSettings(config, copy(config));
const disabled = copy(config); disabled.agent_teams = false;
assert.equal(currentControls(disabled).workflow_default_scope.task.value, "remote");
assert.throws(() => unchangedSettings(config, disabled));
const missing = copy(config); delete missing.default_model;
assert.throws(() => unchangedSettings(config, missing));
const changed = copy(config); changed.sprint.source = "different_source";
assert.throws(() => unchangedSettings(config, changed));
for (const malformed of [
  { ...config, workflow_mode: { value: "solo_remote" } },
  { ...config, workflow_default_scope: { value: "local", source: "guessed" } },
  { ...config, agent_teams: "enabled" },
  { ...config, sprint: { ...config.sprint, enabled: false } },
  { ...config, workflow_default_scope: { ...config.workflow_default_scope, change: { value: "unknown", source: "core_route" } } },
]) assert.throws(() => currentControls(malformed));
const doctor = { contract: "ait.web.personal.doctor.v2", ok: true, repository: { name: config.repo_name, repository_index: 23,
  workflow_default_scope: copy(config.workflow_default_scope), agent_teams: true, sprint: "on" } };
validateCurrentDoctor(config, doctor);
assert.throws(() => validateCurrentDoctor(config, { ...doctor, contract: "ait.web.personal.doctor.v1" }));
assert.throws(() => validateCurrentDoctor(config, { ...doctor, repository: { ...doctor.repository, workflow_mode: "solo_remote" } }));
assert.throws(() => validateCurrentDoctor(config, { ...doctor, repository: { ...doctor.repository, repository_index: 22 } }));
const definitions = { "Default workflow scope": "local (core_route)", "Default Task scope": "remote (fixture_explicit)",
  "Default Change scope": "local (core_route)", "Agent Teams": "on", Sprint: "on (repo_config)" };
assert.equal(renderedControls(definitions, config), true);
assert.equal(renderedControls({ ...definitions, "Default Task scope": "local (core_route)" }, config), false);
assert.equal(renderedControls({ ...definitions, "Agent Teams": "off" }, config), false);

const local = explicitAuthority("local");
const selected = explicitAuthority("remote", "selected&B");
assert.deepEqual(authorityArgv(local), ["--local"]);
assert.deepEqual(authorityArgv(selected), ["--remote", "selected&B"]);
validateCaptureAuthority({ capture_authority: null }, local, 23);
validateCaptureAuthority({ capture_authority: null }, local, null);
validateCaptureAuthority({ capture_authority: { remote_name: "selected&B", repository_index: 23 } }, selected, 23);
for (const proof of [undefined, null, { remote_name: "default-A", repository_index: 23 },
  { remote_name: "selected&B", repository_index: 24 }, { remote_name: "selected&B", repository_index: 23, url: "http://elsewhere" }]) {
  assert.throws(() => validateCaptureAuthority({ capture_authority: proof }, selected, 23));
}
assert.throws(() => validateCaptureAuthority({}, selected, 23));
assert.throws(() => validateCaptureAuthority({ capture_authority: { remote_name: "selected&B", repository_index: 23 } }, local, 23));
for (const args of [[undefined], ["remote"], ["remote", ""], ["local", "origin"], ["remote", "http://elsewhere"], ["remote", " origin"]]) {
  assert.throws(() => explicitAuthority(...args));
}
const read = new URL(scopedReadUrl("http://127.0.0.1:8080/", "/plans/PR-1?view=items", selected));
assert.equal(read.searchParams.get("remote"), "selected&B");
assert.equal(read.searchParams.get("view"), "items");
assert.equal(read.searchParams.get("scope"), "remote");
assert.equal(new URL(scopedReadUrl("http://127.0.0.1:8080/", "/plans/PR-1", explicitAuthority("remote", "team B"))).searchParams.get("remote"), "team B");
for (const route of ["//elsewhere/tasks/RWTT-1", "/plans/PR-1?scope=remote", "/plans/PR-1?remote=origin", "/tasks\\RWTT-1", "/plans/PR-1#local"]) {
  assert.throws(() => scopedReadUrl("http://127.0.0.1:8080/", route, selected));
}
const source = "docs/sprints/probe.md#probe/start";
const request = taskStartRequest(source, "exact browser invocation", selected);
assert.deepEqual(request, { source, intent: "exact browser invocation", scope: "remote", remote: "selected&B" });
assert.deepEqual(taskStartRequest(source, "local", local), { source, intent: "local", scope: "local" });
const location = "/tasks/RWTT-0001?scope=remote&remote=selected%26B";
const receipt = { contract: "ait.web.personal.task-start-receipt.v2", ok: true, operation: "task.start", source,
  intent: request.intent, title: "Probe", plan_item_ref: "probe/start", task_id: "RWTT-0001", change_ref: null,
  scope: "remote", remote: "selected&B", worktree: { name: "rwtt-0001", open_path: "/tmp/rwtt-0001", current_line: "feature/rwtt-0001", reused: false } };
const http = { status: 201, location, body: receipt };
assert.deepEqual(taskStartOutcome(http, "current", request), { created: true, taskId: receipt.task_id, inspectPath: location });
assert.throws(() => taskStartOutcome(http, "legacy", request));
const legacy = { ...receipt, contract: "ait.web.personal.task-start-receipt.v1", change_ref: "RWTT-0001/C-01" };
delete legacy.scope; delete legacy.remote;
assert.equal(taskStartOutcome({ ...http, body: legacy }, "legacy", request).created, true);
assert.throws(() => taskStartOutcome({ ...http, body: legacy }, "current", request));
for (const invalid of [undefined, "/tasks/RWTT-0001", "//elsewhere/tasks/RWTT-0001", "https://elsewhere/tasks/RWTT-0001", "/tasks/RWTT-0002?scope=remote&remote=selected%26B",
  "/tasks/RWTT-0001?scope=remote&remote=origin", "/tasks/RWTT-0001?scope=remote&remote=selected%26B&remote=selected%26B",
  "/tasks/RWTT-0001?scope=remote&remote=selected%26B&extra=1", "/tasks/RWTT-0001?scope=remote&remote=selected%26B#local",
  "/tasks/../tasks/RWTT-0001?scope=remote&remote=selected%26B"]) {
  assert.throws(() => taskStartInspectLocation(invalid, receipt.task_id, selected));
}
// Equal public IDs on another configured remote never redirect this flow.
assert.throws(() => taskStartInspectLocation(location, receipt.task_id, explicitAuthority("remote", "default-A")));
assert.equal(taskStartInspectLocation(location, receipt.task_id, selected), location);
for (const body of [{ ...receipt, change_ref: "RWTT-0001/C-01" }, { ...receipt, remote: "default-A" },
  { ...receipt, scope: "local" }, { ...receipt, worktree: { ...receipt.worktree, reused: true } }]) {
  assert.throws(() => taskStartOutcome({ ...http, body }, "current", request));
}
const reconciled = { status: 409, location, body: { contract: "ait.web.personal.task-start-error.v1", ok: false,
  operation: "task.start", automatic_retry: false,
  reconciliation: { status: "matched_task", plan_item_ref: "probe/start", matched_count: 1, matched_task: { task_id: receipt.task_id } } } };
assert.equal(taskStartOutcome(reconciled, "current", request).created, false);
for (const count of [0, 2]) assert.throws(() => taskStartOutcome({ ...reconciled, body: { ...reconciled.body,
  reconciliation: { ...reconciled.body.reconciliation, matched_count: count } } }, "current", request));
assert.throws(() => taskStartOutcome({ ...reconciled, location: null }, "current", request));
assert.throws(() => taskStartOutcome({ ...http, status: 409 }, "current", request));

const originalWebSocket = globalThis.WebSocket;
try {
  globalThis.WebSocket = class extends EventTarget {
    constructor() { super(); queueMicrotask(() => this.dispatchEvent(new Event("open"))); }
    send(raw) {
      const request = JSON.parse(raw);
      if (request.method === "late") setTimeout(() => this.dispatchEvent(new MessageEvent("message", {
        data: JSON.stringify({ id: request.id, result: {} }),
      })), 20);
    }
    close() { queueMicrotask(() => this.dispatchEvent(new Event("close"))); }
  };
  const transport = await new Cdp("ws://fixture.invalid").ready();
  await assert.rejects(transport.send("late", {}, 5), /timed out/);
  await new Promise(resolve => setTimeout(resolve, 30));
  const pending = transport.send("pending", {}, 1000);
  transport.close();
  await assert.rejects(pending, /closed/);
  assert.equal(transport.pending.size, 0);
} finally { globalThis.WebSocket = originalWebSocket; }

// Execute the actual driver's resume branch with its original uncertain log.
// No browser/Task-start subprocess may run or truncate that original output.
const temporary = mkdtempSync(path.join(os.tmpdir(), "ait-browser-resume-test."));
try {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const driver = readFileSync(path.join(here, "release_web_admission.sh"), "utf8");
  const direct = driver.slice(driver.indexOf("run_direct() {"), driver.indexOf("\nrun_randomized() {"));
  assert(direct.startsWith("run_direct() {"));

  // Execute the production collector on Bash 3.2. A failed command must stop
  // its own phase without suppressing eligible independent phases.
  const collectionHelpers = driver.slice(driver.indexOf("collection_failed=0\n"), driver.indexOf("run_native_direct() ("));
  const collectionGate = driver.slice(driver.indexOf('if [[ ${collection_failed} != 0 || ${collection_blocked} != 0 ]]; then'),
    driver.indexOf('\nif ! receipt_complete closeout; then\n  set_mode solo_remote'));
  assert(collectionHelpers.startsWith("collection_failed=0\n") && collectionGate.includes("exit 75"));
  const collectionFailures = [];
  for (const scenario of ["pass", "assertion", "unsafe", "review", "foundation"]) {
    try {
    const collectionRoot = path.join(temporary, "collection-" + scenario); mkdirSync(collectionRoot);
    const script = `set -euo pipefail
${collectionHelpers}
receipt_complete() { test -f "$collection_root/$1.pass"; }
print_status() { :; }
collection_preflight() {
  if [[ -f "$collection_root/unsafe" || -f "$collection_root/review" ]]; then return 75; fi
}
foundation_phase() {
  printf 'foundation\\n' >>"$collection_root/events"
  foundation_failed=0
  collect_foundation_check first /bin/bash -c 'printf "first\\n" >>"$collection_root/checks"; [[ "$collection_scenario" != foundation ]]'
  collect_foundation_check second /bin/bash -c 'printf "second\\n" >>"$collection_root/checks"'
  if [[ $foundation_failed != 0 ]]; then return 1; fi
  touch "$collection_root/foundation.pass"
}
local_phase() {
  printf 'local\\n' >>"$collection_root/events"
  if [[ $collection_scenario == unsafe ]]; then touch "$collection_root/unsafe"; fi
  if [[ $collection_scenario == assertion || $collection_scenario == unsafe ]]; then /bin/bash -c 'exit 17'; fi
  touch "$collection_root/after-local-command" "$collection_root/solo_local.pass"
}
remote_phase() {
  printf 'remote\\n' >>"$collection_root/events"
  if [[ $collection_scenario == review ]]; then touch "$collection_root/review"; return 75; fi
  touch "$collection_root/solo_remote.pass"
}
random_phase() { printf 'random\\n' >>"$collection_root/events"; touch "$collection_root/randomized_3000.pass"; }
collect_step foundation foundation_phase
collect_step solo_local local_phase
collect_step solo_remote remote_phase
collect_step randomized_3000 random_phase
${collectionGate}
touch "$collection_root/admitted"
`;
    const result = spawnSync("/bin/bash", ["-c", script], { encoding: "utf8", env: { ...process.env,
      collection_root: collectionRoot, collection_scenario: scenario } });
    const expected = scenario === "pass" ? 0 : scenario === "review" ? 75 : 1;
    assert.equal(result.status, expected, scenario + ": " + result.stderr);
    assert.deepEqual(readFileSync(path.join(collectionRoot, "checks"), "utf8").trim().split("\n"), ["first", "second"]);
    const events = readFileSync(path.join(collectionRoot, "events"), "utf8").trim().split("\n");
    assert.deepEqual(events, scenario === "foundation" ? ["foundation"] : scenario === "unsafe" ? ["foundation", "local"]
      : scenario === "review" ? ["foundation", "local", "remote"] : ["foundation", "local", "remote", "random"]);
    assert.equal(existsSync(path.join(collectionRoot, "admitted")), scenario === "pass");
    assert.equal(existsSync(path.join(collectionRoot, "after-local-command")), ["pass", "review"].includes(scenario));
    if (["unsafe", "review", "foundation"].includes(scenario)) assert.match(result.stderr, /BLOCKED/);    } catch (error) { collectionFailures.push(scenario + ": " + error.stack); }
  }
  // Exercise the production clean-authority predicates on their original
  // JSON shapes; a failed queue or status guard may never reach mutations.
  const preflightRoot = path.join(temporary, "collection-preflight"); mkdirSync(preflightRoot);
  const preflightAit = path.join(preflightRoot, "ait");
  writeFileSync(preflightAit, `#!${process.execPath}\nconst rows=JSON.parse(process.env.guard_rows);console.log(JSON.stringify(rows[process.argv[2]]));\n`, { mode: 0o755 });
  for (const variant of ["clean", "dirty", "review", "missing"]) {
    try {
      const status = { current_line: "main", workspace_dirty: variant === "dirty", workspace_changed_count: 0, worktree_name: null,
        worktree_hygiene: { manual_review_candidate_count: 0, stale_count: 0 }, reconciliation: { total_finding_count: 0 } };
      if (variant === "missing") delete status.reconciliation;
      const queue = { summary: { attention_required_count: 0, dirty_worktree_count: 0 }, remote: { reviewer_inbox: { count: variant === "review" ? 1 : 0 } } };
      const result = spawnSync("/bin/bash", ["-c", `set -euo pipefail
${collectionHelpers}
set_mode() { :; }
verify_binary() { :; }
curl() { :; }
personal_bin=''
community_bin=''
community_cli_bin=''
server_bin=''
collection_preflight
printf 'preflight accepted\\n'
`], { encoding: "utf8", env: { ...process.env, repository: preflightRoot, ait_bin: preflightAit,
        server_url: "http://127.0.0.1:1/", guard_rows: JSON.stringify({ status, queue }) } });
      assert.equal(result.status === 0, variant === "clean", variant + ": " + result.stderr);
      assert.equal(result.stdout.includes("preflight accepted"), variant === "clean");
    } catch (error) { collectionFailures.push("preflight/" + variant + ": " + error.stack); }
  }
  mkdirSync(path.join(temporary, "evidence"));
  const log = path.join(temporary, "solo_local-browser.log");
  const input = path.join(temporary, "input.json");
  writeFileSync(input, JSON.stringify({ browser_evidence: { solo_local: { path: "evidence/browser.json" } } }));
  const dispatch = path.join(temporary, "dispatched");
  const guardScript = `set -eu
receipt_complete() { return 1; }
set_mode() { :; }
node() { touch "$browser_guard_dispatch"; return 99; }
repo_root=fixture
community=1
ait_bin=fixture
personal_bin=fixture
community_bin=fixture
community_cli_bin=fixture
server_url=fixture
chrome=fixture
${direct}
run_direct solo_local solo_local solo-local local
`;
  for (const action of ["Task-start", "workflow"]) {
    const original = `Personal browser ${action} dispatch prepared; do not repeat this attempt\noriginal interrupted output\n`;
    writeFileSync(log, original);
    const result = spawnSync("bash", ["-c", guardScript], { encoding: "utf8", env: { ...process.env, input, output_dir: temporary, log_dir: temporary, browser_guard_dispatch: dispatch } });
  assert.equal(result.status, 75, result.stderr);
  assert.match(result.stderr, /exact-authority reconciliation/);
  assert.equal(readFileSync(log, "utf8"), original);
  assert.equal(existsSync(dispatch), false);

  }

  // The A-only producer must not emit current browser pass, even when its
  // current config/status/doctor admission succeeds. These protocol fixtures
  // never launch a product service or count as browser qualification.
  const browserConfig = { ...config, id_namespace_prefix: { value: "WT" }, default_line: "main", is_worktree: false };
  const browserStatus = { contract: "ait-agent-action/v1", command: "status", ok: true, repo_name: "ait-web-test",
    line_name: "main", head_snapshot_id: "SNP-111111111111", worktree: null, workspace: { dirty: false, changed_count: 0 } };
  const native = path.join(temporary, "ait.mjs");
  writeFileSync(native, `#!/usr/bin/env node\nconst result = process.argv[2] === 'config' ? ${JSON.stringify(browserConfig)} : ${JSON.stringify(browserStatus)}; process.stdout.write(JSON.stringify(result));\n`, { mode: 0o755 });
  const service = path.join(temporary, "personal.mjs");
  writeFileSync(service, `#!/usr/bin/env node\nimport fs from 'node:fs'; if (process.argv[2] === 'doctor') process.stdout.write(${JSON.stringify(JSON.stringify(doctor))}); else {fs.writeFileSync(${JSON.stringify(dispatch)}, 'unexpected service launch'); process.exit(99);}\n`, { mode: 0o755 });
  const browserOutput = path.join(temporary, "browser.json");
  const browser = spawnSync(process.execPath, [path.join(here, "release_web_browser_evidence.mjs"),
    "--repository", temporary, "--mode", "solo_local", "--scope", "local", "--ait", native,
    "--personal-bin", service, "--community-bin", service, "--community-cli-bin", service,
    "--server-url", "http://127.0.0.1:8088/", "--chrome", service, "--output", browserOutput],
  { encoding: "utf8", env: { ...process.env, AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" } });
  assert.equal(browser.status, 75, browser.stderr);
  assert.match(browser.stderr, /B–D browser execution is not yet integrated/);
  assert.equal(existsSync(browserOutput), false);
  assert.equal(existsSync(dispatch), false);

  // Exercise the actual retained input producer and verifier. These are
  // protocol fixtures, never live browser/product qualification.
  mkdirSync(path.join(temporary, "acceptance/manifests"), { recursive: true });
  for (const [name, rows] of [["ait-command-matrix.tsv", "one\ntwo\n"], ["community-surfaces.tsv", "a\nb\nc\n"], ["personal-surfaces.tsv", "p\n"]]) {
    writeFileSync(path.join(temporary, "acceptance/manifests", name), rows);
  }
  writeFileSync(path.join(temporary, "acceptance/check-mode-coverage.sh"), "#!/bin/sh\ngrep -q '^coverage\tpass$' \"$2\"\n");
  const candidateFile = path.join(temporary, "candidate.json"), componentsFile = path.join(temporary, "components.json");
  const release = { id: "REL-FAM-1111111111111111", version: "1.2.0", prior_version: "1.1.4", channel: "stable", tag: "v1.2.0" };
  const planRelease = { version: release.version, prior_version: release.prior_version, channel: release.channel, tag: release.tag };
  const candidate = { contract: "ait.release.operator.pre-tag-candidate-binding/v1", status: "ready_for_immutable_tag", release,
    candidate_authority: { sha256: "a".repeat(64), record: { prior_release: { version: "1.1.4", python_version: "1.1.4" } } } };
  writeFileSync(candidateFile, JSON.stringify(candidate));
  writeFileSync(componentsFile, JSON.stringify(Object.fromEntries(["core", "server", "runner", "python", "node", "web_test", "community"].map(key => [key, browserStatus.head_snapshot_id]))));
  const fullStatus = { ...browserStatus, current_line: "main", workspace_dirty: false, worktree_name: null,
    worktree_hygiene: { manual_review_candidate_count: 0, stale_count: 0 }, reconciliation: { total_finding_count: 0 } };
  const queue = { summary: { attention_required_count: 0, dirty_worktree_count: 0 }, remote: { reviewer_inbox: { count: 0 } } };
  const remote = { repository: { repository_index: 23, repository_name: "ait-web-test", namespace: "WT", tombstoned: false } };
  const communityRoot = path.join(temporary, "community-source"), communityStatusFile = path.join(temporary, "community-status.json");
  mkdirSync(communityRoot);
  const communityStatus = { ...browserStatus, repo_name: "ait-web-community" };
  writeFileSync(communityStatusFile, JSON.stringify(communityStatus));
  writeFileSync(native, `#!/usr/bin/env node\nimport fs from 'node:fs'; const rows = ${JSON.stringify({ config: browserConfig, status: fullStatus, queue, worktree: [], repo: remote })}; if (process.cwd() === fs.realpathSync(${JSON.stringify(communityRoot)})) { if (process.argv[2] !== 'status') process.exit(99); process.stdout.write(fs.readFileSync(${JSON.stringify(communityStatusFile)})); } else { if (!(process.argv[2] in rows)) process.exit(99); process.stdout.write(JSON.stringify(rows[process.argv[2]])); }\n`, { mode: 0o755 });
  const boundRoot = path.join(temporary, "bound"), boundInput = path.join(boundRoot, "input.json");
  mkdirSync(boundRoot);
  const initArgs = [path.join(here, "release_web_admission_init.mjs"), "--candidate", candidateFile, "--components", componentsFile,
    "--repository", temporary, "--ait", native, "--server-bin", service, "--community-root", communityRoot, "--community-bin", service, "--community-cli-bin", service,
    "--server-url", "http://127.0.0.1:8088/", "--seed", "42", "--output", boundInput];
  const environment = { ...process.env, AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" };
  const initialized = spawnSync(process.execPath, initArgs, { encoding: "utf8", env: environment });
  assert.equal(initialized.status, 0, initialized.stderr);
  for (const change of [
    value => { value.head_snapshot_id = "SNP-222222222222"; },
    value => { value.repo_name = "another-repository"; },
    value => { value.line_name = "feature/fixture"; },
    value => { value.worktree = { name: "fixture" }; },
    value => { value.workspace.dirty = true; value.workspace.changed_count = 1; },
    value => { delete value.workspace; },
  ]) {
    const source = copy(communityStatus); change(source); writeFileSync(communityStatusFile, JSON.stringify(source));
    const output = path.join(boundRoot, "rejected-source.json"), args = [...initArgs]; args[args.indexOf("--output") + 1] = output;
    const rejected = spawnSync(process.execPath, args, { encoding: "utf8", env: environment });
    assert.notEqual(rejected.status, 0); assert.match(rejected.stderr, /Community source/); assert(!existsSync(output));
  }
  writeFileSync(communityStatusFile, JSON.stringify(communityStatus));
  const retainedBrowser = spawnSync(process.execPath, [path.join(here, "release_web_browser_evidence.mjs"),
    "--repository", temporary, "--mode", "solo_local", "--scope", "local", "--ait", native,
    "--community-bin", service, "--community-cli-bin", service, "--server-url", "http://127.0.0.1:8088/",
    "--chrome", service, "--output", path.join(temporary, "retained-browser.json")], { encoding: "utf8", env: environment });
  assert.equal(retainedBrowser.status, 75, retainedBrowser.stderr);
  assert.match(retainedBrowser.stderr, /retained scope requires its original operation/);
  assert.equal(existsSync(dispatch), false);
  const bound = JSON.parse(readFileSync(boundInput, "utf8"));
  assert.equal(bound.components.community, communityStatus.head_snapshot_id);
  for (const value of [undefined, null, "", "SNP-INVALID"]) {
    const changed = copy(bound); changed.components.community = value;
    if (value === undefined) delete changed.components.community;
    assert.throws(() => webCommunityInScope(changed));
  }
  assert.equal(webPersonalInScope(bound), false); assert(!Object.hasOwn(bound.executables, "personal"));
  assert(!Object.hasOwn(bound.components, "personal")); assert.equal(existsSync(dispatch), false);
  assert.throws(() => webPersonalInScope({ ...bound, components: { ...bound.components, personal: "SNP-111111111111" } }));
  assert.throws(() => webPersonalInScope({ ...bound, executables: { ...bound.executables, personal: null } }));
  for (const value of [null, "", {}]) {
    assert.throws(() => webPersonalInScope({ ...bound, components: { ...bound.components, personal: value }, executables: { ...bound.executables, personal: value } }));
  }

  assert.throws(() => webPersonalInScope({ ...bound, release: { ...release, version: "1.1.4" } }));
  const partialComponents = JSON.parse(readFileSync(componentsFile, "utf8"));
  writeFileSync(componentsFile, JSON.stringify({ ...partialComponents, personal: "SNP-111111111111" }));
  const partialArgs = [...initArgs]; partialArgs[partialArgs.indexOf("--output") + 1] = path.join(boundRoot, "partial-input.json");
  const partialProduced = spawnSync(process.execPath, partialArgs, { encoding: "utf8", env: environment });
  assert.notEqual(partialProduced.status, 0); assert.equal(existsSync(partialArgs.at(-1)), false);
  writeFileSync(componentsFile, JSON.stringify(partialComponents));
  const fullComponentsFile = path.join(temporary, "full-components.json"), fullRoot = path.join(temporary, "full-bound");
  mkdirSync(fullRoot);
  writeFileSync(fullComponentsFile, JSON.stringify({ ...partialComponents, personal: browserStatus.head_snapshot_id }));
  const fullArgs = [...initArgs]; fullArgs[fullArgs.indexOf("--components") + 1] = fullComponentsFile;
  fullArgs[fullArgs.indexOf("--output") + 1] = path.join(fullRoot, "input.json");
  fullArgs.push("--personal-root", temporary, "--personal-bin", service);
  const fullProduced = spawnSync(process.execPath, fullArgs, { encoding: "utf8", env: environment });
  assert.equal(fullProduced.status, 0, fullProduced.stderr);
  const fullInput = JSON.parse(readFileSync(path.join(fullRoot, "input.json"), "utf8"));
  assert.equal(webPersonalInScope(fullInput), true);
  writeFileSync(candidateFile, JSON.stringify({ ...candidate, release: { ...release, prior_version: "1.2.0", version: "1.2.1", tag: "v1.2.1" } }));
  const nextRoot = path.join(temporary, "next-bound"); mkdirSync(nextRoot);
  const nextInput = path.join(nextRoot, "input.json"), nextArgs = [...fullArgs];
  nextArgs[nextArgs.indexOf("--output") + 1] = nextInput;
  const nextProduced = spawnSync(process.execPath, nextArgs, { encoding: "utf8", env: environment });
  assert.equal(nextProduced.status, 0, nextProduced.stderr);
  assert.equal(JSON.parse(readFileSync(nextInput)).components.community, communityStatus.head_snapshot_id);
  writeFileSync(candidateFile, JSON.stringify(candidate));
  // The opposite partial selection, executable without component, also rejects
  // at initialization rather than executing a downgraded branch.
  fullArgs[fullArgs.indexOf("--components") + 1] = componentsFile;
  fullArgs[fullArgs.indexOf("--output") + 1] = path.join(fullRoot, "partial.json");
  assert.notEqual(spawnSync(process.execPath, fullArgs, { encoding: "utf8", env: environment }).status, 0);
  assert.equal(existsSync(path.join(fullRoot, "partial.json")), false);
  const metrics = { binary_sha256: bound.executables.community.sha256, companion_binary_sha256: bound.executables.community_cli.sha256,
    browser_console: "pass", browser_desktop: "pass", "browser_mobile-390x844": "pass", console_warning_or_error_count: 0,
    failed_request_count: 0, external_request_count: 0, responsive: { desktop_max_overflow_px: 0, mobile_max_overflow_px: 0, document_horizontal_overflow: false },
    catalog: { definition_count: 25, enabled_count: 0, buttons: 0, forms: 0, executable_action_links: 0,
      filesystem_path_leak: false, repository_identity_leak: false, upstream_server_url_leak: false }, routes: ["/", "/actions"] };
  const browserEvidence = mode => ({ contract: "ait-web-test.browser-evidence.v1", mode, repository: { head_snapshot_id: browserStatus.head_snapshot_id, index: 23, name: "ait-web-test", namespace: "WT", line: "main", workspace: "clean" }, community: metrics });
  validateWebBrowser(browserEvidence("solo_local"), bound, "solo_local");
  assert.throws(() => validateWebBrowser({ ...browserEvidence("solo_local"), personal: {} }, bound, "solo_local"));
  assert.throws(() => validateWebBrowser({ ...browserEvidence("solo_local"), community: { ...metrics, failed_request_count: 1 } }, bound, "solo_local"));
  const { catalog: unusedCatalog, companion_binary_sha256: unusedCompanion, ...personalMetrics } = metrics;
  validateWebBrowser({ ...browserEvidence("solo_local"), personal: { ...personalMetrics, binary_sha256: fullInput.executables.personal.sha256, routes: ["/"] } }, fullInput, "solo_local");
  assert.throws(() => validateWebBrowser(browserEvidence("solo_local"), fullInput, "solo_local"));

  // A completed Local Task may advance main without changing release source.
  // Exercise the native reader boundary, including its complete argv. These
  // fixtures prove control behavior, never native history or B–D acceptance.
  const runtimeHead = "SNP-222222222222", headProofState = path.join(temporary, "head-proof.json"), headCalls = path.join(temporary, "head-calls.jsonl");
  const headReader = path.join(temporary, "head-reader.mjs");
  writeFileSync(headReader, `#!/usr/bin/env node
import fs from 'node:fs';
const state = JSON.parse(fs.readFileSync(${JSON.stringify(headProofState)}, 'utf8'));
const argv = process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(headCalls)}, JSON.stringify({ argv, cwd: process.cwd() }) + '\\n');
if (argv[1] === 'is-ancestor') process.exit(state.ancestorStatus ?? 0);
process.stdout.write(state.stdout ?? JSON.stringify(state.difference));
process.exit(state.diffStatus ?? 0);
`, { mode: 0o755 });
  const headInput = copy(fullInput);
  headInput.executables.ait = { path: headReader, sha256: createHash("sha256").update(readFileSync(headReader)).digest("hex") };
  const emptyDifference = { old_snapshot_id: browserStatus.head_snapshot_id, new_snapshot_id: runtimeHead,
    files: [], mode_changed: [], added: [], deleted: [], modified: [] };
  const writeHeadProof = state => writeFileSync(headProofState, JSON.stringify({ difference: emptyDifference, ...state }));
  validateBrowserRuntimeHead(browserStatus.head_snapshot_id, headInput);
  assert(!existsSync(headCalls), "equal heads must not require a replacement proof");
  writeHeadProof({});
  const equivalentEvidence = { ...browserEvidence("solo_local"), repository: { ...browserEvidence("solo_local").repository, head_snapshot_id: runtimeHead },
    personal: { ...personalMetrics, binary_sha256: headInput.executables.personal.sha256, routes: ["/"] } };
  const originalHeadInputText = JSON.stringify(headInput);
  validateWebBrowser(equivalentEvidence, headInput, "solo_local", temporary);
  assert.equal(JSON.stringify(headInput), originalHeadInputText, "runtime proof must retain the original input identity");
  assert.equal(equivalentEvidence.repository.head_snapshot_id, runtimeHead);
  assert.deepEqual(readFileSync(headCalls, "utf8").trim().split("\n").map(JSON.parse), [
    { argv: ["snapshot", "is-ancestor", browserStatus.head_snapshot_id, runtimeHead], cwd: realpathSync(temporary) },
    { argv: ["snapshot", "diff", browserStatus.head_snapshot_id, runtimeHead, "--json"], cwd: realpathSync(temporary) },
  ]);
  for (const field of ["files", "mode_changed", "added", "deleted", "modified"]) {
    writeHeadProof({ difference: { ...emptyDifference, [field]: [field === "files" ? { path: "fixture.txt", status: "modified" } : "fixture.txt"] } });
    assert.throws(() => validateBrowserRuntimeHead(runtimeHead, headInput, temporary), /runtime source/);
    const missing = copy(emptyDifference); delete missing[field]; writeHeadProof({ difference: missing });
    assert.throws(() => validateBrowserRuntimeHead(runtimeHead, headInput, temporary));
  }
  for (const state of [
    { ancestorStatus: 1 }, // Unrelated head.
    { ancestorStatus: 2 }, // Unknown head/storage failure.
    { diffStatus: 2 }, { stdout: "truncated {" },
    { difference: { ...emptyDifference, old_snapshot_id: runtimeHead } },
    { difference: { ...emptyDifference, new_snapshot_id: browserStatus.head_snapshot_id } },
  ]) {
    writeHeadProof(state);
    assert.throws(() => validateBrowserRuntimeHead(runtimeHead, headInput, temporary));
  }
  writeHeadProof({});
  for (const head of [undefined, "SNP-UNKNOWN", "SNP-222222222222\n"]) assert.throws(() => validateBrowserRuntimeHead(head, headInput, temporary));
  assert.throws(() => validateBrowserRuntimeHead(runtimeHead, headInput));
  assert.throws(() => validateBrowserRuntimeHead(runtimeHead, bound, temporary), /full Personal lifecycle/);
  const alteredSource = copy(headInput); alteredSource.components.web_test = runtimeHead;
  assert.throws(() => validateBrowserRuntimeHead(runtimeHead, alteredSource, temporary), /original Web source identity/);
  const alteredReader = copy(headInput); alteredReader.executables.ait.sha256 = "0".repeat(64);
  assert.throws(() => validateBrowserRuntimeHead(runtimeHead, alteredReader, temporary), /reader changed/);
  const unavailableReader = copy(headInput); unavailableReader.executables.ait.path += ".missing";
  assert.throws(() => validateBrowserRuntimeHead(runtimeHead, unavailableReader, temporary));
  const dirtyEvidence = copy(equivalentEvidence); dirtyEvidence.repository.workspace = "dirty";
  assert.throws(() => validateWebBrowser(dirtyEvidence, headInput, "solo_local", temporary));

  const wtChecksum = spawnSync("cksum", [], { input: "WT authority inventory fixture\n", encoding: "utf8" });
  assert.equal(wtChecksum.status, 0, wtChecksum.stderr);
  const wtAuthorityDigest = wtChecksum.stdout.trim().split(/\s+/).join(":");
  // Preserve the real foundation producer's shape instead of a synthetic SHA digest.
  const summary = phase => ({ contract: "ait-web-test.direct-run.v1", mode: phase === "solo-local" ? "solo_local" : "solo_remote", phase, status: "pass",
    initial_authority_digest: "1140419897:479662", final_authority_digest: "1140419897:479662", command_nodes: 2, personal_surfaces: phase === "inventory" ? 1 : 0, community_surfaces: 3 });
  for (const digest of [wtAuthorityDigest, "0:0", "4294967295:0", "4294967295:18446744073709551615"]) {
    validateWebDirect({ ...summary("inventory"), initial_authority_digest: digest, final_authority_digest: digest }, bound, "inventory", temporary);
  }
  for (const field of ["initial_authority_digest", "final_authority_digest"]) {
    for (const digest of [undefined, null, 1, [], {}, "", "b".repeat(64), "1", "1:1:1", "-1:1", "1:-1", "+1:1", "1:+1",
      "01:1", "1:01", "1.0:1", "1:1.0", "1e3:1", "1:1e3", " 1:1", "1:1 ", "1:1\n", "1:1\r", "1:1\u2028",
      "4294967296:1", "1:18446744073709551616"]) {
      assert.throws(() => validateWebDirect({ ...summary("inventory"), [field]: digest }, bound, "inventory", temporary));
    }
  }
  assert.throws(() => validateWebDirect({ ...summary("inventory"), final_authority_digest: "1140419898:479662" }, bound, "inventory", temporary));
  for (const phase of ["solo-local", "solo-remote"]) {
    validateWebDirect({ ...summary(phase), final_authority_digest: "1140419898:479662" }, bound, phase, temporary);
  }
  validateWebDirect(summary("solo-local"), bound, "solo-local", temporary);
  assert.throws(() => validateWebDirect({ ...summary("solo-local"), community_surfaces: 2 }, bound, "solo-local", temporary));
  assert.throws(() => validateWebDirect(summary("solo-local"), fullInput, "solo-local", temporary));
  validateWebDirect({ ...summary("solo-local"), personal_surfaces: 1 }, fullInput, "solo-local", temporary);
  const hash = file => createHash("sha256").update(readFileSync(file)).digest("hex");
  writeFileSync(path.join(boundRoot, "cases.tsv"), "inventory.ait\tpass\ninventory.products\tpass\ninventory.authority-unchanged\tpass\ncoverage\tpass\n");
  const inputHash = hash(boundInput), admissionFile = path.join(boundRoot, "web-admission.json");
  // The new scoped input reuses the existing native conductor binding. No
  // separate scope receipt or schema is introduced.
  const operationPlan = { contract: "ait.release.conductor-plan/v1", records_root: temporary, release: planRelease,
    phases: [{ id: "web_admission", actions: [{ id: "bind-web-admission-input", type: "command", argv: [process.execPath, ...initArgs], outputs: [boundInput] }] }] };
  const planFile = path.join(temporary, "conductor-plan.json"), stateFile = path.join(temporary, "conductor-state.json");
  writeFileSync(planFile, JSON.stringify(operationPlan));
  const inputOutputDigest = createHash("sha256").update("f\0.\0" + (0o400) + "\0").update(readFileSync(boundInput)).digest("hex");
  const operationState = { contract: "ait.release.conductor-state/v1", plan_sha256: hash(planFile), release: planRelease,
    phases: [{ id: "web_admission", actions: [{ id: "bind-web-admission-input", status: "complete", outputs: { [boundInput]: inputOutputDigest } }] }] };
  writeFileSync(stateFile, JSON.stringify(operationState));
  validateWebOperation(boundInput, bound, temporary);

  // Exercise the release roles independently. Updating the isolated fixture's
  // input digest lets each rejection reach the prior-binding guard itself.
  const originalCandidate = readFileSync(candidateFile), originalPlan = readFileSync(planFile), originalState = readFileSync(stateFile);
  const originalBoundInput = readFileSync(boundInput);
  const priorCase = (change, accepted = false) => {
    const value = structuredClone(bound), selected = structuredClone(candidate), plan = structuredClone(operationPlan);
    change(value, selected, plan);
    rmSync(boundInput); writeFileSync(boundInput, JSON.stringify(value), { mode: 0o400 });
    writeFileSync(candidateFile, JSON.stringify(selected)); writeFileSync(planFile, JSON.stringify(plan));
    const state = structuredClone(operationState); state.release = plan.release; state.plan_sha256 = hash(planFile);
    state.phases[0].actions[0].outputs[boundInput] = createHash("sha256").update("f\0.\0" + (0o400) + "\0").update(readFileSync(boundInput)).digest("hex");
    writeFileSync(stateFile, JSON.stringify(state));
    if (accepted) validateWebOperation(boundInput, value, temporary);
    else assert.throws(() => validateWebOperation(boundInput, value, temporary));
    rmSync(boundInput); writeFileSync(boundInput, originalBoundInput, { mode: 0o400 });
    writeFileSync(candidateFile, originalCandidate); writeFileSync(planFile, originalPlan); writeFileSync(stateFile, originalState);
  };
  priorCase(() => {}, true);
  priorCase((value, selected) => { delete value.release.prior_version; delete selected.release.prior_version; }, true);
  for (const change of [
    (value, selected) => { delete value.release.prior_version; },
    (value, selected) => { delete selected.release.prior_version; },
    value => { value.release.prior_version = "1.1.3"; },
    (value, selected) => { selected.release.prior_version = "1.1.3"; },
    (value, selected) => { value.release.prior_version = selected.release.prior_version = "1.1.3"; },
    (value, selected) => { value.release.prior_version = selected.release.prior_version = null; },
    (value, selected) => { delete selected.candidate_authority; },
    (value, selected) => { delete selected.candidate_authority.record; },
    (value, selected) => { delete selected.candidate_authority.record.prior_release; },
    (value, selected) => { delete selected.candidate_authority.record.prior_release.version; },
    (value, selected) => { selected.candidate_authority.record.prior_release.version = null; },
    (value, selected, plan) => { plan.release.prior_version = "1.1.3"; },
    (value, selected) => { delete value.release.prior_version; delete selected.release.prior_version; selected.candidate_authority.record.prior_release.version = "1.1.3"; },
    (value, selected) => { delete value.release.prior_version; delete selected.release.prior_version; delete selected.candidate_authority; },
    (value, selected, plan) => { value.release.prior_version = selected.release.prior_version = plan.release.prior_version = selected.candidate_authority.record.prior_release.version = "1.2.0"; },
    (value, selected, plan) => { value.release.prior_version = selected.release.prior_version = plan.release.prior_version = selected.candidate_authority.record.prior_release.version = "1.0.4"; },
    (value, selected, plan) => { delete value.release.prior_version; delete selected.release.prior_version; plan.release.prior_version = selected.candidate_authority.record.prior_release.version = "invalid"; },
  ]) priorCase(change);
  const completedArgs = [path.join(here, "release_web_admission.sh"), "run", "--input", boundInput,
    "--repository", temporary, "--ait", native, "--server-bin", service, "--server-data", temporary,
    "--community-bin", service, "--community-cli-bin", service, "--server-url", "http://127.0.0.1:8088/",
    "--chrome", service, "--output-dir", boundRoot];
  const resumeCompleted = () => spawnSync("/bin/bash", completedArgs, { encoding: "utf8", env: environment });

  const originalInputBytes = readFileSync(boundInput);
  const replaceInput = value => { rmSync(boundInput); writeFileSync(boundInput, value, { mode: 0o400 }); };
  replaceInput(JSON.stringify({ ...bound, execution: { ...bound.execution, random_seed: 43 } }));
  const firstExecutionTamper = resumeCompleted();
  assert.notEqual(firstExecutionTamper.status, 0);
  assert.match(firstExecutionTamper.stderr, /original qualification input digest changed/);
  assert.equal(existsSync(dispatch), false, "changed input cannot start Server before the first receipt");
  replaceInput(originalInputBytes);
  validateWebOperation(boundInput, bound, temporary);


  const save = (file, value) => writeFileSync(file, JSON.stringify(value));
  mkdirSync(path.join(boundRoot, "receipts"), { recursive: true });
  const evidenceFiles = {};
  function receipt(step, file) {
    save(path.join(boundRoot, "receipts", `${step}.json`), { contract: "ait.release.web-admission-step/v1", status: "pass", step,
      input_sha256: inputHash, evidence: { path: file, sha256: hash(file) } });
  }
  for (const [step, phase] of [["foundation", "inventory"], ["solo_local", "solo-local"], ["solo_remote", "solo-remote"]]) {
    const file = path.join(boundRoot, `${step}.json`); save(file, summary(phase)); evidenceFiles[step] = file; receipt(step, file);
  }
  const browserHashes = {};
  for (const mode of ["solo_local", "solo_remote"]) {
    const file = path.join(boundRoot, bound.browser_evidence[mode].path); save(file, browserEvidence(mode)); browserHashes[`${mode}_sha256`] = hash(file);
  }
  const randomFile = path.join(boundRoot, "random.json");
  const random = { contract: "ait-web-test.random-command-run.v1", status: "pass", mode: "solo_remote", repository_index: 23, namespace: "WT",
    seed: 42, requested_groups: 3000, executed_groups: 3000, retirement_invocations: 0, initial_authority_digest: wtAuthorityDigest, final_authority_digest: wtAuthorityDigest };
  save(randomFile, random); receipt("randomized_3000", randomFile);
  save(admissionFile, { contract: "ait.release.web-admission/v1", status: "admitted", input_sha256: inputHash, release_id: release.id,
    browser: browserHashes, direct: { solo_local: "pass", solo_remote: "pass" }, randomized: { seed: 42, groups: 3000, status: "pass" } });
  receipt("closeout", admissionFile);
  const verify = () => spawnSync(process.execPath, [path.join(here, "release_web_admission_verify.mjs"), boundInput, admissionFile, temporary], { encoding: "utf8" });
  assert.equal(verify().status, 0, verify().stderr);
  save(randomFile, { ...random, executed_groups: 2999 }); receipt("randomized_3000", randomFile);
  assert.notEqual(verify().status, 0); save(randomFile, random); receipt("randomized_3000", randomFile);
  save(evidenceFiles.solo_local, { ...summary("solo-local"), personal_surfaces: 1 }); receipt("solo_local", evidenceFiles.solo_local);
  assert.notEqual(verify().status, 0);
  save(evidenceFiles.solo_local, summary("solo-local")); receipt("solo_local", evidenceFiles.solo_local);

  // Execute the real driver's completed-closeout branch, rather than only
  // its standalone verifier. Neither success nor HOLD may replay a lifecycle.
  const completed = resumeCompleted();
  assert.equal(completed.status, 0, completed.stderr);
  assert.match(completed.stdout, /closeout: complete/);
  assert.equal(existsSync(dispatch), false);
  const statusArgs = [...completedArgs]; statusArgs[1] = "status";
  const completedStatus = () => spawnSync("/bin/bash", statusArgs, { encoding: "utf8", env: environment });
  const validStatus = completedStatus(); assert.equal(validStatus.status, 0, validStatus.stderr);
  assert.match(validStatus.stdout, /closeout: complete/);
  const closeoutReceipt = path.join(boundRoot, "receipts/closeout.json"), closeoutBytes = readFileSync(closeoutReceipt);
  rmSync(closeoutReceipt);
  const partialStatus = completedStatus(); assert.equal(partialStatus.status, 0, partialStatus.stderr);
  assert.match(partialStatus.stdout, /closeout: pending/); assert.equal(existsSync(dispatch), false);
  writeFileSync(closeoutReceipt, closeoutBytes);
  save(randomFile, { ...random, executed_groups: 2999 }); receipt("randomized_3000", randomFile);
  const statusArtifacts = [boundInput, admissionFile, randomFile, path.join(boundRoot, "receipts/randomized_3000.json"), closeoutReceipt];
  const statusHashes = statusArtifacts.map(hash), shortStatus = completedStatus();
  assert.equal(shortStatus.status, 75, "completed status must fully verify its 3,000 groups");
  assert(!shortStatus.stdout.includes("closeout: complete"));
  assert.deepEqual(statusArtifacts.map(hash), statusHashes); assert.equal(existsSync(dispatch), false);
  save(randomFile, random); receipt("randomized_3000", randomFile);
  for (const tampered of [{ ...bound, execution: { ...bound.execution, random_seed: 43 } },
    { ...bound, qualification: { status: "foreign" } }, { ...bound, release: { ...bound.release, version: "1.1.4" } }]) {
    replaceInput(JSON.stringify(tampered));
    const result = resumeCompleted(); assert.notEqual(result.status, 0);
    assert.equal(existsSync(dispatch), false); assert.equal(existsSync(path.join(temporary, ".ait-runtime/direct-test-results")), false);
  }
  replaceInput(originalInputBytes);
  // A valid old full-Web standalone binding cannot be downgraded by removing
  // its Personal pair, even though the edited document still says 1.2.0.
  const downgradeRoot = temporary + "-downgrade";
  mkdirSync(path.join(downgradeRoot, "receipts"), { recursive: true });
  try {
    const oldInput = path.join(downgradeRoot, "input.json"), oldSummary = path.join(downgradeRoot, "foundation.json");
    const oldCases = path.join(downgradeRoot, "cases.tsv");
    writeFileSync(oldInput, JSON.stringify(fullInput), { mode: 0o400 });
    save(oldSummary, summary("inventory"));
    writeFileSync(oldCases, "inventory.ait\tpass\ninventory.products\tpass\ninventory.authority-unchanged\tpass\n");
    const oldReceipt = path.join(downgradeRoot, "receipts/foundation.json");
    save(oldReceipt, { contract: "ait.release.web-admission-step/v1", status: "pass", step: "foundation", input_sha256: hash(oldInput),
      evidence: { path: oldSummary, sha256: hash(oldSummary) } });
    validateWebOperation(oldInput, fullInput, temporary);
    const deletedPair = copy(fullInput); delete deletedPair.executables.personal; delete deletedPair.components.personal;
    rmSync(oldInput); writeFileSync(oldInput, JSON.stringify(deletedPair), { mode: 0o400 });
    const oldReceiptHash = hash(oldReceipt), oldSummaryHash = hash(oldSummary);
    const downgradeArgs = [...completedArgs]; downgradeArgs[downgradeArgs.indexOf("--input") + 1] = oldInput;
    downgradeArgs[downgradeArgs.indexOf("--output-dir") + 1] = downgradeRoot;
    const downgraded = spawnSync("/bin/bash", downgradeArgs, { encoding: "utf8", env: environment });
    assert.notEqual(downgraded.status, 0); assert.match(downgraded.stderr, /another immutable input/);
    assert.equal(hash(oldReceipt), oldReceiptHash); assert.equal(hash(oldSummary), oldSummaryHash); assert.equal(existsSync(dispatch), false);
  } finally { rmSync(downgradeRoot, { recursive: true, force: true }); }
  const caseFile = path.join(boundRoot, "cases.tsv"), originalCases = readFileSync(caseFile, "utf8");
  writeFileSync(caseFile, originalCases.replace("coverage\tpass\n", "")); assert.notEqual(verify().status, 0);
  const originalArtifacts = [boundInput, admissionFile, caseFile, randomFile, ...Object.values(evidenceFiles),
    ...["foundation", "solo_local", "solo_remote", "randomized_3000", "closeout"].map(step => path.join(boundRoot, "receipts", `${step}.json`))];
  const retainedHashes = originalArtifacts.map(hash);
  const missingCoverage = resumeCompleted();
  assert.notEqual(missingCoverage.status, 0, "completed resume must revalidate retained coverage");
  assert(!missingCoverage.stdout.includes("closeout: complete"));
  const missingCoverageStatus = completedStatus(); assert.equal(missingCoverageStatus.status, 75);
  assert(!missingCoverageStatus.stdout.includes("closeout: complete"));

  assert.deepEqual(originalArtifacts.map(hash), retainedHashes, "HOLD must preserve the existing artifacts");
  assert.equal(existsSync(dispatch), false, "completed resume must never start a service or lifecycle");
  assert.equal(existsSync(path.join(temporary, ".ait-runtime/direct-test-results")), false);

  writeFileSync(caseFile, originalCases + "personal.synthetic\tpass\n"); assert.notEqual(verify().status, 0);
  writeFileSync(caseFile, originalCases);
  writeFileSync(path.join(temporary, "acceptance/manifests/community-surfaces.tsv"), "a\nb\n");
  assert.notEqual(verify().status, 0);

  // Execute the actual direct-call assembly under macOS Bash 3.2, including
  // its nounset behavior for an empty optional argument array.
  const argvCapture = path.join(temporary, "retained.argv"), serverCapture = path.join(temporary, "retained.server");
  writeFileSync(path.join(temporary, "acceptance/run.sh"), `#!/bin/sh\nprintf '%s\\n' "$@" >"$test_capture"\nprintf '%s' "$AIT_WEB_TEST_SERVER_BIN" >"$test_server_capture"\n`, { mode: 0o755 });
  const runScript = `set -euo pipefail
receipt_complete() { return 1; }
set_mode() { :; }
validate_web_evidence() { :; }
archive_latest_direct_result() { :; }
write_receipt() { :; }
repo_root=fixture
community=1
ait_bin=fixture
personal_bin=''
personal_args=()
community_bin=fixture
community_cli_bin=fixture
server_bin=/exact/bound-server
server_url=fixture
chrome=fixture
result_archive=fixture
${direct}
run_direct solo_local solo_local solo-local local
`;
  const invoked = spawnSync("/bin/bash", ["-c", runScript], { encoding: "utf8", env: { ...process.env, repository: temporary,
    input: boundInput, output_dir: boundRoot, log_dir: temporary, test_capture: argvCapture, test_server_capture: serverCapture } });
  assert.equal(invoked.status, 0, invoked.stderr);
  assert(!readFileSync(argvCapture, "utf8").includes("--personal-bin"));
  assert.equal(readFileSync(serverCapture, "utf8"), "/exact/bound-server");
  const repeated = spawnSync("/bin/bash", ["-c", runScript], { encoding: "utf8", env: { ...process.env, repository: temporary,
    input: boundInput, output_dir: boundRoot, log_dir: temporary, test_capture: argvCapture, test_server_capture: serverCapture } });
  assert.equal(repeated.status, 75, repeated.stderr);
  assert.match(repeated.stderr, /existing WT phase is incomplete or ambiguous/);

  // Paused scope uses real input/operation/receipt validators. All pass-shaped
  // documents below are isolated protocol fixtures, never release evidence.
  const pausedRoot = path.join(temporary, "paused"), pausedInput = path.join(pausedRoot, "input.json");
  mkdirSync(pausedRoot);
  writeFileSync(path.join(temporary, "acceptance/manifests/ait-command-matrix.tsv"), "task\tnamespace\ntask show\tleaf\n");
  // No Community-only manifest or browser JSON is an input to paused checks.
  rmSync(path.join(temporary, "acceptance/manifests/community-surfaces.tsv"));
  rmSync(path.join(temporary, "acceptance/manifests/personal-surfaces.tsv"));
  const withoutCommunity = args => args.filter((value, index) => !["--community-root", "--community-bin", "--community-cli-bin"].includes(value) && !["--community-root", "--community-bin", "--community-cli-bin"].includes(args[index - 1]));
  const pausedArgs = withoutCommunity(initArgs); pausedArgs[pausedArgs.indexOf("--output") + 1] = pausedInput;
  const pausedComponentsFile = path.join(pausedRoot, "components.json"), pausedComponents = copy(partialComponents);
  delete pausedComponents.community; writeFileSync(pausedComponentsFile, JSON.stringify(pausedComponents));
  pausedArgs[pausedArgs.indexOf("--components") + 1] = pausedComponentsFile;
  const pausedProduced = spawnSync(process.execPath, pausedArgs, { encoding: "utf8", env: environment });
  assert.equal(pausedProduced.status, 0, pausedProduced.stderr);
  const paused = JSON.parse(readFileSync(pausedInput));
  assert.equal(webCommunityInScope(paused), false); assert.deepEqual(paused.browser_evidence, {});
  assert.deepEqual(Object.keys(paused.executables).sort(), ["ait", "server"]);
  assert.throws(() => validateWebBrowser(browserEvidence("solo_local"), paused, "solo_local"));
  for (const key of ["community", "community_cli"]) {
    assert.throws(() => webCommunityInScope({ ...paused, executables: { ...paused.executables, [key]: bound.executables[key] } }));
  }
  for (const invalid of [null, {}, { path: "relative", sha256: "a".repeat(64) }, { path: service, sha256: "bad" }]) {
    assert.throws(() => webCommunityInScope({ ...paused, executables: { ...paused.executables, community: invalid, community_cli: invalid } }));
  }
  for (const release of [{ version: "1.2.1", channel: "stable" }, { version: "1.2.0", channel: "preview" }]) {
    assert.throws(() => webCommunityInScope({ ...paused, release }));
  }
  const missingFullArgs = withoutCommunity(fullArgs);
  missingFullArgs[missingFullArgs.indexOf("--components") + 1] = fullComponentsFile;
  missingFullArgs[missingFullArgs.indexOf("--output") + 1] = path.join(pausedRoot, "missing-full.json");
  assert.notEqual(spawnSync(process.execPath, missingFullArgs, { encoding: "utf8", env: environment }).status, 0);
  assert(!existsSync(path.join(pausedRoot, "missing-full.json")));

  const pausedPlan = copy(operationPlan); pausedPlan.phases[0].actions[0].argv = [process.execPath, ...pausedArgs];
  pausedPlan.phases[0].actions[0].outputs = [pausedInput];
  save(planFile, pausedPlan);
  const pausedState = copy(operationState); pausedState.plan_sha256 = hash(planFile);
  const pausedDigest = () => createHash("sha256").update(`f\0.\0${0o400}\0`).update(readFileSync(pausedInput)).digest("hex");
  pausedState.phases[0].actions[0].outputs = { [pausedInput]: pausedDigest() }; save(stateFile, pausedState);
  validateWebOperation(pausedInput, paused, temporary);
  assert.throws(() => validateWebOperation(pausedInput, { ...paused, browser_evidence: bound.browser_evidence }, temporary));
  const nativeCases = phase => ["inventory.ait", "ait.help.task", "ait.help.task.show", "ait.invalid.task.show", "ait.semantic.task.show",
    "native.current.reviewed-fork", `final.${phase}-mode`, ...(phase === "solo-remote" ? ["native.current.two-authorities", "native.current.formal-finish",
      "remote.lifecycle.workflow-finish", "remote.lifecycle.verify-land"] : [])];
  const pausedSummary = phase => ({ ...summary(phase), community_surfaces: 0, personal_surfaces: 0 });
  mkdirSync(path.join(pausedRoot, "receipts"));
  const nativeFiles = {};
  const nativeReceipt = (step, file) => save(path.join(pausedRoot, "receipts", `${step}.json`), {
    contract: "ait.release.web-admission-step/v1", status: "pass", step, input_sha256: hash(pausedInput), evidence: { path: file, sha256: hash(file) } });
  for (const [step, phase] of [["foundation", "inventory"], ["solo_local", "solo-local"], ["solo_remote", "solo-remote"]]) {
    const result = path.join(pausedRoot, step); mkdirSync(result);
    const cases = phase === "inventory" ? ["inventory.ait", "inventory.authority-unchanged"] : nativeCases(phase);
    writeFileSync(path.join(result, "cases.tsv"), cases.map(id => `${id}\tpass\n`).join(""));
    save(path.join(result, "direct-run.config-original.json"), config);
    const file = path.join(result, "summary.json"); save(file, pausedSummary(phase)); nativeFiles[step] = file; nativeReceipt(step, file);
    validateWebDirect(pausedSummary(phase), paused, phase, temporary, result);
    if (phase !== "inventory") {
      for (const missing of cases) {
        writeFileSync(path.join(result, "cases.tsv"), cases.filter(id => id !== missing).map(id => `${id}\tpass\n`).join(""));
        assert.throws(() => validateWebDirect(pausedSummary(phase), paused, phase, temporary, result), undefined, missing);
      }
      writeFileSync(path.join(result, "cases.tsv"), cases.map(id => `${id}\tpass\n`).join(""));
      for (const row of ["community.synthetic\tpass\n", "personal.synthetic\tpass\n", "native.failed\tfail\n"]) {
        writeFileSync(path.join(result, "cases.tsv"), cases.map(id => `${id}\tpass\n`).join("") + row);
        assert.throws(() => validateWebDirect(pausedSummary(phase), paused, phase, temporary, result));
      }
      writeFileSync(path.join(result, "cases.tsv"), cases.map(id => `${id}\tpass\n`).join(""));
    }
  }
  const pausedRandom = path.join(pausedRoot, "random.json"), pausedAdmission = path.join(pausedRoot, "web-admission.json");
  save(pausedRandom, random); nativeReceipt("randomized_3000", pausedRandom);
  const pausedFinal = { contract: "ait.release.web-admission/v1", status: "admitted", input_sha256: hash(pausedInput), release_id: release.id,
    browser: {}, direct: { solo_local: "pass", solo_remote: "pass" }, randomized: { seed: 42, groups: 3000, status: "pass" } };
  save(pausedAdmission, pausedFinal); nativeReceipt("closeout", pausedAdmission);
  const verifyPaused = () => spawnSync(process.execPath, [path.join(here, "release_web_admission_verify.mjs"), pausedInput, pausedAdmission, temporary], { encoding: "utf8" });
  assert.equal(verifyPaused().status, 0, verifyPaused().stderr);
  save(pausedAdmission, { ...pausedFinal, browser: browserHashes }); nativeReceipt("closeout", pausedAdmission); assert.notEqual(verifyPaused().status, 0);
  save(pausedAdmission, pausedFinal); nativeReceipt("closeout", pausedAdmission);
  for (const groups of [0, 2999, 3001]) { save(pausedRandom, { ...random, executed_groups: groups }); nativeReceipt("randomized_3000", pausedRandom); assert.notEqual(verifyPaused().status, 0); }
  save(pausedRandom, random); nativeReceipt("randomized_3000", pausedRandom);
  const pausedRun = [path.join(here, "release_web_admission.sh"), "run", "--input", pausedInput, "--repository", temporary,
    "--ait", native, "--server-bin", service, "--server-data", temporary, "--server-url", "http://127.0.0.1:8088/", "--output-dir", pausedRoot];
  const pausedCompleted = spawnSync("/bin/bash", pausedRun, { encoding: "utf8", env: { ...environment, AIT_WEB_TEST_COMMUNITY_BIN: "/unavailable", AIT_WEB_TEST_COMMUNITY_CLI_BIN: "/unavailable" } });
  assert.equal(pausedCompleted.status, 0, pausedCompleted.stderr); assert.match(pausedCompleted.stdout, /closeout: complete/);
  assert(!existsSync(dispatch)); assert(!existsSync(path.join(pausedRoot, "evidence/solo-local-browser.json")));
  // Original operation digest rejects scope deletion before any receipt exists.
  const pausedBytes = readFileSync(pausedInput);
  rmSync(pausedInput); writeFileSync(pausedInput, JSON.stringify({ ...paused, execution: { ...paused.execution, random_seed: 43 } }), { mode: 0o400 });
  assert.notEqual(spawnSync("/bin/bash", pausedRun, { encoding: "utf8", env: environment }).status, 0);
  assert(!existsSync(dispatch)); rmSync(pausedInput); writeFileSync(pausedInput, pausedBytes, { mode: 0o400 });

  // Execute the actual Native wrapper against leaf-boundary fixtures. The
  // call log proves both modes, clean companion environment, exact Server,
  // restoration and a HOLD without publishing a combined phase result.
  const leafRoot = path.join(temporary, "native-leaves"), leafAcceptance = path.join(leafRoot, "acceptance");
  mkdirSync(path.join(leafAcceptance, "manifests"), { recursive: true });
  writeFileSync(path.join(leafAcceptance, "manifests/ait-command-matrix.tsv"), "task\tnamespace\ntask show\tleaf\n");
  const leafCalls = path.join(temporary, "native-leaves.calls");
  writeFileSync(path.join(leafAcceptance, "lib.sh"), `
awt_validate_repository() { printf 'validate %s\\n' "$3" >>"$leaf_calls"; }
awt_config_capture() { printf 'capture %s\\n' "$1" >>"$leaf_calls"; cp "$leaf_config" "$AIT_WEB_TEST_RESULTS_DIR/direct-run.config-original.json"; }
awt_config_restore() { printf 'restore\\n' >>"$leaf_calls"; }
awt_config_guard() { trap awt_config_restore EXIT; }
awt_state_digest() { printf '1:1'; }
awt_config_assert_restored() { printf 'assert-restored\\n' >>"$leaf_calls"; }
# Keep WT lib.sh's real append/first-failure semantics at this fixture boundary.
awt_sanitize_field() { printf '%s' "$1" | tr '\\t\\r\\n' '   '; }
awt_append_case_result() {
  printf '%s\\t%s\\t%s\\t%s\\t%s\\n' "$(awt_sanitize_field "$1")" "$(awt_sanitize_field "$2")" \\
    "$(awt_sanitize_field "$3")" "$(awt_sanitize_field "$4")" "$(awt_sanitize_field "$5")" >>"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv"
}
awt_record_first_failure() {
  printf '%s\\t%s\\t%s\\t%s\\n' "$(awt_sanitize_field "$1")" "$(awt_sanitize_field "$2")" \\
    "$(awt_sanitize_field "$3")" "$(awt_sanitize_field "$4")" >"$AIT_WEB_TEST_RESULTS_DIR/first-failure.tsv"
}
awt_run_case() {
  awt_case_id=$1
  shift
  awt_case_before=$(awt_state_digest) || exit 1
  if "$@"; then
    awt_case_after=$(awt_state_digest) || exit 1
    awt_append_case_result "$awt_case_id" pass 0 "$awt_case_before" "$awt_case_after"
    return 0
  else
    awt_case_exit=$?
  fi
  awt_case_after=$(awt_state_digest) || awt_case_after=digest-error
  awt_append_case_result "$awt_case_id" fail "$awt_case_exit" "$awt_case_before" "$awt_case_after"
  awt_record_first_failure "$awt_case_id" "$awt_case_exit" "$awt_case_before" "$awt_case_after"
  exit 1
}
`);
  for (const name of ["check-inventory.sh", "run-cli-contracts.sh", "run-ait-solo-local.sh", "run-ait-solo-remote-lifecycle.sh"]) {
    writeFileSync(path.join(leafAcceptance, name), `#!/bin/sh
set -eu
[ -z "$AIT_WEB_TEST_PERSONAL_BIN$AIT_WEB_TEST_COMMUNITY_BIN$AIT_WEB_TEST_COMMUNITY_CLI_BIN$AIT_WEB_TEST_BROWSER_EVIDENCE" ]
[ "$AIT_WEB_TEST_SERVER_BIN" = /exact/bound-server ]
printf '%s %s\\n' '${name}' "$AIT_WEB_TEST_EXPECTED_MODE" >>"$leaf_calls"
case '${name}' in
  run-cli-contracts.sh) printf 'ait.help.task\\tpass\\nait.help.task.show\\tpass\\nait.invalid.task.show\\tpass\\n' >>"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv" ;;
  run-ait-solo-local.sh) printf 'ait.semantic.task.show\\tpass\\nnative.current.reviewed-fork\\tpass\\n' >>"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv" ;;
  run-ait-solo-remote-lifecycle.sh)
    printf 'native.current.two-authorities\\tpass\\nremote.lifecycle.workflow-finish\\tpass\\nremote.lifecycle.verify-land\\tpass\\n' >>"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv"
    . "$leaf_acceptance/lib.sh"
    awt_formal_lifecycle() { [ "$leaf_hold" = 0 ]; }
    awt_run_case native.current.formal-finish awt_formal_lifecycle ;;
esac
`, { mode: 0o755 });
  }
  const leafConfig = path.join(temporary, "leaf-config.json"), leafInput = path.join(temporary, "leaf-input.json");
  save(leafConfig, config); save(leafInput, { ...paused, web_test: { ...paused.web_test, acceptance_set_sha256: webAcceptanceTree(leafAcceptance).sha256 } });
  const nativeWrapper = driver.slice(driver.indexOf("run_native_direct() ("), driver.indexOf("\nrun_foundation() {"));
  assert(nativeWrapper.startsWith("run_native_direct() ("));
  const nativeInvocation = `set -euo pipefail
ait_bin=/exact/bound-ait
marker="$repository/$leaf_mode-$leaf_phase.marker"
original_results=''
if [ "$leaf_resume" = 1 ]; then original_results=$(cat "$marker"); fi
export AIT_WEB_TEST_PERSONAL_BIN='' AIT_WEB_TEST_COMMUNITY_BIN='' AIT_WEB_TEST_COMMUNITY_CLI_BIN='' AIT_WEB_TEST_BROWSER_EVIDENCE='' AIT_WEB_TEST_SERVER_BIN=/exact/bound-server
${nativeWrapper}
run_native_direct "$leaf_mode" "$leaf_phase" "$original_results"
`;
  const leafEnvironment = { ...process.env, repository: leafRoot, repo_root: path.dirname(here), input: leafInput,
    leaf_calls: leafCalls, leaf_config: leafConfig, leaf_acceptance: leafAcceptance, leaf_resume: "0", leaf_hold: "0" };
  for (const [mode, phase] of [["solo_remote", "inventory"], ["solo_local", "solo-local"], ["solo_remote", "solo-remote"]]) {
    const invoked = spawnSync("/bin/bash", ["-c", nativeInvocation], { encoding: "utf8", env: { ...leafEnvironment, leaf_mode: mode, leaf_phase: phase } });
    assert.equal(invoked.status, 0, invoked.stderr);
  }
  const calls = readFileSync(leafCalls, "utf8");
  assert(calls.includes("run-ait-solo-local.sh solo_local")); assert(calls.includes("run-ait-solo-local.sh solo_remote"));
  assert(calls.includes("run-ait-solo-remote-lifecycle.sh solo_remote")); assert.equal(calls.match(/^restore$/gm).length, 3);
  assert(!calls.includes("community"));
  const beforeHold = spawnSync("find", [leafRoot, "-name", "summary.json"], { encoding: "utf8" }).stdout;
  const heldEnvironment = { ...leafEnvironment, leaf_mode: "solo_remote", leaf_phase: "solo-remote", leaf_hold: "1" };
  const held = spawnSync("/bin/bash", ["-c", nativeInvocation], { encoding: "utf8", env: heldEnvironment });
  assert.equal(held.status, 1, held.stderr);
  assert.equal(spawnSync("find", [leafRoot, "-name", "summary.json"], { encoding: "utf8" }).stdout, beforeHold);
  const originalResults = readFileSync(path.join(leafRoot, "solo_remote-solo-remote.marker"), "utf8").trim();
  const originalCaseBytes = readFileSync(path.join(originalResults, "cases.tsv"));
  const firstFailure = path.join(originalResults, "first-failure.tsv"), firstFailureBytes = readFileSync(firstFailure);
  assert.match(originalCaseBytes.toString(), /native\.current\.formal-finish\tfail\t1\t1:1\t1:1\n/);
  const callCount = readFileSync(leafCalls, "utf8").match(/run-ait-solo-local.sh/g).length;
  const incompleteResume = spawnSync("/bin/bash", ["-c", nativeInvocation], { encoding: "utf8", env: { ...heldEnvironment, leaf_resume: "1" } });
  assert.notEqual(incompleteResume.status, 0); assert(!existsSync(path.join(originalResults, "summary.json")));
  assert(readFileSync(path.join(originalResults, "cases.tsv")).subarray(0, originalCaseBytes.length).equals(originalCaseBytes));
  // Resume only the same formal case after independent review. WT appends its
  // success after the preserved failure; it does not replace either record.
  const reviewed = spawnSync("sh", ["-c", '. "$leaf_acceptance/lib.sh"; awt_run_case native.current.formal-finish true'], { encoding: "utf8", env: {
    ...leafEnvironment, AIT_WEB_TEST_RESULTS_DIR: originalResults, AIT_WEB_TEST_EXPECTED_MODE: "solo_remote", AIT_WEB_TEST_SERVER_BIN: "/exact/bound-server",
    AIT_WEB_TEST_PERSONAL_BIN: "", AIT_WEB_TEST_COMMUNITY_BIN: "", AIT_WEB_TEST_COMMUNITY_CLI_BIN: "", AIT_WEB_TEST_BROWSER_EVIDENCE: "" } });
  assert.equal(reviewed.status, 0, reviewed.stderr);
  const resumed = spawnSync("/bin/bash", ["-c", nativeInvocation], { encoding: "utf8", env: { ...heldEnvironment, leaf_resume: "1" } });
  assert.equal(resumed.status, 0, resumed.stderr); assert(existsSync(path.join(originalResults, "summary.json")));
  const completedCases = readFileSync(path.join(originalResults, "cases.tsv"));
  assert(completedCases.subarray(0, originalCaseBytes.length).equals(originalCaseBytes));
  assert.match(completedCases.toString(), /native\.current\.formal-finish\tpass\t0\t1:1\t1:1\n/);
  assert(readFileSync(firstFailure).equals(firstFailureBytes));
  assert.equal(readFileSync(leafCalls, "utf8").match(/run-ait-solo-local.sh/g).length, callCount, "resume must not replay a Native lifecycle");
  const invalidJournal = path.join(temporary, "invalid-native-journal"); mkdirSync(invalidJournal);
  save(path.join(invalidJournal, "direct-run.config-original.json"), config);
  const nativeResult = JSON.parse(readFileSync(path.join(originalResults, "summary.json")));
  const boundLeaves = JSON.parse(readFileSync(leafInput));
  for (const journal of [
    // The last formal result must pass; an older success cannot hide a new HOLD.
    completedCases + "native.current.formal-finish\tfail\t1\t1:1\t1:1\n",
    // Only the same formal case resolves its HOLD, with WT's full success row.
    originalCaseBytes + "unrelated\tpass\t0\t1:1\t1:1\n",
    originalCaseBytes + "native.current.formal-finish\tpass\n",
    // Other failures remain blockers even when that case later passes.
    completedCases + "native.current.reviewed-fork\tfail\t1\t1:1\t1:1\nnative.current.reviewed-fork\tpass\t0\t1:1\t1:1\n",
  ]) {
    writeFileSync(path.join(invalidJournal, "cases.tsv"), journal);
    assert.throws(() => validateWebDirect(nativeResult, boundLeaves, "solo-remote", leafRoot, invalidJournal), /direct execution contains missing or failed cases/);
  }
  assert(readFileSync(path.join(originalResults, "cases.tsv")).equals(completedCases));
  assert(readFileSync(firstFailure).equals(firstFailureBytes));
  const uncertainRoot = path.join(temporary, "native-uncertain");
  mkdirSync(path.join(uncertainRoot, ".ait-runtime/direct-test-results"), { recursive: true });
  writeFileSync(path.join(uncertainRoot, "solo_remote.marker"), "");
  writeFileSync(path.join(uncertainRoot, "solo_remote.log"), "original uncertain launch\n");
  const uncertainScript = `set -euo pipefail
community=0
receipt_complete() { return 1; }
set_mode() { :; }
run_native_direct() { touch "$browser_guard_dispatch"; return 99; }
${direct}
run_direct solo_remote solo_remote solo-remote remote origin
`;
  const uncertain = spawnSync("/bin/bash", ["-c", uncertainScript], { encoding: "utf8", env: { ...process.env,
    repository: uncertainRoot, log_dir: uncertainRoot, browser_guard_dispatch: dispatch } });
  assert.equal(uncertain.status, 75, uncertain.stderr); assert.match(uncertain.stderr, /nothing is replayed/); assert(!existsSync(dispatch));

  assert.deepEqual(collectionFailures, [], "complete Web collection regression batch");
} finally { rmSync(temporary, { recursive: true, force: true }); }
console.log("release Web current controls and restoration checks passed");
