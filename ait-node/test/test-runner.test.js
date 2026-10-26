import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { runRequiredSuites } from "../scripts/test-runner.mjs";

test("required suites fail when empty, entirely skipped, or filtered to zero", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-zero-tests-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const empty = path.join(root, "empty.mjs");
  const skipped = path.join(root, "skipped.mjs");
  const active = path.join(root, "active.mjs");
  await writeFile(empty, "export {};\n");
  await writeFile(skipped, "import test from 'node:test'; test('required', { skip: true }, () => {});\n");
  await writeFile(active, "import test from 'node:test'; test('required', () => {});\n");
  let errors = "";
  const options = { writeOut: () => {}, writeError: (text) => { errors += text; } };
  assert.equal(runRequiredSuites([], options), 1);
  assert.equal(runRequiredSuites([empty], options), 1);
  assert.equal(runRequiredSuites([skipped], options), 1);
  assert.equal(runRequiredSuites([active], {
    ...options, arguments: ["--test-name-pattern=does-not-match"],
  }), 1);
  assert.match(errors, /suite list is empty/);
  assert.match(errors, /executed zero tests/);
  assert.equal(runRequiredSuites([active], options), 0);
});

test("a test failure cannot be masked by a passing required suite", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-test-failure-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const failed = path.join(root, "failed.mjs");
  await writeFile(failed, "import test from 'node:test'; test('required', () => { throw new Error('expected'); });\n");
  assert.equal(runRequiredSuites([failed], { writeOut: () => {}, writeError: () => {} }), 1);
});
