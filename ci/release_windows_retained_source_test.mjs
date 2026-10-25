#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { requiresRetainedSource, selectNativeCase, requirePassedCase, runNativeWindowsQa, selectCoreEditRootCases, runCoreEditRootCases, CORE_EDIT_ROOT_PREFIX, SERVER_CASES, CORE_CASE } from "./release_windows_retained_source.mjs";
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

const roots = ["inspects_complete_windows_drive_roots", "rejects_non_directory_ancestors",
  "rejects_symbolic_link_components"].map(suffix => CORE_EDIT_ROOT_PREFIX + suffix);
const rootListing = roots.map(test => `${test}: test`).join("\r\n") + "\r\nunrelated: test\r\n";
assert.deepEqual(selectCoreEditRootCases(rootListing), roots);
assert.throws(() => selectCoreEditRootCases("unrelated: test\n"), /complete-root regression unavailable/);
assert.throws(() => selectCoreEditRootCases(`${roots[1]}: test\n`), /complete-root regression unavailable/);
assert.throws(() => selectCoreEditRootCases(rootListing + `${roots[0]}: test\n`), /ambiguous/);
const rootPass = roots.map(test => `test ${test} ... ok`).join("\n");
for (const output of [rootPass, rootPass.replace(`test ${roots[2]} ... ok`, `test ${roots[2]} ... ignored`),
  rootPass.replace(`test ${roots[1]} ... ok`, ""), "running 0 tests\ntest result: ok. 0 passed; 0 failed\n"]) {
  const calls = [];
  const execute = () => runCoreEditRootCases((label, manifest, flags) => {
    calls.push({ label, manifest, flags });
    return calls.length === 1 ? rootListing : output;
  }, "/fixed/ait-core/rust/Cargo.toml");
  if (output === rootPass) execute(); else assert.throws(execute, /did not execute and pass/);
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[0].flags, ["-p", "ait-cli", "--lib", CORE_EDIT_ROOT_PREFIX, "--", "--list"]);
  assert.deepEqual(calls[1].flags, ["-p", "ait-cli", "--lib", CORE_EDIT_ROOT_PREFIX,
    "--", "--test-threads=1", "--color", "never"]);
}
const missingCalls = [];
assert.throws(() => runCoreEditRootCases((label) => { missingCalls.push(label); return "unrelated: test\n"; },
  "/fixed/ait-core/rust/Cargo.toml"), /complete-root regression unavailable/);
assert.deepEqual(missingCalls, ["core-explicit-edit-root-case-list"]);
console.log("Windows native case selection rejects missing, ignored and zero-test output");
