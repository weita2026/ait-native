import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  AgentCapabilities,
  AgentClient,
  NativeProtocolError,
} from "../src/index.js";

test("capabilities come from the real Rust binding", () => {
  const capabilities = new AgentClient().capabilities();

  assert.equal(capabilities.contract, "ait.agent.worker.capabilities.v1");
  assert.ok(capabilities.version);
  assert.deepEqual(new Set(capabilities.supportedTransports), new Set([
    "telegram",
    "discord",
    "slack",
    "line",
  ]));
  assert.ok(
    capabilities.eventLoopBackends.includes(
      capabilities.defaultEventLoopBackend,
    ),
  );
  assert.equal(capabilities.raw.python_worker_execution_allowed, false);
});

for (const [field, value, message] of [
  ["contract", "wrong", /unsupported capabilities contract/],
  ["binary", "wrong", /wrong binary/],
  ["python_worker_execution_allowed", true, /forbidden Python fallback/],
]) {
  test(`capabilities fail closed for ${field}`, () => {
    const payload = {
      contract: "ait.agent.worker.capabilities.v1",
      binary: "ait-agent-worker",
      version: "0.10.6",
      platform: "test",
      architecture: "test",
      supported_transports: ["telegram"],
      event_loop_backends: ["portable_poll"],
      default_event_loop_backend: "portable_poll",
      python_worker_execution_allowed: false,
    };
    payload[field] = value;

    assert.throws(() => new AgentCapabilities(payload), message);
  });
}

test("management lists an empty manifest through the real binding", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-agent-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, ".ait"));
  const manifestPath = path.join(root, ".ait", "agent-workers.json");

  assert.deepEqual(
    new AgentClient().listWorkers("telegram", {
      repoRoot: root,
      manifestPath,
    }),
    [],
  );
});

test("reply provider uses the real worker transaction binding", () => {
  const result = new AgentClient().replyProvider({
    contract: "unsupported",
  });

  assert.equal(
    result.contract,
    "ait.agent.gateway_reply_provider_response.v1",
  );
  assert.equal(result.error.kind, "provider_request_contract");
});

test("manager and worker input validation happens before native calls", () => {
  const client = new AgentClient();

  assert.throws(
    () => client.listWorkers("email"),
    /unsupported agent transport/,
  );
  assert.throws(() => client.start("telegram", " "), /worker name/);
  assert.throws(
    () => client.workerTransaction("run-command", {}),
    /unsupported worker operation/,
  );
  assert.throws(
    () => client.replyProvider(undefined),
    /worker payload is required/,
  );
  assert.throws(
    () => client.replyProvider({}, { nowUnixSeconds: 1.5 }),
    /safe integer/,
  );
  assert.throws(
    () => client.listWorkers("telegram", { commandArgs: [] }),
    /unsupported agent options fields/,
  );
});

test("capability constructor rejects non-object payloads", () => {
  assert.throws(() => new AgentCapabilities([]), NativeProtocolError);
});
