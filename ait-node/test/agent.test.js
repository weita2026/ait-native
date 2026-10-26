import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  AgentCapabilities,
  AgentClient,
  NativeProtocolError,
  NativeRuntime,
} from "../src/index.js";

const RETIREMENT = "Chat integrations are retired in AIT 1.2.0";
function retired(error) {
  assert.ok(error instanceof Error);
  assert.ok(error.message.includes(RETIREMENT), error.message);
  return true;
}

test("all retained native chat symbols throw the existing native error", () => {
  const runtime = new NativeRuntime();
  assert.deepEqual(runtime.bindingInfo().supported_surfaces, ["ait-core"]);
  const addon = runtime.loadAddon();
  for (const [name, args] of [
    ["agentWorkerCapabilitiesJson", []],
    ["agentManagementJson", ["not JSON"]],
    ["agentWorkerTransactionJson", ["not JSON"]],
  ]) {
    assert.equal(typeof addon[name], "function");
    assert.throws(() => addon[name](...args), retired);
  }
  assert.throws(() => runtime.agentCapabilities(), retired);
  assert.throws(() => runtime.agentManagement({}), retired);
  assert.throws(() => runtime.agentWorkerTransaction({}), retired);
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

test("every chat facade fails without changing existing worker or user data", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-agent-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, ".ait"));
  const manifestPath = path.join(root, ".ait", "agent-workers.json");
  // An unreadable-as-JSON manifest cannot preempt the retirement error.
  const contents = "user-owned manifest; preserve exactly\n";
  await writeFile(manifestPath, contents);
  const before = await readdir(root, { recursive: true });
  const options = { repoRoot: root, manifestPath, cwd: root };
  const client = new AgentClient();
  const calls = [
    () => client.capabilities(),
    () => client.manage({ operation: "list", repo_root: root, manifest_path: manifestPath }),
    () => client.add({ transport: "telegram", name: "main" }, options),
    () => client.listWorkers("telegram", options),
    () => client.status("telegram", "main", options),
    () => client.start("telegram", "main", { ...options, workerBinary: path.join(root, "no-worker") }),
    () => client.stop("telegram", "main", options),
    () => client.restart("telegram", "main", options),
    () => client.remove("telegram", "main", options),
    () => client.logs("telegram", "main", options),
    () => client.slackCommand({}, options),
    () => client.discordInteraction({}, options),
    () => client.replyProvider({ contract: "unsupported" }, options),
  ];
  for (const call of calls) {
    assert.throws(call, retired);
  }
  assert.equal(await readFile(manifestPath, "utf8"), contents);
  assert.deepEqual(await readdir(root, { recursive: true }), before);
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
