import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  NativeProtocolError,
  NativeResolutionError,
  NativeRuntime,
  requiredAddonExports,
} from "../src/index.js";
import {
  detectRuntimeLibc,
  selectPlatformPayload,
} from "../src/runtime.js";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("binding info and version come from the real in-process addon", () => {
  const runtime = new NativeRuntime();

  const payload = runtime.bindingInfo();

  assert.equal(payload.contract, "ait.language.binding.v1");
  assert.equal(payload.runtime_authority, "rust");
  assert.equal(payload.node_binding, "napi");
  assert.equal(payload.process_transport_allowed, false);
  assert.equal(payload.version, "1.1.0");
  assert.deepEqual(payload.supported_surfaces, [
    "ait-core",
    "ait-agent",
    "ait-agent-worker",
  ]);
  assert.equal(runtime.version(), payload.version);
});

test("generic call resolves the installed N-API exports directly", () => {
  const runtime = new NativeRuntime();

  for (const name of requiredAddonExports()) {
    assert.equal(typeof runtime.resolveCallable(name), "function");
  }
  assert.deepEqual(
    JSON.parse(runtime.call("bindingInfoJson")),
    runtime.bindingInfo(),
  );
  assert.equal(requiredAddonExports().includes("runCli"), true);
  assert.throws(() => runtime.runCli(["ok", 1]), /array of NUL-free strings/);
});

test("management is exported while retired task publish operations remain absent", () => {
  const runtime = new NativeRuntime();
  const addon = runtime.loadAddon();

  assert.equal(typeof runtime.agentManagement, "function");
  assert.equal(typeof addon.agentManagementJson, "function");
  for (const name of [
    "taskWorkflowTaskPublish",
    "task_workflow_task_publish",
    "taskPublish",
    "task_publish",
  ]) {
    assert.equal(name in addon, false);
    assert.throws(() => runtime.resolveCallable(name), NativeResolutionError);
  }
});

test("missing addon and missing export fail closed", () => {
  const runtime = new NativeRuntime({
    addonPath: new URL("../native/does-not-exist.node", import.meta.url),
  });

  assert.throws(() => runtime.loadAddon(), NativeResolutionError);
  assert.throws(
    () => new NativeRuntime().resolveCallable("notAnAitNapiExport"),
    NativeResolutionError,
  );
});

test("binding payload validation is explicit", () => {
  const runtime = new NativeRuntime();
  runtime.call = () => "{}";

  assert.throws(() => runtime.bindingInfo(), NativeProtocolError);
});

test("Linux addon selection distinguishes glibc from musl before loading", async () => {
  assert.equal(
    detectRuntimeLibc("linux", {
      header: { glibcVersionRuntime: "2.39" },
      sharedObjects: [],
    }),
    "glibc",
  );
  assert.equal(
    detectRuntimeLibc("linux", {
      header: {},
      sharedObjects: ["/lib/ld-musl-x86_64.so.1"],
    }),
    "musl",
  );
  assert.equal(
    detectRuntimeLibc("linux", { header: {}, sharedObjects: [] }),
    "unknown",
  );
  assert.equal(detectRuntimeLibc("darwin", null), null);

  const contract = JSON.parse(
    await readFile(path.join(ROOT, "lib", "npm-payload-contract.json"), "utf8"),
  );
  const glibc = selectPlatformPayload(contract, {
    os: "linux",
    cpu: "x64",
    libc: "glibc",
  });
  assert.equal(glibc.target, "x86_64-unknown-linux-gnu");
  assert.throws(
    () =>
      selectPlatformPayload(contract, {
        os: "linux",
        cpu: "x64",
        libc: "musl",
      }),
    /does not support Node-API on linux\/x64\/musl/,
  );
});
