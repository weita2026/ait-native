import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

function parseArguments(values) {
  const parsed = new Map();
  for (let index = 0; index < values.length; index += 2) {
    const key = values[index];
    const value = values[index + 1];
    if (!key?.startsWith("--") || !value) {
      throw new Error(
        "Usage: browser-check.mjs --workload GD-01 --candidate PATH --browser PATH",
      );
    }
    parsed.set(key.slice(2), value);
  }
  for (const required of ["workload", "candidate", "browser"]) {
    if (!parsed.has(required)) {
      throw new Error(`Missing --${required}`);
    }
  }
  return parsed;
}

async function reserveLoopbackPort() {
  const server = createServer();
  await new Promise((resolveReady, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveReady);
  });
  const address = server.address();
  assert.equal(typeof address, "object");
  const port = address.port;
  await new Promise((resolveClosed, reject) =>
    server.close((error) => (error ? reject(error) : resolveClosed())),
  );
  return port;
}

async function waitFor(check, description, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await check();
      if (value) {
        return value;
      }
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error(
    `${description} timed out${lastError ? `: ${lastError.message}` : ""}`,
  );
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  const exited = once(child, "exit");
  child.kill("SIGTERM");
  await Promise.race([
    exited,
    new Promise((resolveWait) => setTimeout(resolveWait, 2_000)),
  ]);
  if (child.exitCode === null && child.signalCode === null) {
    const killed = once(child, "exit");
    child.kill("SIGKILL");
    await killed;
  }
}

async function removeProfile(path) {
  let lastError;
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      await rm(path, { recursive: true, force: true, maxRetries: 2 });
      return;
    } catch (error) {
      lastError = error;
      await new Promise((resolveWait) => setTimeout(resolveWait, 100));
    }
  }
  throw lastError;
}

class CdpClient {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Set();
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id && this.pending.has(message.id)) {
        const { resolveResponse, rejectResponse } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) {
          rejectResponse(new Error(message.error.message || JSON.stringify(message.error)));
        } else {
          resolveResponse(message.result || {});
        }
        return;
      }
      for (const listener of this.listeners) {
        listener(message);
      }
    });
  }

  send(method, params = {}, sessionId) {
    const id = this.nextId++;
    const payload = { id, method, params };
    if (sessionId) {
      payload.sessionId = sessionId;
    }
    return new Promise((resolveResponse, rejectResponse) => {
      this.pending.set(id, { resolveResponse, rejectResponse });
      this.socket.send(JSON.stringify(payload));
    });
  }

  waitEvent(method, sessionId, timeoutMs = 10_000) {
    return new Promise((resolveEvent, rejectEvent) => {
      const timer = setTimeout(() => {
        this.listeners.delete(listener);
        rejectEvent(new Error(`CDP event ${method} timed out`));
      }, timeoutMs);
      const listener = (message) => {
        if (message.method === method && (!sessionId || message.sessionId === sessionId)) {
          clearTimeout(timer);
          this.listeners.delete(listener);
          resolveEvent(message.params || {});
        }
      };
      this.listeners.add(listener);
    });
  }
}

async function inspectViewport(client, url, viewport) {
  const { targetId } = await client.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await client.send("Target.attachToTarget", {
    targetId,
    flatten: true,
  });
  const consoleErrors = [];
  const failedRequests = [];
  const eventListener = (message) => {
    if (message.sessionId !== sessionId) {
      return;
    }
    if (message.method === "Runtime.exceptionThrown") {
      consoleErrors.push(
        message.params?.exceptionDetails?.text || "uncaught browser exception",
      );
    }
    if (
      message.method === "Runtime.consoleAPICalled" &&
      message.params?.type === "error"
    ) {
      consoleErrors.push("console.error");
    }
    if (message.method === "Network.loadingFailed") {
      failedRequests.push(message.params?.errorText || "network load failed");
    }
    if (
      message.method === "Network.responseReceived" &&
      Number(message.params?.response?.status || 0) >= 400
    ) {
      failedRequests.push(
        `${message.params.response.status} ${message.params.response.url || ""}`.trim(),
      );
    }
  };
  client.listeners.add(eventListener);
  try {
    await Promise.all([
      client.send("Runtime.enable", {}, sessionId),
      client.send("Network.enable", {}, sessionId),
      client.send("Page.enable", {}, sessionId),
      client.send(
        "Emulation.setDeviceMetricsOverride",
        {
          width: viewport.width,
          height: viewport.height,
          deviceScaleFactor: 1,
          mobile: viewport.mobile,
        },
        sessionId,
      ),
    ]);
    const loaded = client.waitEvent("Page.loadEventFired", sessionId);
    await client.send("Page.navigate", { url }, sessionId);
    await loaded;
    const expression = `new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(async () => {
        const waitAnimationFrames = (count) => new Promise((resolveFrames) => {
          let remaining = count;
          const nextFrame = () => {
            remaining -= 1;
            if (remaining <= 0) {
              resolveFrames();
            } else {
              requestAnimationFrame(nextFrame);
            }
          };
          requestAnimationFrame(nextFrame);
        });
        const canvas = document.querySelector("#game");
        const start = document.querySelector("#start");
        const pause = document.querySelector("#pause");
        if (start) start.click();
        if (pause) pause.click();
        const paused = window.__AIT_GAME__?.state?.().paused;
        if (pause) pause.click();
        const resumed = window.__AIT_GAME__?.state?.().paused === false;
        const isGd05 = new URLSearchParams(location.search).get("workload") === "GD-05";
        let focusLossStopsSimulation = null;
        let focusRecoveryResumesSimulation = null;
        let focusRecoveryDeltaMs = null;
        if (isGd05) {
          const beforeFocusLossTime = Number(window.__AIT_GAME__?.state?.().timeMs);
          Object.defineProperty(document, "hasFocus", {
            configurable: true,
            value: () => false,
          });
          window.dispatchEvent(new Event("blur"));
          await waitAnimationFrames(4);
          const duringFocusLossTime = Number(window.__AIT_GAME__?.state?.().timeMs);
          focusLossStopsSimulation =
            Number.isFinite(beforeFocusLossTime) &&
            Number.isFinite(duringFocusLossTime) &&
            duringFocusLossTime === beforeFocusLossTime;

          Object.defineProperty(document, "hasFocus", {
            configurable: true,
            value: () => true,
          });
          window.dispatchEvent(new Event("focus"));
          await waitAnimationFrames(3);
          const afterFocusRecoveryTime = Number(window.__AIT_GAME__?.state?.().timeMs);
          focusRecoveryDeltaMs = afterFocusRecoveryTime - duringFocusLossTime;
          focusRecoveryResumesSimulation =
            Number.isFinite(afterFocusRecoveryTime) &&
            focusRecoveryDeltaMs > 0 &&
            focusRecoveryDeltaMs <= 200;
        }
        const canvasRect = canvas?.getBoundingClientRect();
        resolve({
          title: document.title,
          ready: Boolean(canvas && start && pause && window.__AIT_GAME__?.state),
          paused,
          resumed,
          lives: window.__AIT_GAME__?.state?.().lives,
          canvasVisible: Boolean(canvasRect && canvasRect.width > 0 && canvasRect.height > 0),
          canvasFitsViewport: Boolean(canvasRect && canvasRect.right <= innerWidth + 0.5),
          horizontalOverflow: document.documentElement.scrollWidth > innerWidth + 1,
          touchControlCount: document.querySelectorAll("[data-touch-control]").length,
          benchmarkPanelCount: document.querySelectorAll("[data-benchmark-panel]").length,
          focusLossStopsSimulation,
          focusRecoveryResumesSimulation,
          focusRecoveryDeltaMs,
        });
      }));
    })`;
    const evaluated = await client.send(
      "Runtime.evaluate",
      { expression, awaitPromise: true, returnByValue: true },
      sessionId,
    );
    if (evaluated.exceptionDetails) {
      throw new Error(evaluated.exceptionDetails.text || "browser probe evaluation failed");
    }
    const state = evaluated.result?.value;
    assert.equal(state?.ready, true, "game canvas, controls, and benchmark seam must load");
    assert.equal(state?.paused, true, "Pause control must pause the game");
    assert.equal(state?.resumed, true, "Pause control must resume the game");
    assert.equal(state?.lives, 3, "Restarted game must expose three lives");
    assert.equal(state?.canvasVisible, true, "game canvas must be visible");
    assert.equal(state?.canvasFitsViewport, true, "game canvas must fit the viewport");
    assert.equal(state?.horizontalOverflow, false, "page must not overflow horizontally");
    if (url.includes("workload=GD-05")) {
      assert.ok(state.touchControlCount >= 3, "GD-05 must expose mobile touch controls");
      assert.ok(state.benchmarkPanelCount >= 1, "GD-05 must expose the benchmark panel");
      assert.equal(
        state.focusLossStopsSimulation,
        true,
        "GD-05 must stop simulation time while window focus is lost",
      );
      assert.equal(
        state.focusRecoveryResumesSimulation,
        true,
        "GD-05 must resume without a focus-loss time jump (delta: " +
          state.focusRecoveryDeltaMs +
          ")",
      );
    }
    assert.deepEqual(consoleErrors, [], `browser console errors: ${consoleErrors.join("; ")}`);
    assert.deepEqual(failedRequests, [], `browser request failures: ${failedRequests.join("; ")}`);
    return { passed: true, state, consoleErrors, failedRequests };
  } catch (error) {
    return {
      passed: false,
      error: error instanceof Error ? error.message : String(error),
      consoleErrors,
      failedRequests,
    };
  } finally {
    client.listeners.delete(eventListener);
    await client.send("Target.closeTarget", { targetId }).catch(() => {});
  }
}

const argumentsMap = parseArguments(process.argv.slice(2));
const workloadId = argumentsMap.get("workload");
const candidate = resolve(argumentsMap.get("candidate"));
const browser = resolve(argumentsMap.get("browser"));
const port = await reserveLoopbackPort();
const profile = await mkdtemp(join(tmpdir(), "ait-agent-token-browser-"));
const serverOutput = [];
const browserOutput = [];
let server;
let chrome;

try {
  server = spawn(process.execPath, [join(candidate, "scripts/serve.mjs")], {
    cwd: candidate,
    env: { ...process.env, PORT: String(port) },
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout.on("data", (value) => serverOutput.push(String(value)));
  server.stderr.on("data", (value) => serverOutput.push(String(value)));
  const baseUrl = `http://127.0.0.1:${port}`;
  await waitFor(async () => {
    const response = await fetch(`${baseUrl}/index.html`, { cache: "no-store" });
    return response.ok;
  }, "fixture loopback server");

  chrome = spawn(
    browser,
    [
      "--headless=new",
      "--remote-debugging-port=0",
      "--remote-allow-origins=*",
      `--user-data-dir=${profile}`,
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-default-apps",
      "--disable-extensions",
      "--disable-sync",
      "--metrics-recording-only",
      "--no-first-run",
      "--no-default-browser-check",
      "--host-resolver-rules=MAP * 0.0.0.0, EXCLUDE 127.0.0.1, EXCLUDE localhost",
      "about:blank",
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  chrome.stdout.on("data", (value) => browserOutput.push(String(value)));
  chrome.stderr.on("data", (value) => browserOutput.push(String(value)));
  const devTools = await waitFor(async () => {
    const text = await readFile(join(profile, "DevToolsActivePort"), "utf8");
    const [debugPort, path] = text.trim().split("\n");
    return debugPort && path ? { debugPort, path } : null;
  }, "Chrome DevTools endpoint");
  const socket = new WebSocket(`ws://127.0.0.1:${devTools.debugPort}${devTools.path}`);
  await new Promise((resolveOpen, rejectOpen) => {
    socket.addEventListener("open", resolveOpen, { once: true });
    socket.addEventListener("error", rejectOpen, { once: true });
  });
  const client = new CdpClient(socket);
  const url = `${baseUrl}/?seed=1337&bossAfter=10&workload=${encodeURIComponent(workloadId)}`;
  const desktop = await inspectViewport(client, url, {
    width: 1280,
    height: 900,
    mobile: false,
  });
  const mobile = await inspectViewport(client, url, {
    width: 390,
    height: 844,
    mobile: true,
  });
  socket.close();
  const consoleErrors = desktop.consoleErrors.length + mobile.consoleErrors.length;
  const failedRequests = desktop.failedRequests.length + mobile.failedRequests.length;
  const horizontalOverflow = Boolean(
    desktop.state?.horizontalOverflow || mobile.state?.horizontalOverflow,
  );
  const passed = desktop.passed && mobile.passed && consoleErrors === 0 && failedRequests === 0;
  const notes = [];
  if (!desktop.passed) notes.push(`desktop: ${desktop.error}`);
  if (!mobile.passed) notes.push(`mobile: ${mobile.error}`);
  const report = {
    contract: "ait-agent-token-browser-report/v1",
    workload_id: workloadId,
    required_for_equivalent_completion: true,
    status: passed ? "passed" : "failed",
    desktop_passed: desktop.passed,
    mobile_passed: mobile.passed,
    console_errors: consoleErrors,
    failed_requests: failedRequests,
    horizontal_overflow: horizontalOverflow,
    notes,
  };
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  process.exitCode = passed ? 0 : 1;
} catch (error) {
  process.stdout.write(
    `${JSON.stringify(
      {
        contract: "ait-agent-token-browser-report/v1",
        workload_id: workloadId,
        required_for_equivalent_completion: true,
        status: "harness_error",
        desktop_passed: null,
        mobile_passed: null,
        console_errors: null,
        failed_requests: null,
        horizontal_overflow: null,
        notes: [
          error instanceof Error ? error.message : String(error),
          ...serverOutput,
          ...browserOutput,
        ],
      },
      null,
      2,
    )}\n`,
  );
  process.exitCode = 2;
} finally {
  await stopChild(chrome);
  await stopChild(server);
  await removeProfile(profile);
}
