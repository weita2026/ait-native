#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { requiresRetainedSource, selectNativeCase, requirePassedCase, runNativeWindowsQa, SERVER_CASES, CORE_CASE } from "./release_windows_retained_source.mjs";
// The workflow imports this module from inline Node with the target as argv[1].
// That argument is not a script path and must not trigger CLI entry detection.
const imported = spawnSync(process.execPath, ["--input-type=module", "-e",
  `import { selectNativeCase } from ${JSON.stringify(new URL("./release_windows_retained_source.mjs", import.meta.url).href)}; process.stdout.write(selectNativeCase("native_case: test\\n", "native_case"));`,
  "x86_64-pc-windows-msvc"], { encoding: "utf8" });
assert.equal(imported.status, 0, imported.stderr);
assert.equal(imported.stdout, "native_case");
assert.equal(requiresRetainedSource("1.1.4"), false);
assert.equal(requiresRetainedSource("1.2.0-rc.1"), true);
assert.equal(requiresRetainedSource("1.2.0"), true);
assert.throws(() => requiresRetainedSource("unknown"));
if (process.platform !== "win32") {
  assert.throws(() => runNativeWindowsQa({ target: "x86_64-pc-windows-msvc", root: "/not-invoked", logs: "/not-created" }), /actual native Windows execution/);
}
const name = "router::http::tests::restart::" + SERVER_CASES[0];
assert.equal(selectNativeCase(`${name}: test\r\n1 test, 0 benchmarks\r\n`, SERVER_CASES[0]), name);
assert.equal(selectNativeCase(`${CORE_CASE}: test\n`, CORE_CASE), CORE_CASE);
// Cargo exits successfully for missing filters and ignored tests. Neither
// output may turn a Windows cfg omission into native functional evidence.
assert.throws(() => selectNativeCase("init: test\n1 test, 0 benchmarks\n", SERVER_CASES[0]));
assert.throws(() => selectNativeCase(`${name}: test\nother::${SERVER_CASES[0]}: test\n`, SERVER_CASES[0]));
requirePassedCase(`running 1 test\ntest ${name} ... ok\r\ntest result: ok. 1 passed; 0 failed\n`, name);
for (const output of ["running 0 tests\ntest result: ok. 0 passed; 0 failed\n", `test ${name} ... ignored\n`,
  `test other::case ... ok\n`, `test ${name} ... FAILED\n`]) assert.throws(() => requirePassedCase(output, name));
console.log("Windows native case selection rejects missing, ignored and zero-test output");
