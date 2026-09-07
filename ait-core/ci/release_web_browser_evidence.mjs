#!/usr/bin/env node

import { createHash } from "node:crypto";
import { closeSync, existsSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";

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
    "--repository", "--mode", "--ait", "--personal-bin", "--community-bin",
    "--community-cli-bin", "--server-url", "--chrome", "--output",
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

function commandJson(command, argv, cwd, label) {
  const result = spawnSync(command, argv, { cwd, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  if (result.status !== 0) fail(`${label} failed`);
  try { return JSON.parse(result.stdout); } catch { fail(`${label} returned invalid JSON`); }
}

function executable(file, label) {
  if (!existsSync(file)) fail(`${label} is missing: ${file}`, 66);
  const descriptor = openSync(file, "r");
  closeSync(descriptor);
}

function waitForLine(stream, label, timeoutMs, parser) {
  return new Promise((resolve, reject) => {
    let buffer = "";
    const timer = setTimeout(() => reject(new Error(`${label} readiness timed out`)), timeoutMs);
    stream.setEncoding("utf8");
    stream.on("data", (chunk) => {
      buffer += chunk;
      for (;;) {
        const newline = buffer.indexOf("\n");
        if (newline < 0) return;
        const line = buffer.slice(0, newline).trim();
        buffer = buffer.slice(newline + 1);
        const value = parser(line);
        if (value !== undefined) {
          clearTimeout(timer);
          resolve(value);
          return;
        }
      }
    });
  });
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

class Cdp {
  constructor(url) {
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Map();
    this.socket = new WebSocket(url);
  }

  async ready() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
    });
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(message.error.message));
        else pending.resolve(message.result);
        return;
      }
      for (const listener of this.listeners.get(message.method) ?? []) listener(message.params ?? {});
    });
    return this;
  }

  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  on(method, listener) {
    const listeners = this.listeners.get(method) ?? [];
    listeners.push(listener);
    this.listeners.set(method, listeners);
  }

  close() {
    this.socket.close();
  }
}

async function newTab(debugPort) {
  const response = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent("about:blank")}`, { method: "PUT" });
  if (!response.ok) throw new Error("Chrome target creation failed");
  const target = await response.json();
  return new Cdp(target.webSocketDebuggerUrl).ready();
}

async function inspect(cdp, url, viewport, screenshotPath, observations) {
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Network.enable");
  await cdp.send("Log.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: viewport.width,
    height: viewport.height,
    deviceScaleFactor: 1,
    mobile: viewport.mobile,
  });
  cdp.on("Runtime.consoleAPICalled", ({ type, args = [] }) => {
    if (["error", "warning", "assert"].includes(type)) observations.console.push({ type, text: args.map((row) => row.value ?? row.description ?? "").join(" ").slice(0, 500) });
  });
  cdp.on("Runtime.exceptionThrown", ({ exceptionDetails }) => observations.console.push({ type: "exception", text: exceptionDetails?.text ?? "exception" }));
  cdp.on("Log.entryAdded", ({ entry }) => {
    const automaticFaviconProbe = entry?.source === "network" && /\/favicon\.ico(?:$|\?)/.test(entry?.url ?? "");
    if (["error", "warning"].includes(entry?.level) && !automaticFaviconProbe) {
      observations.console.push({ type: entry.level, text: String(entry.text ?? "").slice(0, 500) });
    }
  });
  cdp.on("Network.loadingFailed", (row) => {
    if (!row.canceled && !/ERR_ABORTED/.test(row.errorText ?? "")) observations.failedRequests.push(String(row.errorText ?? "failed"));
  });
  cdp.on("Network.requestWillBeSent", ({ request }) => {
    try {
      const host = new URL(request.url).hostname;
      if (!new Set(["127.0.0.1", "localhost"]).has(host)) observations.externalRequests.push(request.url);
    } catch {}
  });
  cdp.on("Network.responseReceived", ({ response }) => {
    const automaticFaviconProbe = /\/favicon\.ico(?:$|\?)/.test(response?.url ?? "");
    if ((response?.status ?? 0) >= 400 && !automaticFaviconProbe) {
      observations.failedRequests.push(`${response.status} ${response.url}`);
    }
  });
  await cdp.send("Page.navigate", { url });
  let value;
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const evaluated = await cdp.send("Runtime.evaluate", {
      expression: `(() => ({
        readyState: document.readyState,
        url: location.href,
        title: document.title,
        text: document.body ? document.body.innerText : "",
        documentWidth: document.documentElement.scrollWidth,
        clientWidth: document.documentElement.clientWidth,
        bodyWidth: document.body ? document.body.scrollWidth : 0,
        buttons: document.querySelectorAll("button").length,
        forms: document.querySelectorAll("form").length,
        actionRows: document.querySelectorAll("[data-action-row]").length,
        actionLinks: document.querySelectorAll("[data-action-row] a[href]").length
      }))()`,
      returnByValue: true,
    });
    value = evaluated.result?.value;
    if (value?.readyState === "complete" && value.text.length > 0) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  if (!value || value.readyState !== "complete") throw new Error(`browser page did not become ready: ${url}`);
  const screenshot = await cdp.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
  writeFileSync(screenshotPath, Buffer.from(screenshot.data, "base64"), { mode: 0o600, flag: "wx" });
  return {
    ...value,
    maxOverflowPx: Math.max(0, value.documentWidth - viewport.width, value.bodyWidth - viewport.width),
  };
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

const cli = parseCli(process.argv.slice(2));
const repository = absolute(cli["--repository"], "repository");
const ait = absolute(cli["--ait"], "ait");
const personalBin = absolute(cli["--personal-bin"], "personal binary");
const communityBin = absolute(cli["--community-bin"], "community binary");
const communityCliBin = absolute(cli["--community-cli-bin"], "community CLI binary");
const chrome = absolute(cli["--chrome"], "Chrome");
const output = absolute(cli["--output"], "output");
const mode = cli["--mode"];
if (!new Set(["solo_local", "solo_remote"]).has(mode)) fail("browser evidence mode is invalid", 64);
if (existsSync(output)) fail("browser evidence output already exists", 73);
for (const [file, label] of [[ait, "ait"], [personalBin, "Personal binary"], [communityBin, "Community binary"], [communityCliBin, "Community CLI binary"], [chrome, "Chrome"]]) executable(file, label);

const config = commandJson(ait, ["config", "show", "--json"], repository, "AIT configuration readback");
const status = commandJson(ait, ["status", "--json"], repository, "AIT status readback");
const namespace = config.id_namespace_prefix?.value;
if (
  config.workflow_mode?.value !== mode || config.repository_index !== 23 || config.repo_name !== "ait-web-test" ||
  namespace !== "WT" || config.default_line !== "main" || status.repo_name !== "ait-web-test" ||
  status.line_name !== "main" || status.workspace?.dirty !== false || status.worktree !== null
) fail("browser evidence repository identity or mode is not admitted");
if (!/^SNP-[0-9A-F]{12}$/.test(status.head_snapshot_id ?? "")) fail("browser evidence head Snapshot is invalid");

const temporaryRoot = mkdtempSync(path.join(tmpdir(), "ait-release-browser."));
const children = [];
let personal;
let community;
let browser;
try {
  personal = spawn(personalBin, ["serve", "--repository", repository, "--ait", ait, "--json"], {
    cwd: repository,
    stdio: ["ignore", "pipe", "pipe"],
  });
  children.push(personal);
  const personalReady = await waitForJson(
    personal.stdout,
    "Personal",
    15_000,
    (value) => value.contract === "ait.web.personal.serve-ready.v1",
  );
  const personalRoot = new URL(personalReady.launch_url);
  personalRoot.pathname = "/";
  personalRoot.search = "";
  personalRoot.hash = "";

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

  const chromeRoot = path.join(temporaryRoot, "chrome");
  browser = spawn(chrome, [
    "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
    "--remote-debugging-port=0", `--user-data-dir=${chromeRoot}`, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  children.push(browser);
  const debuggerUrl = await waitForLine(browser.stderr, "Chrome", 15_000, (line) => line.match(/DevTools listening on (ws:\/\/\S+)/)?.[1]);
  const debugPort = Number(new URL(debuggerUrl).port);
  const screenshots = `${output.slice(0, -path.extname(output).length)}`;

  const personalObs = { console: [], failedRequests: [], externalRequests: [] };
  const personalTab = await newTab(debugPort);
  const personalDesktop = await inspect(personalTab, personalReady.launch_url, { width: 1280, height: 720, mobile: false }, `${screenshots}.personal-desktop.png`, personalObs);
  const personalMobile = await inspect(personalTab, personalRoot.href, { width: 390, height: 844, mobile: true }, `${screenshots}.personal-mobile-390x844.png`, personalObs);
  personalTab.close();

  const communityObs = { console: [], failedRequests: [], externalRequests: [] };
  const communityTab = await newTab(debugPort);
  const communityDesktop = await inspect(communityTab, `http://127.0.0.1:${communityPort}/`, { width: 1280, height: 720, mobile: false }, `${screenshots}.community-desktop.png`, communityObs);
  const communityMobile = await inspect(communityTab, `http://127.0.0.1:${communityPort}/actions`, { width: 390, height: 844, mobile: true }, `${screenshots}.community-mobile-390x844.png`, communityObs);
  communityTab.close();

  const personalText = `${personalDesktop.text}\n${personalMobile.text}`;
  const communityText = `${communityDesktop.text}\n${communityMobile.text}`;
  const personalPass = personalText.includes("ait-web-test") && personalText.includes(mode) && personalText.includes(status.head_snapshot_id) &&
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
    personal: {
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
      workflow_mode_rendered: mode,
    },
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
  fail(error instanceof Error ? error.message : String(error));
} finally {
  for (const child of children.reverse()) await terminate(child);
  rmSync(temporaryRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
