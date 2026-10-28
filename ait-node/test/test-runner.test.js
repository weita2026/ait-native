import assert from "node:assert/strict";
import childProcess from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { syncBuiltinESMExports } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { runRequiredSuites } from "../scripts/test-runner.mjs";

test("required suites fail when empty, entirely skipped, todo, or filtered to zero", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-zero-tests-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const empty = path.join(root, "empty.mjs");
  const skipped = path.join(root, "skipped.mjs");
  const todo = path.join(root, "todo.mjs");
  const active = path.join(root, "active.mjs");
  await writeFile(empty, "export {};\n");
  await writeFile(skipped, "import test from 'node:test'; test('required', { skip: true }, () => {});\n");
  await writeFile(todo, "import test from 'node:test'; test('required', { todo: true }, () => {});\n");
  await writeFile(active, "import test from 'node:test'; test('required', () => {});\n");
  let errors = "";
  const options = { writeOut: () => {}, writeError: (text) => { errors += text; } };
  assert.equal(runRequiredSuites([], options), 1);
  assert.equal(runRequiredSuites([empty], options), 1);
  assert.equal(runRequiredSuites([skipped], options), 1);
  assert.equal(runRequiredSuites([todo], options), 1);
  assert.equal(runRequiredSuites([active], {
    ...options, arguments: ["--test-name-pattern=does-not-match"],
  }), 1);
  assert.match(errors, /suite list is empty/);
  assert.match(errors, /executed zero tests/);
  assert.equal(runRequiredSuites([active], options), 0);
});

test("TAP-escaped file identities cannot qualify an empty module", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-tap#identity-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  // Windows supplies backslashes in the absolute path. A POSIX filename
  // exercises the same escaping on the other supported hosts.
  const empty = path.join(root, process.platform === "win32" ? "empty#file.mjs" : "empty\\#file.mjs");
  const earlyExit = path.join(root, process.platform === "win32" ? "exit#file.mjs" : "exit\\#file.mjs");
  const active = path.join(root, "active#file.mjs");
  await writeFile(empty, "export {};\n");
  await writeFile(earlyExit, "process.exit(0);\n");
  await writeFile(active, "import test from 'node:test'; test('required # case', () => {});\n");
  let errors = "";
  const options = { writeOut: () => {}, writeError: (text) => { errors += text; } };
  assert.equal(runRequiredSuites([empty], options), 1);
  assert.equal(runRequiredSuites([earlyExit], options), 1);
  assert.match(errors, /executed zero tests/);
  assert.equal(runRequiredSuites([active], options), 0);
});

test("Node 22 implicit file results retain escaped Windows and hash identities", (context) => {
  // Node 22 can count an empty file as one passing implicit test. Keep its
  // reporter format covered even on runtimes that report zero tests instead.
  const windows = String.raw`C:\fixtures#root\empty.mjs`;
  let output = String.raw`TAP version 13
# Subtest: C:\\fixtures\#root\\empty.mjs
ok 1 - C:\\fixtures\#root\\empty.mjs
1..1
# tests 1
# suites 0
# pass 1
# fail 0
# cancelled 0
# skipped 0
# todo 0
`;
  context.mock.method(childProcess, "spawnSync", () => ({ stdout: output, stderr: "", status: 0 }));
  syncBuiltinESMExports();
  const options = { writeOut: () => {}, writeError: () => {} };
  try {
    assert.equal(runRequiredSuites([windows], options), 1);
    output = output.replaceAll(String.raw`C:\\fixtures\#root\\empty.mjs`, String.raw`/fixtures\#root/empty.mjs`);
    assert.equal(runRequiredSuites(["/fixtures#root/empty.mjs"], options), 1);
    output = output.replaceAll(String.raw`/fixtures\#root/empty.mjs`, "registered case");
    assert.equal(runRequiredSuites(["/fixtures#root/empty.mjs"], options), 0);
  } finally {
    context.mock.restoreAll();
    syncBuiltinESMExports();
  }
});

test("a test failure cannot be masked by a passing required suite", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-test-failure-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const failed = path.join(root, "failed.mjs");
  const exited = path.join(root, "exited.mjs");
  const active = path.join(root, "active.mjs");
  await writeFile(failed, "import test from 'node:test'; test('required', () => { throw new Error('expected'); });\n");
  await writeFile(exited, "process.exit(23);\n");
  await writeFile(active, "import test from 'node:test'; test('required', () => {});\n");
  const options = { writeOut: () => {}, writeError: () => {} };
  assert.equal(runRequiredSuites([failed, active], options), 1);
  assert.equal(runRequiredSuites([exited, active], options), 1);
});
