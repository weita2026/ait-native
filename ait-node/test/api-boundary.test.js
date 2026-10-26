import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { NativeResolutionError, NativeRuntime, requiredAddonExports } from "../src/index.js";
import { requiredAddonConstants, requiredAddonStringConstants } from "../src/runtime.js";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("published functions have no CLI parser or process transport backdoor", async () => {
  const files = await readdir(path.join(ROOT, "src"), { recursive: true });
  for (const name of files.filter((name) => name.endsWith(".js"))) {
    const source = await readFile(path.join(ROOT, "src", name), "utf8");
    assert.doesNotMatch(source, /node:child_process|entry_with_args|entryWithArgs/);
    assert.doesNotMatch(source, /\b(?:spawn|spawnSync|execFile|execSync)\s*\(/);
    if (name !== "terminal.js") {
      assert.doesNotMatch(source, /process\.argv|parseArgs|\.runCli\s*\(/);
    }
  }
  const types = await readFile(path.join(ROOT, "src", "index.d.ts"), "utf8");
  assert.doesNotMatch(types, /\brunCli\s*\(/);
  assert.doesNotMatch(types, /\btaskWorkflowRepoCommand\s*\(/);
  assert.equal("runCli" in NativeRuntime.prototype, false);
  assert.equal(requiredAddonExports().includes("runCli"), false);
  assert.equal(requiredAddonExports().includes("taskWorkflowRepoCommand"), false);
});

test("the reused loader rejects an addon exposing the removed CLI relay", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-api-boundary-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const addonPath = path.join(root, "legacy.cjs");
  const names = requiredAddonExports();
  await writeFile(addonPath, [
    `module.exports = Object.fromEntries(${JSON.stringify(names)}.map(name => [name, () => { throw new Error('export must not be probed'); }]));`,
    "module.exports.runCli = () => { throw new Error('CLI relay must never execute'); };",
  ].join("\n"));
  assert.throws(() => new NativeRuntime({ addonPath }).loadAddon(), (error) => {
    assert.ok(error instanceof NativeResolutionError);
    assert.match(error.message, /removed CLI relay/);
    return true;
  });
});

test("npm entry remains JavaScript and invokes the terminal frontend", async () => {
  const packageJson = JSON.parse(await readFile(path.join(ROOT, "package.json"), "utf8"));
  assert.deepEqual(packageJson.bin, { ait: "bin/ait.mjs" });
  const command = await readFile(path.join(ROOT, packageJson.bin.ait), "utf8");
  assert.match(command, /^#!\/usr\/bin\/env node/);
  assert.match(command, /runTerminal\(process\.argv\.slice\(2\)/);
  assert.doesNotMatch(command, /\.runCli\s*\(|node:child_process|\.exe\b/);
});

test("renaming a relay to an unmapped native export cannot bypass the loader", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-unmapped-api-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const addonPath = path.join(root, "unmapped.cjs");
  await writeFile(addonPath, [
    `module.exports = Object.fromEntries(${JSON.stringify(requiredAddonExports())}.map(name => [name, () => null]));`,
    "module.exports.executeCommand = () => { throw new Error('must never execute'); };",
  ].join("\n"));
  assert.throws(() => new NativeRuntime({ addonPath }).loadAddon(), /unmapped function: executeCommand/);
});

test("generic lookup cannot resolve inherited JavaScript methods as native exports", () => {
  const runtime = new NativeRuntime({ addonPath: "/unavailable/native.node" });
  for (const name of ["constructor", "toString", "hasOwnProperty", "runCli"]) {
    assert.throws(() => runtime.resolveCallable(name), (error) => {
      assert.ok(error instanceof NativeResolutionError);
      assert.match(error.message, /does not declare/);
      return true;
    });
  }
});


test("loader validates native timeout constants without probing retained constructors", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-constant-boundary-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const addonPath = path.join(root, "constants.cjs");
  await writeFile(addonPath, [
    `module.exports = Object.fromEntries(${JSON.stringify(requiredAddonExports())}.map(name => [name, () => { throw new Error('must not probe functions or constructors'); }]));`,
    `Object.assign(module.exports, ${JSON.stringify(requiredAddonConstants())});`,
    "module.exports.DEFAULT_REQUEST_TIMEOUT_SECONDS = 0;",
  ].join("\n"));
  assert.throws(() => new NativeRuntime({ addonPath }).loadAddon(), /incompatible constant DEFAULT_REQUEST_TIMEOUT_SECONDS/);
});


test("loader checks retained native string constants without constructor or policy probes", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-sdk-values-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const addonPath = path.join(root, "values.cjs");
  await writeFile(addonPath, [
    `module.exports = Object.fromEntries(${JSON.stringify(requiredAddonExports())}.map(name => [name, () => { throw new Error('must not probe functions or constructors'); }]));`,
    `Object.assign(module.exports, ${JSON.stringify(requiredAddonConstants())});`,
    `Object.assign(module.exports, Object.fromEntries(${JSON.stringify(requiredAddonStringConstants())}.map(name => [name, "fixture"])));`,
    "module.exports.CONTRACT_VERSION = 1;",
  ].join("\n"));
  assert.throws(() => new NativeRuntime({ addonPath }).loadAddon(), /CONTRACT_VERSION as a string constant/);
});
