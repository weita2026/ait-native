import assert from "node:assert/strict";
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

test("retired management surface is absent and worker input fails closed", () => {
  const client = new AgentClient();

  for (const name of [
    "manage",
    "add",
    "listWorkers",
    "status",
    "start",
    "stop",
    "restart",
    "remove",
    "logs",
  ]) {
    assert.equal(name in client, false);
  }
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
});

test("capability constructor rejects non-object payloads", () => {
  assert.throws(() => new AgentCapabilities([]), NativeProtocolError);
});
