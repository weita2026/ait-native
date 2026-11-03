#!/usr/bin/env node

import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, realpathSync, rmSync, writeFileSync, writeSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { enterDerivedOperation } from "./release_operation_owner.mjs";
import { currentControls, validateCurrentDoctor, renderedControls, explicitAuthority, authorityArgv,
  webPersonalInScope, validateWebOperation, validateBrowserRuntimeHead, webAcceptanceTree, scopedReadUrl, taskStartRequest, taskStartOutcome, validateCaptureAuthority } from "./release_web_controls.mjs";

import { validateWorkflowRequest, currentWorkflowReceipt, currentReviewedForkRejection } from "./release_web_workflow.mjs";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function parseCli(argv) {
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) fail("invalid browser evidence arguments", 64);
    options[key] = value;
  }
  for (const key of [
    "--repository", "--mode", "--ait", "--community-bin",
    "--community-cli-bin", "--server-url", "--chrome", "--output", "--scope",
  ]) if (!options[key]) fail(`missing required argument: ${key}`, 64);
  return options;
}

function absolute(value, label) {
  if (!path.isAbsolute(value)) fail(`${label} must be an absolute path`, 64);
  return path.resolve(value);
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function commandJson(command, argv, cwd, label, env = process.env) {
  const result = spawnSync(command, argv, { cwd, env, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(`${label} failed (${result.status ?? result.signal ?? "spawn"}):\n${String(result.stdout ?? "").slice(-8000)}\n${String(result.stderr ?? result.error ?? "").slice(-8000)}`);
  try { return JSON.parse(result.stdout); } catch { throw new Error(`${label} returned invalid JSON`); }
}

function executable(file, label) {
  if (!existsSync(file)) fail(`${label} is missing: ${file}`, 66);
  const descriptor = openSync(file, "r");
  closeSync(descriptor);
}

function waitForJson(stream, label, timeoutMs, predicate) {
  return new Promise((resolve, reject) => {
    let buffer = "";
    const timer = setTimeout(() => reject(new Error(`${label} readiness timed out`)), timeoutMs);
    stream.setEncoding("utf8");
    stream.on("data", (chunk) => {
      buffer += chunk;
      try {
        const value = JSON.parse(buffer);
        if (predicate(value)) {
          clearTimeout(timer);
          resolve(value);
        }
      } catch {}
    });
  });
}

async function unusedPort() {
  const server = createServer();
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function waitHttp(url, child, label) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`${label} exited before readiness`);
    try {
      const response = await fetch(url, { redirect: "manual" });
      if (response.status >= 200 && response.status < 500) return;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${label} HTTP readiness timed out`);
}

export class Cdp {
  constructor(url) {
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Map();
    this.socket = new WebSocket(url);
  }

  async ready() {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.socket.close(); reject(new Error("Chrome DevTools connection timed out")); }, 10000);
      this.socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
      this.socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Chrome DevTools connection failed")); }, { once: true });
    });
    this.socket.addEventListener("close", () => {
      for (const pending of this.pending.values()) pending.reject(new Error("Chrome DevTools disconnected; retain any dispatched mutation for reconciliation"));
      this.pending.clear();
    });
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(message.error.message));
        else pending.resolve(message.result);
        return;
      }
      for (const listener of this.listeners.get(message.method) ?? []) listener(message.params ?? {});
    });
    return this;
  }

  send(method, params = {}, timeoutMs = 30000) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Chrome DevTools ${method} timed out; retain any dispatched mutation for reconciliation`));
      }, timeoutMs);
      this.pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
      try { this.socket.send(JSON.stringify({ id, method, params })); }
      catch (error) { this.pending.delete(id); clearTimeout(timer); reject(error); }
    });
  }

  on(method, listener) {
    const listeners = this.listeners.get(method) ?? [];
    listeners.push(listener);
    this.listeners.set(method, listeners);
  }

  close() {
    for (const pending of this.pending.values()) pending.reject(new Error("Chrome DevTools closed"));
    this.pending.clear();
    this.socket.close();
  }
}

export async function newTab(debugPort) {
  const response = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent("about:blank")}`, { method: "PUT" });
  if (!response.ok) throw new Error("Chrome target creation failed");
  const target = await response.json();
  return new Cdp(target.webSocketDebuggerUrl).ready();
}

export async function openNativeBrowser({ helper, playwrightModule, chrome, diagnosticsDir }) {
  for (const [file, label] of [[helper, "WT headless helper"], [playwrightModule, "Playwright module"]]) {
    if (!path.isAbsolute(file ?? "") || !existsSync(file)) throw new Error(`${label} requires an explicit existing absolute path`);
  }
  const runner = await import(pathToFileURL(helper).href);
  if (typeof runner.openHeadlessBrowser !== "function") throw new Error("WT headless launcher contract is unavailable");
  return runner.openHeadlessBrowser({ chrome, playwrightModule, diagnosticsDir });
}

function observePage(page, observations, origin) {
  const favicon = value => { try { const url = new URL(value); return url.origin === origin && url.pathname === "/favicon.ico"; } catch { return false; } };
  page.on("console", message => {
    const automaticFavicon404 = message.type() === "error" && favicon(message.location().url) &&
      /Failed to load resource:.*404/.test(message.text());
    if (["error", "warning", "assert"].includes(message.type()) && !automaticFavicon404)
      observations.console.push({ type: message.type(), text: message.text().slice(0, 500), location: message.location() });
  });
  page.on("pageerror", error => observations.console.push({ type: "exception", text: error.message }));
  page.on("requestfailed", request => observations.failedRequests.push(`${request.failure()?.errorText} ${request.url()}`));
  page.on("request", request => {
    if (new URL(request.url()).origin !== origin) observations.externalRequests.push(request.url());
  });
  page.on("response", response => {
    if (response.status() >= 400 && !(response.status() === 404 && favicon(response.url())))
      observations.failedRequests.push(`${response.status()} ${response.url()}`);
  });
}

async function inspect(page, url, viewport, screenshotPath) {
  await page.setViewportSize({ width: viewport.width, height: viewport.height });
  await page.goto(url, { waitUntil: "load" });
  // Read rendered layout only. Mutations use actual forms and native dialogs.
  const value = await page.evaluate(() => ({
    readyState: document.readyState, url: location.href, title: document.title,
    text: document.body?.innerText ?? "",
    definitions: Object.fromEntries([...document.querySelectorAll("dt")].map(dt =>
      [dt.textContent.trim(), dt.nextElementSibling?.textContent.replace(/\s+/g, " ").trim()])),
    documentWidth: document.documentElement.scrollWidth, clientWidth: document.documentElement.clientWidth,
    bodyWidth: document.body?.scrollWidth ?? 0, buttons: document.querySelectorAll("button").length,
    forms: document.querySelectorAll("form").length, actionRows: document.querySelectorAll("[data-action-row]").length,
    actionLinks: document.querySelectorAll("[data-action-row] a[href]").length,
  }));
  if (value.readyState !== "complete" || !value.text) throw new Error(`browser page did not become ready: ${url}`);
  if (existsSync(screenshotPath)) throw new Error(`browser screenshot already exists: ${screenshotPath}`);
  await page.screenshot({ path: screenshotPath, fullPage: false });
  return { ...value, maxOverflowPx: Math.max(0, value.documentWidth - viewport.width, value.bodyWidth - viewport.width) };
}

async function terminate(child) {
  if (!child || child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    new Promise((resolve) => setTimeout(resolve, 2_000)),
  ]);
  if (child.exitCode === null) child.kill("SIGKILL");
}

export async function runPersonalTaskStartBrowserAction({ page, base, authority, ait, repository, generation,
  source = "docs/sprints/direct_personal_action_cases.md#ait-web-test/direct-personal-action-cases/start-probe", env = process.env,
  intent = "Native browser Task-start qualification", captureStem }) {
  if (!["current", "legacy"].includes(generation)) throw new Error("Browser Task-start generation must be verified before mutation");
  if (captureStem) {
    if (!path.isAbsolute(captureStem)) throw new Error("Browser case capture stem must be absolute");
    for (const suffix of [".confirmation.stdout", ".request.stdout", ".stdout", ".headers"])
      if (existsSync(captureStem + suffix)) throw new Error("Browser case already contains an attempt; inspect its original captures and native state without repeating the action");
    mkdirSync(path.dirname(captureStem), { recursive: true, mode: 0o700 });
  }
  const request = taskStartRequest(source, intent, authority);
  const plans = commandJson(ait, ["plan", "list", ...authorityArgv(authority), "--json"], repository, "exact browser Plan authority", env);
  const matches = Array.isArray(plans) ? plans.filter(row => row.head_artifact_path === source.split("#")[0]) : [];
  if (matches.length !== 1 || !/^PR-[0-9]+$/.test(matches[0].plan_id ?? "")) throw new Error("Browser Task-start fixture has no unique Plan in the selected authority");
  const planUrl = scopedReadUrl(base, `/plans/${encodeURIComponent(matches[0].plan_id)}`, authority);
  await page.goto(planUrl, { waitUntil: "domcontentloaded" });
  const forms = page.locator("[data-task-start-form]");
  await forms.first().waitFor({ state: "visible" });
  const exact = [];
  for (let i = 0; i < await forms.count(); i++) {
    const candidate = forms.nth(i);
    if (await candidate.locator('input[name="source"]').inputValue() === source) exact.push(candidate);
  }
  if (exact.length !== 1) throw new Error("The exact scoped browser Task-start form is unavailable");
  const form = exact[0];
  await form.locator('input[name="intent"]').fill(intent);
  const scopeSelect = form.locator('select[name="scope"]');
  if (await scopeSelect.count()) await scopeSelect.selectOption(authority.scope);
  else if (await form.locator('input[name="scope"]').inputValue() !== authority.scope) throw new Error("Task-start form lost its verified scope");
  const remoteInput = form.locator('input[name="remote"]');
  if (authority.scope === "remote") {
    if (await remoteInput.getAttribute("type") === "hidden") {
      if (await remoteInput.inputValue() !== authority.remote) throw new Error("Task-start form lost its named Remote");
    } else await remoteInput.fill(authority.remote);
  } else if (await remoteInput.count() && await remoteInput.getAttribute("type") !== "hidden") await remoteInput.fill("");
  let submitted = 0, attempted = false, dialog, captureError;
  const preserve = (suffix, value) => {
    if (!captureStem) return;
    try { writeFileSync(captureStem + suffix, value(), { mode: 0o600, flag: "wx" }); }
    catch (error) { captureError ??= error; }
  };
  const observe = native => {
    const uri = new URL(native.url());
    if (native.method() === "POST" && uri.origin === new URL(base).origin && uri.pathname === "/api/actions/task-start") {
      submitted++; attempted = true;
      if (submitted === 1) preserve(".request.stdout", () => native.postData() ?? "");
    }
  };
  page.on("request", observe);
  try {
    writeSync(1, "Personal browser Task-start dispatch prepared; do not repeat this attempt\n");
    const dialogPromise = page.waitForEvent("dialog").catch(error => ({ error }));
    const responsePromise = page.waitForResponse(response => {
      const uri = new URL(response.url());
      return uri.origin === new URL(base).origin && uri.pathname === "/api/actions/task-start" && response.request().method() === "POST";
    }).then(response => {
      attempted = true;
      if (submitted === 0) preserve(".request.stdout", () => response.request().postData() ?? "");
      preserve(".headers", () => `HTTP ${response.status()}\nLocation: ${response.headers().location ?? "<absent>"}\n`);
      return response;
    }).catch(error => ({ error }));
    const click = form.getByRole("button", { name: "Review & start", exact: true }).click().catch(error => ({ error }));
    dialog = await dialogPromise;
    if (dialog.error) throw dialog.error;
    const confirmation = dialog.message();
    if (dialog.type() !== "confirm" || !confirmation.includes(source) || !confirmation.includes(intent) ||
        !confirmation.includes(authority.scope === "local" ? "Local" : "Remote") ||
        (authority.scope === "remote" && !confirmation.includes(authority.remote))) {
      await dialog.dismiss(); dialog = undefined; await click;
      throw new Error("Native confirmation dialog changed the exact Task source, intent or authority");
    }
    preserve(".confirmation.stdout", () => confirmation);
    if (captureError) throw captureError;
    attempted = true;
    await dialog.accept(); dialog = undefined;
    const clicked = await click;
    if (clicked?.error) throw clicked.error;
    const response = await responsePromise;
    if (response.error) throw response.error;
    const observed = response.request(), raw = await response.text();
    preserve(".stdout", () => raw);
    if (captureError) throw captureError;
    if (new URL(observed.url()).search || submitted !== 1 || !isDeepStrictEqual(observed.postDataJSON(), request))
      throw new Error("Observed Task-start network request differs from the exact JSON-only action");
    const http = { status: response.status(), location: response.headers().location, body: JSON.parse(raw) };
    writeSync(1, `HTTP ${http.status}\nLocation: ${http.location ?? "<absent>"}\n${raw}\n`);
    const outcome = taskStartOutcome(http, generation, request);
    return { outcome, receipt: http.body, request, form, planUrl };
  } catch (error) {
    await dialog?.dismiss?.().catch(() => {});
    if (attempted) error.retainBrowserFixture = true;
    throw error;
  } finally { page.off("request", observe); }
}

async function inspectBrowserTask(page, base, started, task) {
  const { outcome, form, planUrl } = started;
  const link = form.locator("[data-mutation-output] a.mutation-result-link");
  await link.waitFor({ state: "visible" });
  if (await link.getAttribute("href") !== outcome.inspectPath) throw new Error("Personal Inspect link differs from the server-verified Location");
  const expected = new URL(outcome.inspectPath, base).href;
  await Promise.all([page.waitForURL(expected), link.click()]);
  const verify = async () => {
    if (page.url() !== expected) throw new Error("Personal Inspect navigation lost its selected authority");
    const text = await page.locator("body").innerText();
    if (![task.task_id, task.title, task.intent].filter(Boolean).every(value => text.includes(value))) {
      throw new Error("Personal Inspect rendered another Task/authority with the same public ID");
    }
  };
  await verify();
  await page.reload({ waitUntil: "domcontentloaded" }); await verify();
  await page.goBack({ waitUntil: "domcontentloaded" });
  if (page.url() !== planUrl) throw new Error("Personal Back navigation lost the scoped Plan context");
  await page.goForward({ waitUntil: "domcontentloaded" }); await verify();
}

// WT owns reusable scenarios. Native owns this adapter and its existing result
// boundary. Keep native waits intact and never repeat an uncertain UI action.
export async function runPersonalWorkflowBrowserAction(options) {
  return performPersonalWorkflowBrowserAction(options);
}

// This is an expected negative case before the first revision. The actual UI
// error and unchanged native Task/Change bindings are both required. It never
// replaces a successful correction capture or enables the B–D pass emitter.
export async function runPersonalRejectedForkBrowserAction(options) {
  const { request, reviewedSnapshot, repositoryIndex, ait, repository, worktreeName, captureStem, env = process.env } = options;
  const authority = validateWorkflowRequest(request);
  if (request.operation !== "change_create" || repositoryIndex !== 23 || !captureStem ||
      typeof reviewedSnapshot !== "string" || reviewedSnapshot.length !== 16 || !/^SNP-[0-9A-F]{12}$/.test(reviewedSnapshot) ||
      request.fork_snapshot_id === reviewedSnapshot) throw new Error("Wrong-fork case requires the exact initial WT capture and a different requested fork");
  const inspect = () => {
    const task = commandJson(ait, ["task", "show", request.task_id, ...authorityArgv(authority), "--json"], repository, "wrong-fork native Task", env);
    if (task.task_id !== request.task_id || task.repo_name !== "ait-web-test" || task.status !== "active") throw new Error("Wrong-fork Task context changed");
    const worktree = commandJson(ait, ["worktree", "show", worktreeName, "--json"], repository, "wrong-fork native worktree", env);
    if (worktree.bound_task_id !== request.task_id || worktree.bound_change_ref !== null || worktree.head_snapshot_id !== reviewedSnapshot)
      throw new Error("Wrong-fork case lost the exact initial Task head or binding");
    validateCaptureAuthority(worktree, authority, repositoryIndex);
    const changes = commandJson(ait, ["change", "list", "--all", ...authorityArgv(authority), "--json"], repository, "wrong-fork native Changes", env);
    if (!Array.isArray(changes) || changes.some(change => change.task_id === request.task_id)) throw new Error("Wrong-fork case must preserve a Task without revision Changes");
  };
  inspect();
  try {
    const result = await performPersonalWorkflowBrowserAction(options, reviewedSnapshot);
    inspect();
    return result;
  } catch (error) {
    if (existsSync(captureStem + ".request.stdout")) error.retainBrowserFixture = true;
    throw error;
  }
}

async function performPersonalWorkflowBrowserAction({ page, base, request, generation, repositoryIndex,
  worktreeName, ait, repository, diagnosticsDir, captureStem, env = process.env }, rejectedFork) {
  if (generation !== "current") throw new Error("Current workflow UI requires verified current native contracts");
  const authority = validateWorkflowRequest(request);
  if (diagnosticsDir) {
    if (!path.isAbsolute(diagnosticsDir)) throw new Error("Browser diagnostics directory must be absolute");
    mkdirSync(diagnosticsDir, { recursive: true, mode: 0o700 });
  }
  if (captureStem) {
    if (!path.isAbsolute(captureStem)) throw new Error("Browser case capture stem must be absolute");
    for (const suffix of [".confirmation.stdout", ".request.stdout", ".stdout", ".headers"]) {
      if (existsSync(captureStem + suffix)) throw new Error("Browser case already contains an attempt; inspect its original captures and native state without repeating the action");
    }
    mkdirSync(path.dirname(captureStem), { recursive: true, mode: 0o700 });
  }
  if (request.operation === "snapshot_create") {
    const worktree = commandJson(ait, ["worktree", "show", worktreeName, "--json"], repository, "pre-capture native worktree read", env);
    if (worktree.bound_task_id !== request.task_id || !Object.hasOwn(worktree, "bound_change_ref") ||
        (worktree.bound_change_ref !== null && worktree.bound_change_ref !== request.change_ref)) throw new Error("Capture worktree changed its exact authoring binding");
    validateCaptureAuthority(worktree, authority, repositoryIndex);
    const task = commandJson(ait, ["task", "show", request.task_id, ...authorityArgv(authority), "--json"], repository, "pre-capture exact native Task", env);
    if (task.task_id !== request.task_id || task.status !== "active" || typeof task.repo_name !== "string" || !task.repo_name)
      throw new Error("Capture requires the exact active native Task");
    const changes = commandJson(ait, ["change", "list", "--all", ...authorityArgv(authority), "--json"], repository, "pre-capture Task-owned Changes", env);
    if (!Array.isArray(changes)) throw new Error("Capture Task-owned Changes are unavailable");
    const owned = changes.filter(change => change.task_id === request.task_id);
    if (request.change_ref === null) {
      if (owned.length) throw new Error("Task has revision Changes; capture the explicit reviewed Change");
    } else {
      const selected = owned.filter(change => change.change_ref === request.change_ref);
      if (selected.length !== 1 || selected[0].repo_name !== task.repo_name ||
          !["draft", "active", "review"].includes(selected[0].status))
        throw new Error("Capture requires one exact writable Task-owned Change");
    }
  }
  const form = page.locator(`[data-current-workflow][data-operation="${request.operation}"][data-task="${request.task_id}"]`);
  if (await form.count() !== 1) throw new Error("One exact current workflow form is required");
  const candidate = page.locator("[data-current-candidate]");
  if (request.operation.startsWith("workflow_")) {
    if (await form.getAttribute("data-candidate-kind") !== request.candidate_kind) throw new Error("Workflow form changed its candidate kind");
    const destination = candidate.locator("[data-destination]");
    if (await destination.count()) await destination.selectOption(request.remote);
    else if (await form.getAttribute("data-remote") !== request.remote) throw new Error("Workflow form changed its selected Remote");
    await candidate.locator("[data-candidate-pin]").fill(request.expected_identity === null ? "" : JSON.stringify(request.expected_identity));
    await form.locator('[name="apply"]').setChecked(request.apply);
  } else if (await form.getAttribute("data-scope") !== authority.scope ||
      (await form.getAttribute("data-remote") || null) !== authority.remote) throw new Error("Workflow form changed its explicit authority");
  for (const name of ["message", "title", "fork_snapshot_id", "confirmation", "summary", "reviewer"]) {
    const control = form.locator(`[name="${name}"]`);
    if (await control.count()) await control.fill(request[name] ?? "");
  }
  if (request.operation === "snapshot_create") await form.locator('[name="change_ref"]').selectOption(request.change_ref ?? "");
  const reviewDetails = form.locator("details");
  if (await reviewDetails.count() && await reviewDetails.getAttribute("open") === null) await reviewDetails.locator("summary").click();
  for (const name of ["reviewed_files", "findings", "risks", "tests", "recommendation"]) {
    const control = form.locator(`[name="${name}"]`);
    if (await control.count()) await control.fill(request.review?.[name] ?? "");
  }
  let submitted = 0, dialog, captureError;
  const preserve = (suffix, value) => {
    if (!captureStem) return;
    try { writeFileSync(captureStem + suffix, value(), { mode: 0o600, flag: "wx" }); }
    catch (error) { captureError ??= error; }
  };
  const observe = native => {
    const uri = new URL(native.url());
    if (native.method() === "POST" && uri.origin === new URL(base).origin && uri.pathname === "/api/actions/workflow") {
      submitted++;
      if (submitted === 1) preserve(".request.stdout", () => native.postData() ?? "");
    }
  };
  page.on("request", observe);
  try {
    writeSync(1, "Personal browser workflow dispatch prepared; do not repeat this attempt\n");
    const confirmation = page.waitForEvent("dialog", { timeout: 15000 });
    const responsePromise = page.waitForResponse(response => response.request().method() === "POST" &&
      new URL(response.url()).origin === new URL(base).origin && new URL(response.url()).pathname === "/api/actions/workflow", { timeout: 0 }).then(response => {
      if (submitted === 0) preserve(".request.stdout", () => response.request().postData() ?? "");
      preserve(".headers", () => `HTTP ${response.status()}\nLocation: ${response.headers().location ?? "<absent>"}\n`);
      return response;
    });
    responsePromise.catch(() => {});
    const click = form.locator('button[type="submit"]').click({ timeout: 15000 }); click.catch(() => {});
    dialog = await confirmation;
    const text = dialog.message();
    if (dialog.type() !== "confirm" || !isDeepStrictEqual(JSON.parse(text.slice(text.indexOf("{"), text.lastIndexOf("}") + 1)), request))
      throw new Error("Native confirmation changed the exact workflow request");
    if (captureStem) writeFileSync(captureStem + ".confirmation.stdout", text, { mode: 0o600, flag: "wx" });
    await dialog.accept(); dialog = undefined; await click;
    const response = await responsePromise, observed = response.request();
    const raw = await response.text();
    writeSync(1, `HTTP ${response.status()}\n${raw}\n`);
    preserve(".stdout", () => raw);
    if (captureError) throw captureError;
    if (new URL(observed.url()).search || submitted !== 1 || !isDeepStrictEqual(observed.postDataJSON(), request))
      throw new Error("Observed workflow request differs from the confirmed JSON-only action");
    const http = { status: response.status(), location: response.headers().location, body: JSON.parse(raw) };
    if (rejectedFork !== undefined) {
      const body = currentReviewedForkRejection(http, request, rejectedFork);
      return { status: response.status(), body };
    }
    const body = currentWorkflowReceipt(http, request, repositoryIndex);
    const output = form.locator("[data-current-output]");
    await output.locator("a").waitFor({ state: "visible" });
    if (await output.locator("a").getAttribute("href") !== body.inspect_path) throw new Error("Workflow Inspect differs from the verified receipt");
    return { status: response.status(), body };
  } catch (error) {
    await dialog?.dismiss().catch(() => {});
    if (submitted) error.retainBrowserFixture = true;
    if (diagnosticsDir) {
      await page.screenshot({ path: path.join(diagnosticsDir, "native-workflow-failure.png") }).catch(() => {});
      writeFileSync(path.join(diagnosticsDir, "native-workflow.stderr"), `${error.stack}\nurl=${page.url()}\nobserved requests=${submitted}\n`, { mode: 0o600 });
    }
    throw error;
  } finally { page.off("request", observe); }
}

export function cleanupBrowserTaskScenario({ ait, repository, taskId, authority, worktreePath, expectedFinalState, env = process.env }) {
  if (!["abandoned", "completed"].includes(expectedFinalState)) throw new Error("Browser scenario final state is invalid");
  const current = commandJson(ait, ["task", "show", taskId, ...authorityArgv(authority), "--json"], repository, "browser scenario final Task readback", env);
  if (current.task_id !== taskId) throw new Error("Browser scenario final Task identity changed");
  if (current.status === expectedFinalState) {
    if (existsSync(worktreePath)) throw new Error("Browser scenario terminal Task still owns its worktree");
    return expectedFinalState;
  }
  if (expectedFinalState !== "abandoned" || current.status !== "active") {
    throw new Error("Browser lifecycle has not reached its expected native final state; retain the Task for its original handoff");
  }
  const abandoned = commandJson(ait, ["task", "abandon", taskId, ...authorityArgv(authority), "--json"], repository, "same-authority browser Task cleanup", env);
  if (abandoned.task_id !== taskId || abandoned.status !== "abandoned" || existsSync(worktreePath)) {
    throw new Error("Browser-created Task cleanup is incomplete in its selected authority");
  }
  return "abandoned";
}

export function validateReviewerBrowserRequest(request) {
  const authority = validateWorkflowRequest(request);
  if (request.operation === "task_finish") return authority;
  if (request.operation !== "workflow_finish" || request.apply !== true || authority.remote !== "origin" ||
      request.expected_identity?.repository_index !== 23) {
    throw new Error("WT Reviewer operation must finish the original Repository 23 candidate on origin");
  }
  if (!request.reviewer?.trim() || !request.review) throw new Error("WT Reviewer must supply its own review input");
  return authority;
}

export function validateWTReviewerSession(contacts, sessionId) {
  const reviewer = contacts?.members?.filter(member => member.agent_index === 1 && member.role_index === 2 && member.role_state === "active") ?? [];
  if (contacts.registered !== true || reviewer.length !== 1 || !sessionId || reviewer[0].session_id !== sessionId) {
    throw new Error("WT Reviewer operation requires its actual bound session");
  }
}

export function validateDeveloperBrowserRequest(request) {
  const authority = validateWorkflowRequest(request);
  if (!["snapshot_create", "change_create", "workflow_ready"].includes(request.operation) ||
      (authority.scope === "remote" && authority.remote !== "origin") ||
      (request.operation === "workflow_ready" && request.expected_identity !== null && request.expected_identity.repository_index !== 23)) {
    throw new Error("WT Developer operation must capture, revise or prepare the original Repository 23 candidate");
  }
  return authority;
}

export function validateWTDeveloperSession(contacts, sessionId) {
  const developer = contacts?.members?.filter(member => member.agent_index === 2 && member.role_index === 1 && member.role_state === "active") ?? [];
  if (contacts.registered !== true || developer.length !== 1 || !sessionId || developer[0].session_id !== sessionId) {
    throw new Error("WT Developer operation requires its actual bound session");
  }
}

// Both exported bridges inherit the genuine actor's native process context.
// Their callers cannot choose the role or override the child environment.
async function runPersonalActorBrowserOperation({ inputPath, repository, request, worktreeName, captureStem, chrome, playwrightModule }, actor) {
  const authority = actor === "reviewer" ? validateReviewerBrowserRequest(request) : validateDeveloperBrowserRequest(request);
  if (actor === "developer" && request.operation === "snapshot_create" && (typeof worktreeName !== "string" || !worktreeName.trim())) {
    throw new Error("WT Developer capture requires the original native worktree name");
  }
  for (const [file, label] of [[inputPath, "Web input"], [chrome, "Chrome"], [playwrightModule, "Playwright module"]]) {
    if (!path.isAbsolute(file ?? "") || !existsSync(file)) throw new Error(`${label} must be an existing absolute path`);
  }
  const input = JSON.parse(readFileSync(inputPath, "utf8"));
  validateWebOperation(inputPath, input, repository);
  if (!webPersonalInScope(input)) throw new Error("WT browser actor requires the bound Personal program");
  const root = path.dirname(path.resolve(inputPath));
  if (!path.isAbsolute(captureStem ?? "") || !path.resolve(captureStem).startsWith(root + path.sep)) {
    throw new Error("WT browser actor captures must remain in the original Web operation");
  }
  const ait = input.executables.ait.path, personalBin = input.executables.personal.path;
  for (const [name, file] of [["ait", ait], ["personal", personalBin]]) {
    executable(file, name);
    if (sha256(file) !== input.executables[name].sha256) throw new Error("WT browser actor program differs from the frozen input");
  }
  const config = commandJson(ait, ["config", "show", "--json"], repository, "WT browser actor Repository");
  currentControls(config);
  const status = commandJson(ait, ["status", "--json"], repository, "WT browser actor canonical status");
  if (config.repo_name !== "ait-web-test" || config.repository_index !== 23 || config.id_namespace_prefix?.value !== "WT" ||
      config.default_line !== "main" || status.repo_name !== "ait-web-test" || status.line_name !== "main" ||
      status.worktree !== null || status.workspace?.dirty !== false) {
    throw new Error("WT browser actor must use the persistent clean Repository 23 context");
  }
  validateBrowserRuntimeHead(status.head_snapshot_id, input, repository);
  if (webAcceptanceTree(path.join(repository, "acceptance")).sha256 !== input.web_test.acceptance_set_sha256) {
    throw new Error("WT browser actor acceptance tree differs from the frozen input");
  }
  const contacts = commandJson(ait, ["team", "contacts", "--json"], repository, "WT browser actor session binding");
  if (actor === "reviewer") validateWTReviewerSession(contacts, process.env.CODEX_THREAD_ID);
  else validateWTDeveloperSession(contacts, process.env.CODEX_THREAD_ID);
  if (authority.scope === "remote") {
    const remotes = commandJson(ait, ["remote", "list", "--json"], repository, "WT browser actor origin URL");
    const selected = Array.isArray(remotes) ? remotes.filter(remote => remote.name === "origin") : [];
    if (selected.length !== 1 || new URL(selected[0].url).href !== new URL(input.server.url).href) throw new Error("WT browser actor origin differs from the bound Server");
    const remote = commandJson(ait, ["repo", "show", "--remote", "origin", "--json"], repository, "WT browser actor named authority");
    if (remote.repository?.repository_index !== 23 || remote.repository.repository_name !== "ait-web-test" ||
        remote.repository.namespace !== "WT" || remote.repository.tombstoned !== false) throw new Error("WT browser actor origin identity differs");
  }
  validateCurrentDoctor(config, commandJson(personalBin, ["doctor", "--repository", repository, "--ait", ait, "--json"], repository, "WT browser actor current doctor"));
  const inspectAuthority = request.candidate_kind === "package" ? explicitAuthority("local") : authority;
  const task = commandJson(ait, ["task", "show", request.task_id, ...authorityArgv(inspectAuthority), "--json"], repository, "WT browser actor exact Task");
  if (task.task_id !== request.task_id || task.repo_name !== "ait-web-test" ||
      (request.candidate_kind === "package" && task.status !== "completed") ||
      (actor === "developer" && request.candidate_kind !== "package" && task.status !== "active")) throw new Error("WT browser actor Task context differs");
  let personal, browser;
  try {
    personal = spawn(personalBin, ["serve", "--repository", repository, "--ait", ait, "--json"], {
      cwd: repository, stdio: ["ignore", "pipe", "pipe"],
    });
    const ready = await waitForJson(personal.stdout, "WT browser actor Personal", 15_000,
      value => value.contract === "ait.web.personal.serve-ready.v1");
    browser = await openNativeBrowser({ helper: path.join(repository, "acceptance/headless-browser.mjs"),
      playwrightModule, chrome, diagnosticsDir: captureStem + ".diagnostics" });
    const page = await browser.context.newPage();
    await page.goto(ready.launch_url, { waitUntil: "domcontentloaded" });
    const base = new URL(ready.launch_url).origin;
    await page.goto(scopedReadUrl(base, `/tasks/${encodeURIComponent(request.task_id)}`, inspectAuthority), { waitUntil: "domcontentloaded" });
    return await runPersonalWorkflowBrowserAction({ page, base, request, generation: "current", repositoryIndex: 23,
      ait, repository, worktreeName, captureStem, diagnosticsDir: captureStem + ".diagnostics" });
  } finally {
    try { await browser?.close(); } finally { await terminate(personal); }
  }
}

export async function runPersonalReviewerBrowserOperation(options) {
  return runPersonalActorBrowserOperation(options, "reviewer");
}

export async function runPersonalDeveloperBrowserOperation(options) {
  return runPersonalActorBrowserOperation(options, "developer");
}

export async function runPersonalTaskStartBrowserScenario({ page, base, authority, ait, repository, generation, source, intent, diagnosticsDir, captureStem, onCreated, expectedFinalState = "abandoned", env = process.env }) {
  if (!page?.getByRole || !page?.waitForEvent) throw new Error("WT's supported Playwright page is required for real UI actions");
  if (!["abandoned", "completed"].includes(expectedFinalState)) throw new Error("Browser scenario final state is invalid");
  if (diagnosticsDir) {
    if (!path.isAbsolute(diagnosticsDir)) throw new Error("Browser diagnostics directory must be absolute");
    mkdirSync(diagnosticsDir, { recursive: true, mode: 0o700 });
  }
  let requestCount = 0, cleanupConfirmed = false;
  const observe = request => {
    const uri = new URL(request.url());
    if (uri.origin === new URL(base).origin && uri.pathname === "/api/actions/task-start" && request.method() === "POST") requestCount++;
  };
  page.on("request", observe);
  let scenarioSucceeded = false;
  try {
  const started = await runPersonalTaskStartBrowserAction({ page, base, authority, ait, repository, generation, source, env, intent, captureStem });
  if (!started.outcome.created) {
    const existing = commandJson(ait, ["task", "show", started.outcome.taskId, ...authorityArgv(authority), "--json"], repository, "reconciled exact-authority Task readback", env);
    await inspectBrowserTask(page, base, started, existing);
    throw new Error("Browser Task start returned a reconciled 409; Inspect remains available, creation did not pass and is not retried");
  }
  const task = commandJson(ait, ["task", "show", started.outcome.taskId, ...authorityArgv(authority), "--json"], repository, "browser-created Task readback", env);
  if (task.task_id !== started.outcome.taskId || task.plan_item_ref !== started.receipt.plan_item_ref || task.intent !== started.request.intent ||
      task.title !== started.receipt.title || task.status !== "active") throw new Error("Browser-created Task has no matching native authority; retain it for reconciliation");
  try {
    const worktree = commandJson(ait, ["worktree", "show", started.receipt.worktree.name, "--json"], repository, "browser-created worktree readback", env);
    if (worktree.bound_task_id !== task.task_id || worktree.bound_change_ref !== started.receipt.change_ref || worktree.name !== started.receipt.worktree.name ||
        worktree.open_path !== started.receipt.worktree.open_path || !existsSync(worktree.open_path)) {
      throw new Error("Browser-created worktree does not match the native Task binding");
    }
    await inspectBrowserTask(page, base, started, task);
    if (onCreated) await onCreated({ page, task, worktree, receipt: started.receipt });
    if (requestCount !== 1) throw new Error("Task-start UI attempted another creation request");
    scenarioSucceeded = true;
    return { task, worktree, receipt: started.receipt };
  } finally {
    if (scenarioSucceeded) {
      const finalState = cleanupBrowserTaskScenario({ ait, repository, taskId: task.task_id, authority,
        worktreePath: started.receipt.worktree.open_path, expectedFinalState, env });
      cleanupConfirmed = true;
      const inspectUrl = new URL(started.outcome.inspectPath, base).href;
      const cleanupRead = await page.goto(inspectUrl, { waitUntil: "load" });
      if (cleanupRead?.status() !== 200 || page.url() !== inspectUrl) {
        throw new Error(`Native ${finalState} state is confirmed, but the same-authority Task UI read failed: HTTP ${cleanupRead?.status() ?? "unavailable"}; no mutation was retried`);
      }
      await page.getByRole("heading", { name: task.task_id, exact: true }).waitFor({ state: "visible" });
      if (!(await page.locator("body").innerText()).includes(finalState)) {
        throw new Error(`Native ${finalState} state is confirmed, but its exact Task UI did not render that state`);
      }
    }
  }
  } catch (error) {
    if (requestCount > 0 && !cleanupConfirmed) error.retainBrowserFixture = true;
    if (diagnosticsDir) {
      await page.screenshot({ path: path.join(diagnosticsDir, "native-ui-failure.png"), fullPage: false }).catch(() => {});
      writeFileSync(path.join(diagnosticsDir, "native-ui.stderr"),
        `${error.stack}\nurl=${page.url()}\nobserved Task-start requests=${requestCount}\ncleanup confirmed=${cleanupConfirmed}\n`, { mode: 0o600 });
    }
    throw error;
  } finally { page.off("request", observe); }
}

async function main() {
const cli = parseCli(process.argv.slice(2));
const repository = absolute(cli["--repository"], "repository");
const ait = absolute(cli["--ait"], "ait");
const personalBin = cli["--personal-bin"] ? absolute(cli["--personal-bin"], "personal binary") : null;
const communityBin = absolute(cli["--community-bin"], "community binary");
const communityCliBin = absolute(cli["--community-cli-bin"], "community CLI binary");
const chrome = absolute(cli["--chrome"], "Chrome");
const output = absolute(cli["--output"], "output");
await enterDerivedOperation();
const mode = cli["--mode"];
if (!new Set(["solo_local", "solo_remote"]).has(mode)) fail("browser evidence mode is invalid", 64);
const authority = explicitAuthority(cli["--scope"], cli["--remote"]);
if ((mode === "solo_local") !== (authority.scope === "local")) fail("browser scenario and explicit authority disagree", 64);
if (existsSync(output)) fail("browser evidence output already exists", 73);
const operationPlanPath = path.join(process.env.AIT_RELEASE_OPERATION_ROOT, "conductor-plan.json");
if (!personalBin || existsSync(operationPlanPath)) {
  if (!existsSync(operationPlanPath)) fail("browser evidence HOLD: retained scope requires its original operation/input binding", 75);
  const plan = JSON.parse(readFileSync(operationPlanPath, "utf8"));
  const binding = plan.phases.find(row => row.id === "web_admission")?.actions.find(row => row.id === "bind-web-admission-input");
  const outputPositions = binding?.argv.flatMap((arg, index) => arg === "--output" ? [index] : []) ?? [];
  if (outputPositions.length !== 1) fail("browser evidence HOLD: original input invocation is unavailable", 75);
  const inputPath = binding.argv[outputPositions[0] + 1], input = JSON.parse(readFileSync(inputPath, "utf8"));
  validateWebOperation(inputPath, input, repository);
  if (webPersonalInScope(input) !== Boolean(personalBin)) fail("browser evidence HOLD: selected scope differs from the bound input", 75);
  if (output !== path.resolve(path.dirname(inputPath), input.browser_evidence[mode].path) || cli["--server-url"] !== input.server.url) fail("browser evidence HOLD: output or Server differs from the bound invocation", 75);
  for (const [key, file] of [["ait", ait], ["community", communityBin], ["community_cli", communityCliBin], ...(personalBin ? [["personal", personalBin]] : [])]) {
    if (input.executables[key]?.path !== file || input.executables[key].sha256 !== sha256(file)) fail("browser evidence HOLD: executable differs from the bound input", 75);
  }
}

for (const [file, label] of [[ait, "ait"], [personalBin, "Personal binary"], [communityBin, "Community binary"], [communityCliBin, "Community CLI binary"], [chrome, "Chrome"]].filter(([file]) => file !== null)) executable(file, label);

const config = commandJson(ait, ["config", "show", "--json"], repository, "AIT configuration readback");
const status = commandJson(ait, ["status", "--json"], repository, "AIT status readback");
const namespace = config.id_namespace_prefix?.value;
currentControls(config);
if (authority.scope === "remote") {
  const remotes = commandJson(ait, ["remote", "list", "--json"], repository, "configured browser remote");
  const selected = Array.isArray(remotes) ? remotes.filter(row => row.name === authority.remote) : [];
  if (selected.length !== 1 || new URL(selected[0].url).href !== new URL(cli["--server-url"]).href) fail("browser remote is not the bound private Server");
  const remote = commandJson(ait, ["repo", "show", ...authorityArgv(authority), "--json"], repository, "selected browser Repository");
  if (remote.repository?.repository_index !== 23 || remote.repository.repository_name !== "ait-web-test" ||
      remote.repository.namespace !== "WT" || remote.repository.tombstoned !== false) fail("browser remote Repository identity differs");
}
if (personalBin) validateCurrentDoctor(config, commandJson(personalBin, ["doctor", "--repository", repository, "--ait", ait, "--json"], repository, "Personal exact current doctor"));
if (
  config.repository_index !== 23 || config.repo_name !== "ait-web-test" ||
  namespace !== "WT" || config.default_line !== "main" || status.repo_name !== "ait-web-test" ||
  status.line_name !== "main" || status.workspace?.dirty !== false || status.worktree !== null
) fail("browser evidence repository identity or mode is not admitted");
if (!/^SNP-[0-9A-F]{12}$/.test(status.head_snapshot_id ?? "")) fail("browser evidence head Snapshot is invalid");

// LNAT-0021 still needs the actual B–D browser scenarios and their independent
// Reviewer/backend handoff. The existing renderer/Task-start path covers A.
// Keep this explicit hold until those executions are wired and reviewed; no
// candidate flag, fixture result or A-only receipt can enable the pass emitter.
if (personalBin) fail("Personal B–D browser execution is not yet integrated: Task capture/correction, Local/formal/package finish and original-pin Reviewer/backend evidence are required; current browser pass is unavailable", 75);

const temporaryRoot = mkdtempSync(path.join(tmpdir(), "ait-release-browser."));
const children = [];
let personal;
let community;
let browser;
try {
  let personalReady, personalRoot;
  if (personalBin) {
  personal = spawn(personalBin, ["serve", "--repository", repository, "--ait", ait, "--json"], {
    cwd: repository,
    stdio: ["ignore", "pipe", "pipe"],
  });
  children.push(personal);
  personalReady = await waitForJson(
    personal.stdout,
    "Personal",
    15_000,
    (value) => value.contract === "ait.web.personal.serve-ready.v1",
  );
  personalRoot = new URL(personalReady.launch_url);
  personalRoot.pathname = "/";
  personalRoot.search = "";
  personalRoot.hash = "";
  }

  const communityPort = await unusedPort();
  const communityConfig = path.join(temporaryRoot, "community.json");
  writeFileSync(communityConfig, `${JSON.stringify({
    schema_version: 1,
    listen: `127.0.0.1:${communityPort}`,
    upstream_server_url: cli["--server-url"],
    server_client: { request_timeout_ms: 5000, max_response_bytes: 1048576 },
    secrets: {},
    repositories: [{ index: 23, name: "ait-web-test", namespace: "WT" }],
    trusted_proxies: [],
  }, null, 2)}\n`, { mode: 0o600 });
  community = spawn(communityBin, ["serve", "--config", communityConfig], {
    cwd: repository,
    stdio: ["ignore", "ignore", "pipe"],
  });
  children.push(community);
  await waitHttp(`http://127.0.0.1:${communityPort}/healthz`, community, "Community");

  browser = await openNativeBrowser({
    helper: path.join(repository, "acceptance/headless-browser.mjs"),
    playwrightModule: cli["--playwright"] ?? process.env.AIT_WEB_TEST_PLAYWRIGHT_MODULE,
    chrome, diagnosticsDir: `${output}.diagnostics`,
  });
  const screenshots = output.slice(0, -path.extname(output).length);
  const personalObs = { console: [], failedRequests: [], externalRequests: [] };
  let personalDesktop, personalMobile;
  if (personalBin) {
  const personalPage = await browser.context.newPage();
  observePage(personalPage, personalObs, personalRoot.origin);
  personalDesktop = await inspect(personalPage, personalReady.launch_url,
    { width: 1280, height: 720 }, `${screenshots}.personal-desktop.png`);
  personalMobile = await inspect(personalPage, personalRoot.href,
    { width: 390, height: 844 }, `${screenshots}.personal-mobile-390x844.png`);
  await runPersonalTaskStartBrowserScenario({ page: personalPage, base: personalRoot.href, authority, ait, repository, generation: "current", diagnosticsDir: `${output}.diagnostics` });
  await personalPage.close();
  }

  const communityObs = { console: [], failedRequests: [], externalRequests: [] };
  const communityPage = await browser.context.newPage();
  const communityRoot = `http://127.0.0.1:${communityPort}`;
  observePage(communityPage, communityObs, communityRoot);
  const communityDesktop = await inspect(communityPage, `${communityRoot}/`,
    { width: 1280, height: 720 }, `${screenshots}.community-desktop.png`);
  const communityMobile = await inspect(communityPage, `${communityRoot}/actions`,
    { width: 390, height: 844 }, `${screenshots}.community-mobile-390x844.png`);
  await communityPage.close();

  const personalText = personalBin ? `${personalDesktop.text}\n${personalMobile.text}` : "";
  const communityText = `${communityDesktop.text}\n${communityMobile.text}`;
  const personalPass = !personalBin || personalText.includes("ait-web-test") && personalText.includes(status.head_snapshot_id) &&
    renderedControls(personalDesktop.definitions, config) && renderedControls(personalMobile.definitions, config) &&
    personalDesktop.maxOverflowPx === 0 && personalMobile.maxOverflowPx === 0 && personalObs.console.length === 0 &&
    personalObs.failedRequests.length === 0 && personalObs.externalRequests.length === 0;
  const communityPass = communityMobile.actionRows === 25 && communityMobile.buttons === 0 && communityMobile.forms === 0 &&
    communityMobile.actionLinks === 0 && !/ait-web-test|127\.0\.0\.1:8088|\bWT\b/.test(communityText) &&
    communityDesktop.maxOverflowPx === 0 && communityMobile.maxOverflowPx === 0 && communityObs.console.length === 0 &&
    communityObs.failedRequests.length === 0 && communityObs.externalRequests.length === 0;
  if (!personalPass || !communityPass) {
    writeFileSync(`${output}.failure.json`, `${JSON.stringify({
      personalPass,
      communityPass,
      personal: { desktop: personalDesktop, mobile: personalMobile, observations: personalObs },
      community: { desktop: communityDesktop, mobile: communityMobile, observations: communityObs },
    }, null, 2)}\n`, { mode: 0o600 });
    throw new Error(`browser evidence assertions failed; see ${output}.failure.json`);
  }

  const evidence = {
    contract: "ait-web-test.browser-evidence.v1",
    mode,
    captured_at: new Date().toISOString(),
    repository: {
      head_snapshot_id: status.head_snapshot_id,
      index: 23,
      line: "main",
      name: "ait-web-test",
      namespace: "WT",
      workspace: "clean",
    },
    ...(personalBin ? { personal: {
      binary_sha256: sha256(personalBin),
      browser_console: "pass",
      browser_desktop: "pass",
      "browser_mobile-390x844": "pass",
      console_warning_or_error_count: personalObs.console.length,
      failed_request_count: personalObs.failedRequests.length,
      external_request_count: personalObs.externalRequests.length,
      repository_authority_rendered: `ait-web-test/23/WT/main/${status.head_snapshot_id}`,
      responsive: {
        desktop_max_overflow_px: personalDesktop.maxOverflowPx,
        document_horizontal_overflow: false,
        mobile_max_overflow_px: personalMobile.maxOverflowPx,
        role_rail_overflow: "intentional-auto-scroll",
      },
      routes: ["/"],
      workflow_mode_rendered: mode, // Historical key: the verified scenario label, never a native config mode.
    } } : {}),
    community: {
      binary_sha256: sha256(communityBin),
      browser_console: "pass",
      browser_desktop: "pass",
      "browser_mobile-390x844": "pass",
      catalog: {
        buttons: communityMobile.buttons,
        definition_count: communityMobile.actionRows,
        enabled_count: 0,
        executable_action_links: communityMobile.actionLinks,
        filesystem_path_leak: false,
        forms: communityMobile.forms,
        repository_identity_leak: false,
        upstream_server_url_leak: false,
      },
      companion_binary_sha256: sha256(communityCliBin),
      console_warning_or_error_count: communityObs.console.length,
      failed_request_count: communityObs.failedRequests.length,
      external_request_count: communityObs.externalRequests.length,
      responsive: {
        desktop_max_overflow_px: communityDesktop.maxOverflowPx,
        document_horizontal_overflow: false,
        mobile_max_overflow_px: communityMobile.maxOverflowPx,
        role_rail_overflow: "intentional-auto-scroll",
      },
      routes: ["/", "/actions"],
    },
  };
  writeFileSync(output, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600, flag: "wx" });
  process.stdout.write(`${mode}: browser evidence passed\n`);
} catch (error) {
  process.stderr.write((error instanceof Error ? error.message : String(error)) + "\n");
  process.exitCode = 65;
} finally {
  try { await browser?.close(); } finally {
    for (const child of children.reverse()) await terminate(child);
  }
  rmSync(temporaryRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { process.stderr.write(error.message + "\n"); process.exitCode = 65; });
}
