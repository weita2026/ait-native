#!/usr/bin/env node

import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { cleanupBrowserTaskScenario, runPersonalTaskStartBrowserAction, runPersonalWorkflowBrowserAction, runPersonalRejectedForkBrowserAction,
  runPersonalDeveloperBrowserOperation, runPersonalReviewerBrowserOperation, validateDeveloperBrowserRequest, validateWTDeveloperSession,
  validateReviewerBrowserRequest, validateWTReviewerSession } from "./release_web_browser_evidence.mjs";
import { explicitAuthority } from "./release_web_controls.mjs";

const root = mkdtempSync(path.join(os.tmpdir(), "ait-browser-action-contract."));
const taskId = "LWTT-1234";
const request = { contract: "ait.web.personal.workflow-mutation-request.v2", operation: "task_finish",
  task_id: taskId, scope: "local", remote: null, confirmation: taskId };
const body = { contract: "ait.web.personal.workflow-mutation-receipt.v2", ok: true,
  operation: request.operation, target: taskId, scope: "local", remote: null,
  authority: { kind: "task_finish", task_id: taskId, task_state: "completed",
    landed_snapshot_id: "SNP-111111111111", patchset_ref: null }, inspect_path: `/tasks/${taskId}?scope=local` };

// This transport fixture checks capture and replay prevention only. It does
// not qualify a real browser, backend, Reviewer or B–D release scenario.
function transport(receipt = body, rejectAcceptance = false, faults = {}, mutation = request, status = 200) {
  const page = new EventEmitter();
  const base = "http://127.0.0.1:8080/";
  const posted = JSON.stringify(mutation, null, 2) + "\n", raw = JSON.stringify(receipt, null, 2) + "\n";
  let confirm, respond, responseFailed, accepted;
  const confirmation = new Promise(resolve => { confirm = resolve; });
  const response = new Promise((resolve, reject) => { respond = resolve; responseFailed = reject; });
  const clicked = new Promise(resolve => { accepted = resolve; });
  const observed = { method: () => "POST", url: () => base + "api/actions/workflow",
    postData: () => posted, postDataJSON: () => JSON.parse(posted) };
  const http = { request: () => observed, status: () => status, text: async () => {
    if (faults.bodyRead) throw new Error("response body disconnected");
    return raw;
  },
    headers: () => ({ location: receipt.inspect_path }) };
  const dialogText = "Confirm workflow\n" + JSON.stringify(mutation);
  const dialog = { type: () => "confirm", message: () => dialogText, dismiss: async () => {}, accept: async () => {
    if (rejectAcceptance) throw new Error("confirmation transport interrupted");
    page.emit("request", observed);
    faults.onRequest?.();
    if (faults.noResponse) responseFailed(new Error("response disconnected")); else respond(http);
    accepted();
  } };
  const unused = { count: async () => 0, selectOption: async value => { page.selectedChange = value; } };
  const link = { waitFor: async () => {}, getAttribute: async () => receipt.inspect_path };
  const form = { count: async () => 1, getAttribute: async key => key === "data-scope" ? mutation.scope : key === "data-remote" ? mutation.remote : null,
    locator: selector => selector === 'button[type="submit"]' ? { click: async () => {
      page.clicks++; confirm(dialog); await clicked;
    } } : selector === "[data-current-output]" ? { locator: () => link } : unused };
  page.clicks = 0;
  page.locator = selector => selector.startsWith("[data-current-workflow]") ? form : unused;
  page.waitForEvent = () => confirmation;
  page.waitForResponse = () => response;
  page.url = () => base;
  return { page, base, posted, raw, dialogText };
}

const suffixes = [".confirmation.stdout", ".request.stdout", ".stdout", ".headers"];
const action = (fixture, captureStem) => runPersonalWorkflowBrowserAction({ ...fixture, request,
  generation: "current", repositoryIndex: 23, captureStem });

function startTransport(authority, faults = {}, reconciled = false) {
  const page = new EventEmitter(), base = "http://127.0.0.1:8080/";
  const source = "docs/sprints/browser-capture.md#browser/start", intent = "Capture the exact Task-start attempt";
  const mutation = { source, intent, scope: authority.scope, ...(authority.remote ? { remote: authority.remote } : {}) };
  const target = authority.scope === "local" ? "LWTT-1234" : "RWTT-1234";
  const location = `/tasks/${target}?scope=${authority.scope}${authority.remote ? "&remote=" + authority.remote : ""}`;
  const receipt = reconciled ? { contract: "ait.web.personal.task-start-error.v1", ok: false, operation: "task.start", automatic_retry: false,
    reconciliation: { status: "matched_task", plan_item_ref: "browser/start", matched_count: 1, matched_task: { task_id: target } } } : {
    contract: "ait.web.personal.task-start-receipt.v2", ok: true, operation: "task.start", source, intent, title: "Browser capture",
    plan_item_ref: "browser/start", task_id: target, change_ref: null, ...authority,
    worktree: { name: target.toLowerCase(), open_path: path.join(root, "not-created"), current_line: "feature/" + target.toLowerCase(), reused: false },
  };
  const posted = JSON.stringify(faults.changedRequest ? { ...mutation, source: "docs/another.md#another/start" } : mutation, null, 2) + "\n";
  const raw = JSON.stringify(receipt, null, 2) + "\n", status = reconciled ? 409 : 201;
  const observed = { method: () => "POST", url: () => base + "api/actions/task-start", postData: () => posted, postDataJSON: () => JSON.parse(posted) };
  let confirm, respond, responseFailed, clicked, url = base;
  const confirmation = new Promise(resolve => { confirm = resolve; });
  const response = new Promise((resolve, reject) => { respond = resolve; responseFailed = reject; });
  const finishedClick = new Promise(resolve => { clicked = resolve; });
  const http = { request: () => observed, status: () => status, headers: () => ({ location: faults.wrongLocation ? "/tasks/LWTT-9999?scope=local" : location }),
    text: async () => { if (faults.bodyRead) throw new Error("Task-start response body disconnected"); return raw; } };
  const dialogText = `Start ${authority.scope === "local" ? "Local" : "Remote"} ${authority.remote ?? ""}\n${source}\n${intent}`;
  const dialog = { type: () => "confirm", message: () => dialogText, dismiss: async () => { clicked(); }, accept: async () => {
    if (faults.accept) throw new Error("Task-start confirmation disconnected");
    page.emit("request", observed); if (faults.duplicateRequest) page.emit("request", observed);
    if (faults.noResponse) responseFailed(new Error("Task-start response disconnected")); else respond(http);
    clicked();
  } };
  const form = { locator: selector => {
    if (selector === 'input[name="source"]') return { inputValue: async () => source };
    if (selector === 'input[name="intent"]') return { fill: async value => { page.intent = value; } };
    if (selector === 'select[name="scope"]') return { count: async () => 1, selectOption: async value => { page.scope = value; } };
    if (selector === 'input[name="remote"]') return { count: async () => 1, getAttribute: async () => "text", fill: async value => { page.remote = value; } };
    throw new Error("unexpected Task-start control: " + selector);
  }, getByRole: () => ({ click: async () => { page.clicks++; confirm(dialog); await finishedClick; } }) };
  page.clicks = 0; page.goto = async next => { url = next; }; page.url = () => url;
  page.locator = () => ({ first: () => ({ waitFor: async () => {} }), count: async () => 1, nth: () => form });
  page.waitForEvent = () => confirmation; page.waitForResponse = () => response;
  return { page, base, source, intent, posted, raw, dialogText, receipt, location, status, mutation };
}

try {
  const planAit = path.join(root, "plan-ait.mjs"), planCalls = path.join(root, "plan-calls.jsonl");
  writeFileSync(planAit, `#!${process.execPath}\nimport fs from 'node:fs';const args=process.argv.slice(2);fs.appendFileSync(${JSON.stringify(planCalls)},JSON.stringify(args)+'\\n');if(args[0]!=='plan'||args[1]!=='list')process.exit(99);console.log(JSON.stringify([{head_artifact_path:'docs/sprints/browser-capture.md',plan_id:'PR-1'}]));\n`, { mode: 0o755 });
  for (const authority of [explicitAuthority("local"), explicitAuthority("remote", "origin")]) {
    const invokeStart = (fixture, captureStem) => runPersonalTaskStartBrowserAction({ ...fixture, authority,
      ait: planAit, repository: root, generation: "current", captureStem });
    const fixture = startTransport(authority), captureStem = path.join(root, "task-start-" + authority.scope);
    const started = await invokeStart(fixture, captureStem);
    assert.equal(started.outcome.created, true); assert.deepEqual(started.receipt, fixture.receipt); assert.deepEqual(started.request, fixture.mutation);
    assert.equal(fixture.page.clicks, 1); assert.equal(fixture.page.intent, fixture.intent); assert.equal(fixture.page.scope, authority.scope);
    assert.equal(fixture.page.remote, authority.remote ?? "");
    assert.equal(readFileSync(captureStem + ".request.stdout", "utf8"), fixture.posted);
    assert.equal(readFileSync(captureStem + ".stdout", "utf8"), fixture.raw);
    assert.equal(readFileSync(captureStem + ".confirmation.stdout", "utf8"), fixture.dialogText);
    assert.equal(readFileSync(captureStem + ".headers", "utf8"), `HTTP 201\nLocation: ${fixture.location}\n`);
    const originals = suffixes.map(suffix => readFileSync(captureStem + suffix));
    for (const suffix of suffixes) assert.equal(statSync(captureStem + suffix).mode & 0o777, 0o600);
    const repeat = startTransport(authority);
    await assert.rejects(invokeStart(repeat, captureStem), /already contains an attempt/); assert.equal(repeat.page.clicks, 0);
    suffixes.forEach((suffix, index) => assert.deepEqual(readFileSync(captureStem + suffix), originals[index]));
    for (const [name, fault, requestSaved, headersSaved, bodySaved] of [
      ["body", { bodyRead: true }, true, true, false], ["response", { noResponse: true }, true, false, false],
      ["accept", { accept: true }, false, false, false], ["request", { changedRequest: true }, true, true, true],
      ["duplicate", { duplicateRequest: true }, true, true, true], ["location", { wrongLocation: true }, true, true, true],
    ]) {
      const failed = startTransport(authority, fault), stem = path.join(root, `task-start-${authority.scope}-${name}`);
      await assert.rejects(invokeStart(failed, stem), error => error.retainBrowserFixture === true);
      assert.equal(failed.page.clicks, 1); assert.equal(existsSync(stem + ".request.stdout"), requestSaved);
      assert.equal(existsSync(stem + ".headers"), headersSaved); assert.equal(existsSync(stem + ".stdout"), bodySaved);
      if (requestSaved) assert.equal(readFileSync(stem + ".request.stdout", "utf8"), failed.posted);
      if (bodySaved) assert.equal(readFileSync(stem + ".stdout", "utf8"), failed.raw);
      const retried = startTransport(authority);
      await assert.rejects(invokeStart(retried, stem), /already contains an attempt/); assert.equal(retried.page.clicks, 0);
    }
    const reconciled = startTransport(authority, {}, true), stem = path.join(root, "task-start-reconciled-" + authority.scope);
    assert.equal((await invokeStart(reconciled, stem)).outcome.created, false);
    assert.equal(reconciled.page.clicks, 1); assert.equal(readFileSync(stem + ".stdout", "utf8"), reconciled.raw);
  }
  assert(readFileSync(planCalls, "utf8").trim().split("\n").map(JSON.parse).every(argv => argv.includes("--local") ||
    (argv.includes("--remote") && argv[argv.indexOf("--remote") + 1] === "origin")));
  const contacts = { registered: true, members: [{ agent_index: 1, role_index: 2, role_state: "active", session_id: "fixture-reviewer" }] };
  validateWTReviewerSession(contacts, "fixture-reviewer");
  for (const [record, session] of [[contacts, "fixture-developer"], [contacts, undefined], [{ ...contacts, registered: false }, "fixture-reviewer"],
    [{ ...contacts, members: [] }, "fixture-reviewer"], [{ ...contacts, members: [contacts.members[0], contacts.members[0]] }, "fixture-reviewer"]]) {
    assert.throws(() => validateWTReviewerSession(record, session));
  }
  const developerContacts = { registered: true, members: [{ agent_index: 2, role_index: 1, role_state: "active", session_id: "fixture-developer" }] };
  validateWTDeveloperSession(developerContacts, "fixture-developer");
  for (const [record, session] of [[contacts, "fixture-reviewer"], [developerContacts, "fixture-reviewer"], [developerContacts, undefined],
    [{ ...developerContacts, registered: false }, "fixture-developer"], [{ ...developerContacts, members: [] }, "fixture-developer"],
    [{ ...developerContacts, members: [developerContacts.members[0], developerContacts.members[0]] }, "fixture-developer"],
    [{ ...developerContacts, members: [{ ...developerContacts.members[0], role_state: "inactive" }] }, "fixture-developer"],
    [{ ...developerContacts, members: [{ ...developerContacts.members[0], role_index: 2 }] }, "fixture-developer"]]) {
    assert.throws(() => validateWTDeveloperSession(record, session));
  }
  assert.throws(() => validateWTReviewerSession(developerContacts, "fixture-developer"));
  const developerCapture = { contract: request.contract, operation: "snapshot_create", task_id: taskId,
    scope: "local", remote: null, change_ref: null, message: "Initial frozen browser capture" };
  assert.deepEqual(validateDeveloperBrowserRequest(developerCapture), explicitAuthority("local"));
  const developerChange = { contract: request.contract, operation: "change_create", task_id: taskId,
    scope: "local", remote: null, title: "Reviewed correction", fork_snapshot_id: "SNP-111111111111" };
  assert.deepEqual(validateDeveloperBrowserRequest(developerChange), explicitAuthority("local"));
  for (const operation of [developerCapture, { ...developerCapture, change_ref: `${taskId}/C-01` }, developerChange]) {
    const remote = { ...operation, task_id: "RWTT-1234", scope: "remote", remote: "origin" };
    if (operation.change_ref) remote.change_ref = "RWTT-1234/C-01";
    assert.deepEqual(validateDeveloperBrowserRequest(remote), explicitAuthority("remote", "origin"));
    assert.throws(() => validateDeveloperBrowserRequest({ ...remote, remote: "another" }));
  }
  assert.throws(() => validateDeveloperBrowserRequest(request));
  await assert.rejects(runPersonalDeveloperBrowserOperation({ request }), /WT Developer operation/);
  await assert.rejects(runPersonalDeveloperBrowserOperation({ request, actor: "reviewer", env: { CODEX_THREAD_ID: "fixture-reviewer" } }), /WT Developer operation/);
  await assert.rejects(runPersonalDeveloperBrowserOperation({ request: developerCapture }), /original native worktree name/);
  await assert.rejects(runPersonalReviewerBrowserOperation({ request: developerCapture }), /WT Reviewer operation/);
  assert.deepEqual(validateReviewerBrowserRequest(request), explicitAuthority("local"));
  const formal = { contract: request.contract, operation: "workflow_finish", task_id: "RWTT-1234", scope: "remote", remote: "origin",
    candidate_kind: "patchset", apply: true, expected_identity: { repository_index: 23, patchset_id: "RWTT-1234/P-01",
      base_snapshot_id: "SNP-111111111111", revision_snapshot_id: "SNP-222222222222" }, confirmation: "RWTT-1234/P-01",
    reviewer: "transport-fixture-only", review: Object.fromEntries(["reviewed_files", "findings", "risks", "tests", "recommendation"].map(key => [key, "fixture"])) };
  assert.deepEqual(validateReviewerBrowserRequest(formal), explicitAuthority("remote", "origin"));
  const developerReady = { ...formal, operation: "workflow_ready" };
  for (const key of ["confirmation", "reviewer", "review"]) delete developerReady[key];
  for (const apply of [false, true]) {
    for (const expected_identity of [null, developerReady.expected_identity])
      assert.deepEqual(validateDeveloperBrowserRequest({ ...developerReady, apply, expected_identity }), explicitAuthority("remote", "origin"));
  }
  const packageReady = { ...developerReady, task_id: taskId, candidate_kind: "package", expected_identity: {
    repository_index: 23, package_hash: "a".repeat(64), worker_job_index: 1,
    base_snapshot_id: "SNP-111111111111", revision_snapshot_id: "SNP-222222222222",
  } };
  assert.deepEqual(validateDeveloperBrowserRequest(packageReady), explicitAuthority("remote", "origin"));
  for (const candidate of [developerReady, packageReady]) {
    assert.throws(() => validateDeveloperBrowserRequest({ ...candidate, remote: "another" }));
    assert.throws(() => validateDeveloperBrowserRequest({ ...candidate, expected_identity: { ...candidate.expected_identity, repository_index: 24 } }));
  }
  assert.throws(() => validateDeveloperBrowserRequest(formal));
  for (const change of [value => { value.apply = false; }, value => { value.remote = "another"; },
    value => { value.expected_identity.repository_index = 24; }, value => { delete value.reviewer; }, value => { delete value.review; },
    value => { value.operation = "workflow_ready"; delete value.confirmation; delete value.reviewer; delete value.review; }]) {
    const invalid = structuredClone(formal); change(invalid); assert.throws(() => validateReviewerBrowserRequest(invalid));
  }
  const first = transport(), stem = path.join(root, "accepted");
  assert.deepEqual(await action(first, stem), { status: 200, body });
  assert.equal(first.page.clicks, 1);
  assert.equal(readFileSync(stem + ".request.stdout", "utf8"), first.posted);
  assert.equal(readFileSync(stem + ".stdout", "utf8"), first.raw);
  assert.equal(readFileSync(stem + ".confirmation.stdout", "utf8"), first.dialogText);
  assert.equal(readFileSync(stem + ".headers", "utf8"), `HTTP 200\nLocation: ${body.inspect_path}\n`);
  const originals = suffixes.map(suffix => readFileSync(stem + suffix));
  for (const suffix of suffixes) assert.equal(statSync(stem + suffix).mode & 0o777, 0o600);
  const repeat = transport();
  await assert.rejects(action(repeat, stem), /already contains an attempt/);
  assert.equal(repeat.page.clicks, 0);
  suffixes.forEach((suffix, index) => assert.deepEqual(readFileSync(stem + suffix), originals[index]));

  const interrupted = transport(body, true), interruptedStem = path.join(root, "interrupted");
  await assert.rejects(action(interrupted, interruptedStem), /transport interrupted/);
  assert(existsSync(interruptedStem + ".confirmation.stdout"));
  const retry = transport();
  await assert.rejects(action(retry, interruptedStem), /already contains an attempt/);
  assert.equal(retry.page.clicks, 0);

  for (const [name, fault, hasHeaders] of [["body-disconnect", { bodyRead: true }, true], ["no-response", { noResponse: true }, false]]) {
    const fixture = transport(body, false, fault), captureStem = path.join(root, name);
    await assert.rejects(action(fixture, captureStem), error => /disconnected/.test(error.message) && error.retainBrowserFixture === true);
    assert.equal(fixture.page.clicks, 1);
    assert.equal(readFileSync(captureStem + ".request.stdout", "utf8"), fixture.posted);
    assert.equal(readFileSync(captureStem + ".confirmation.stdout", "utf8"), fixture.dialogText);
    assert.equal(existsSync(captureStem + ".headers"), hasHeaders);
    if (hasHeaders) assert.equal(readFileSync(captureStem + ".headers", "utf8"), `HTTP 200\nLocation: ${body.inspect_path}\n`);
    assert(!existsSync(captureStem + ".stdout"), "an unavailable response body must remain absent");
    const repeated = transport();
    await assert.rejects(action(repeated, captureStem), /already contains an attempt/);
    assert.equal(repeated.page.clicks, 0);
  }

  const wrong = transport({ ...body, target: "LWTT-9999" }), wrongStem = path.join(root, "wrong-result");
  await assert.rejects(action(wrong, wrongStem));
  assert.equal(wrong.page.clicks, 1);
  assert.equal(readFileSync(wrongStem + ".stdout", "utf8"), wrong.raw);
  assert.equal(readFileSync(wrongStem + ".request.stdout", "utf8"), wrong.posted);

  const forkStateFile = path.join(root, "fork-state.json"), forkCallsFile = path.join(root, "fork-calls.jsonl"), forkAit = path.join(root, "fork-ait.mjs");
  writeFileSync(forkAit, `#!${process.execPath}\nimport fs from 'node:fs';const args=process.argv.slice(2),state=JSON.parse(fs.readFileSync(${JSON.stringify(forkStateFile)}));fs.appendFileSync(${JSON.stringify(forkCallsFile)},JSON.stringify(args)+'\\n');if(args[0]==='task'&&args[1]==='show')console.log(JSON.stringify(state.task));else if(args[0]==='worktree'&&args[1]==='show')console.log(JSON.stringify(state.worktree));else if(args[0]==='change'&&args[1]==='list')console.log(JSON.stringify(state.changes));else process.exit(99);\n`, { mode: 0o755 });
  for (const authority of [explicitAuthority("local"), explicitAuthority("remote", "origin")]) {
    const target = authority.scope === "local" ? taskId : "RWTT-1234";
    const mutation = { contract: request.contract, operation: "change_create", task_id: target, ...authority,
      title: "Correct the exact reviewed Snapshot", fork_snapshot_id: "SNP-000000000000" };
    const rejection = { contract: "ait.web.personal.workflow-mutation-error.v2", ok: false, operation: mutation.operation,
      target, task_id: target, ...authority, code: "mutation_precondition_failed", automatic_retry: false,
      message: "Reviewed fork must equal the exact frozen Task head; no latest-Snapshot fallback", reconciliation: null, expected_identity: null };
    const native = { task: { task_id: target, repo_name: "ait-web-test", status: "active" },
      worktree: { bound_task_id: target, bound_change_ref: null, head_snapshot_id: "SNP-111111111111",
        capture_authority: authority.scope === "local" ? null : { remote_name: "origin", repository_index: 23 } }, changes: [] };
    const saveNative = () => writeFileSync(forkStateFile, JSON.stringify(native)); saveNative(); writeFileSync(forkCallsFile, "");
    const fixture = transport(rejection, false, {}, mutation, 409), captureStem = path.join(root, "rejected-fork-" + authority.scope);
    const options = { ...fixture, request: mutation, generation: "current", repositoryIndex: 23, ait: forkAit, repository: root,
      worktreeName: "original-task", reviewedSnapshot: native.worktree.head_snapshot_id, captureStem };
    assert.deepEqual(await runPersonalRejectedForkBrowserAction(options), { status: 409, body: rejection });
    assert.equal(fixture.page.clicks, 1);
    assert.equal(readFileSync(captureStem + ".request.stdout", "utf8"), fixture.posted);
    assert.equal(readFileSync(captureStem + ".stdout", "utf8"), fixture.raw);
    assert.equal(readFileSync(captureStem + ".headers", "utf8"), "HTTP 409\nLocation: <absent>\n");
    const reads = readFileSync(forkCallsFile, "utf8").trim().split("\n").map(JSON.parse);
    assert.equal(reads.length, 6); assert(reads.every(argv => ["show", "list"].includes(argv[1])));
    for (const argv of reads.filter(argv => argv[0] !== "worktree")) {
      assert(argv.includes(authority.scope === "local" ? "--local" : "--remote"));
      if (authority.scope === "remote") assert.equal(argv[argv.indexOf("--remote") + 1], "origin");
    }
    const original = suffixes.map(suffix => readFileSync(captureStem + suffix));
    const repeated = transport(rejection, false, {}, mutation, 409);
    await assert.rejects(runPersonalRejectedForkBrowserAction({ ...options, ...repeated }), /already contains an attempt/);
    assert.equal(repeated.page.clicks, 0); suffixes.forEach((suffix, index) => assert.deepEqual(readFileSync(captureStem + suffix), original[index]));

    const mistaken = transport(rejection, false, {}, mutation, 409);
    await assert.rejects(runPersonalWorkflowBrowserAction({ ...options, ...mistaken, captureStem: path.join(root, "positive-reject-" + authority.scope) }), /failed or is ambiguous/);
    const effected = transport(rejection, false, { onRequest: () => { native.changes.push({ task_id: target, change_ref: target + "/C-01" }); saveNative(); } }, mutation, 409);
    await assert.rejects(runPersonalRejectedForkBrowserAction({ ...options, ...effected, captureStem: path.join(root, "effected-" + authority.scope) }),
      error => /without revision Changes/.test(error.message) && error.retainBrowserFixture === true);
    const guarded = transport(rejection, false, {}, mutation, 409);
    await assert.rejects(runPersonalRejectedForkBrowserAction({ ...options, ...guarded, captureStem: path.join(root, "preexisting-change-" + authority.scope) }), /without revision Changes/);
    assert.equal(guarded.page.clicks, 0);
  }

  for (const authority of [explicitAuthority("local"), explicitAuthority("remote", "origin")]) {
    const target = authority.scope === "local" ? taskId : "RWTT-1234", change = target + "/C-01";
    const baseNative = () => ({ task: { task_id: target, repo_name: "ait-web-test", status: "active" },
      worktree: { bound_task_id: target, bound_change_ref: null,
        capture_authority: authority.scope === "local" ? null : { remote_name: "origin", repository_index: 23 } },
      changes: [{ task_id: target, change_ref: change, repo_name: "ait-web-test", status: "draft" }] });
    const mutation = changeRef => ({ contract: request.contract, operation: "snapshot_create", task_id: target, ...authority,
      change_ref: changeRef, message: "Capture the exact reviewed revision" });
    const receipt = changeRef => ({ contract: body.contract, ok: true, operation: "snapshot_create", target: changeRef ?? target, ...authority,
      authority: { kind: "snapshot", task_id: target, change_ref: changeRef, snapshot_id: "SNP-222222222222", line_name: "feature/original-task",
        capture_binding: authority.scope === "local" ? null : { contract: "ait.snapshot_capture_binding.v2", repository_index: 23,
          task_id: target, change_ref: changeRef, snapshot_id: "SNP-222222222222", line_name: "feature/original-task" } },
      inspect_path: `/tasks/${target}?scope=${authority.scope}${authority.remote ? "&remote=" + authority.remote : ""}` });
    const invoke = (native, changeRef, stem) => {
      writeFileSync(forkStateFile, JSON.stringify(native)); writeFileSync(forkCallsFile, "");
      const fixture = transport(receipt(changeRef), false, {}, mutation(changeRef), 201);
      return { fixture, run: () => runPersonalWorkflowBrowserAction({ ...fixture, request: mutation(changeRef), generation: "current",
        repositoryIndex: 23, ait: forkAit, repository: root, worktreeName: "original-task", captureStem: path.join(root, stem + "-" + authority.scope) }) };
    };
    const captureCases = [["initial", null, null], ...["draft", "active", "review"].flatMap(status => [
      ["revision-unset-" + status, change, null, status], ["revision-bound-" + status, change, change, status],
    ])];
    for (const [name, changeRef, binding, status] of captureCases) {
      const native = baseNative(); native.worktree.bound_change_ref = binding;
      if (changeRef === null) native.changes = [];
      else native.changes[0].status = status;
      const trial = invoke(native, changeRef, name);
      assert.deepEqual(await trial.run(), { status: 201, body: receipt(changeRef) });
      assert.equal(trial.fixture.page.clicks, 1); assert.equal(trial.fixture.page.selectedChange, changeRef ?? "");
      const reads = readFileSync(forkCallsFile, "utf8").trim().split("\n").map(JSON.parse);
      assert.equal(reads.length, 3); assert(reads.every(argv => ["show", "list"].includes(argv[1])));
      for (const argv of reads.filter(argv => argv[0] !== "worktree")) {
        assert(argv.includes(authority.scope === "local" ? "--local" : "--remote"));
        if (authority.remote) assert.equal(argv[argv.indexOf("--remote") + 1], authority.remote);
      }
    }
    let index = 0;
    const invalid = [
      native => { native.worktree.bound_task_id = "LWTT-9999"; },
      native => { native.worktree.bound_change_ref = target + "/C-02"; },
      native => { delete native.worktree.bound_change_ref; },
      native => { native.task.status = "completed"; }, native => { native.task.task_id = "LWTT-9999"; },
      native => { delete native.task.repo_name; }, native => { native.changes = null; }, native => { native.changes = []; },
      native => { native.changes[0].task_id = "LWTT-9999"; }, native => { native.changes[0].repo_name = "another-repo"; },
      native => { delete native.changes[0].status; }, native => { native.changes[0].status = null; },
      native => { native.changes.push(native.changes[0]); },
      ...["landed", "archived", "abandoned", "canceled", "unknown-contract-state", "open", "", "DRAFT", "reviewed"]
        .map(status => native => { native.changes[0].status = status; }),
      native => { native.worktree.capture_authority = { remote_name: "another", repository_index: 24 }; },
    ];
    for (const mutate of invalid) {
      const native = baseNative(); mutate(native); const trial = invoke(native, change, "invalid-capture-" + index++);
      await assert.rejects(trial.run()); assert.equal(trial.fixture.page.clicks, 0);
    }
    const omitted = invoke(baseNative(), null, "omitted-revision");
    await assert.rejects(omitted.run(), /explicit reviewed Change/); assert.equal(omitted.fixture.page.clicks, 0);
  }

  const stateFile = path.join(root, "state.json"), callsFile = path.join(root, "calls.jsonl");
  const worktreePath = path.join(root, "owned-worktree"), ait = path.join(root, "ait.mjs");
  writeFileSync(ait, `#!${process.execPath}\nimport fs from 'node:fs';const args=process.argv.slice(2);fs.appendFileSync(${JSON.stringify(callsFile)},JSON.stringify(args)+'\\n');if(args[0]!=='task')process.exit(99);if(args[1]==='show')console.log(fs.readFileSync(${JSON.stringify(stateFile)},'utf8'));else if(args[1]==='abandon'){fs.rmdirSync(${JSON.stringify(worktreePath)});console.log(JSON.stringify({task_id:args[2],status:'abandoned'}));}else process.exit(99);\n`, { mode: 0o755 });
  const calls = () => readFileSync(callsFile, "utf8").trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
  const state = status => { writeFileSync(stateFile, JSON.stringify({ task_id: taskId, status })); writeFileSync(callsFile, ""); };
  const cleanup = expectedFinalState => cleanupBrowserTaskScenario({ ait, repository: root, taskId,
    authority: explicitAuthority("local"), worktreePath, expectedFinalState });
  state("completed");
  assert.equal(cleanup("completed"), "completed");
  assert.equal(calls().length, 1); assert(calls().every(args => args[1] === "show"));

  state("active"); mkdirSync(worktreePath);
  assert.throws(() => cleanup("completed"), /has not reached/);
  assert(existsSync(worktreePath)); assert(calls().every(args => args[1] === "show"));
  state("active");
  assert.equal(cleanup("abandoned"), "abandoned");
  assert.equal(calls().filter(args => args[1] === "abandon").length, 1); assert(!existsSync(worktreePath));
  state("abandoned");
  assert.equal(cleanup("abandoned"), "abandoned");
  assert(calls().every(args => args[1] === "show"));
  state("completed"); mkdirSync(worktreePath);
  assert.throws(() => cleanup("completed"), /still owns its worktree/);
  assert(existsSync(worktreePath)); assert(calls().every(args => args[1] === "show"));
  console.log("Browser action raw captures, replay prevention and lifecycle cleanup checks passed");
} finally {
  rmSync(root, { recursive: true, force: true });
}
