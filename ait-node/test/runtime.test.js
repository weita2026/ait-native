import assert from "node:assert/strict";
import test from "node:test";

import {
  NativeProtocolError,
  NativeResolutionError,
  NativeRuntime,
  requiredAddonExports,
} from "../src/index.js";

test("binding info and version come from the real in-process addon", () => {
  const runtime = new NativeRuntime();

  const payload = runtime.bindingInfo();

  assert.equal(payload.contract, "ait.language.binding.v1");
  assert.equal(payload.runtime_authority, "rust");
  assert.equal(payload.node_binding, "napi");
  assert.equal(payload.process_transport_allowed, false);
  assert.equal(payload.version, "1.0.0-rc.3");
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

test("removed task publish operation is not exported", () => {
  const runtime = new NativeRuntime();
  const addon = runtime.loadAddon();

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
