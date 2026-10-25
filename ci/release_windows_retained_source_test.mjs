#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { requiresRetainedSource, selectNativeCase, requirePassedCase, runNativeWindowsQa, selectCoreEditRootCases, runCoreEditRootCases, CORE_EDIT_ROOT_PREFIX, CORE_CLEANUP_PREFIX, FIXTURE_LOCAL_FINISH_PREFIX, selectNativePrefixCases, runCoreCloseoutPrerequisites, SERVER_CASES, CORE_CASE } from "./release_windows_retained_source.mjs";
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

const cleanupNames = ["live_directory", "dangling_directory", "file_link"].map(name => CORE_CLEANUP_PREFIX + name);
const finishNames = ["resumes_same_receipt", "preserves_nonempty_failure"].map(name => FIXTURE_LOCAL_FINISH_PREFIX + name);
const listing = names => names.map(name => `${name}: test`).join("\r\n") + "\r\n";
const passed = names => names.map(name => `test ${name} ... ok`).join("\r\n") +
  `\r\ntest result: ok. ${names.length} passed; 0 failed; 0 ignored; 0 measured\r\n`;
assert.deepEqual(selectNativePrefixCases(listing(cleanupNames), CORE_CLEANUP_PREFIX, 3), cleanupNames);
for (const names of [[], cleanupNames.slice(1), [...cleanupNames, cleanupNames[0]], [...cleanupNames, CORE_CLEANUP_PREFIX + "extra"]]) {
  assert.throws(() => selectNativePrefixCases(listing(names), CORE_CLEANUP_PREFIX, 3));
}
for (const failure of [null, "missing-list", "ignored", "zero", "wrong-identity", "partial-summary", "duplicate-summary", "fixture-failure"]) {
  const calls = [];
  const execute = () => runCoreCloseoutPrerequisites((label, manifest, flags) => {
    calls.push({ label, manifest, flags });
    const names = label.startsWith("core-directory") ? cleanupNames : finishNames;
    if (label.endsWith("-case-list")) return failure === "missing-list" ? "unrelated: test\n" : listing(names);
    if (label === "core-fixture-local-finish" && failure === "fixture-failure") throw new Error("fixture failed");
    let output = passed(names);
    if (label === "core-directory-link-cleanup") {
      if (failure === "ignored") output = output.replace(`test ${names[0]} ... ok`, `test ${names[0]} ... ignored`);
      if (failure === "zero") output = "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
      if (failure === "wrong-identity") output = output.replace(names[0], "another_case");
      if (failure === "partial-summary") output = output.replace("3 passed; 0 failed; 0 ignored;", "2 passed; 0 failed; 1 ignored;");
      if (failure === "duplicate-summary") output += "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
    }
    return output;
  }, "/fixed/ait-core/rust/Cargo.toml");
  if (failure === null) execute(); else assert.throws(execute);
  assert.equal(calls.length, failure === null || failure === "fixture-failure" ? 4 : failure === "missing-list" ? 1 : 2);
  for (const call of calls) assert.equal(call.flags.includes("--exact"), false, "prefix groups must not use --exact");
  assert.deepEqual(calls[0].flags, ["-p", "ait-cli", "--lib", CORE_CLEANUP_PREFIX, "--", "--list"]);
  if (calls.length === 4) {
    assert.deepEqual(calls[2].flags, ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli", FIXTURE_LOCAL_FINISH_PREFIX, "--", "--list"]);
    assert.deepEqual(calls[3].flags, ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli", FIXTURE_LOCAL_FINISH_PREFIX,
      "--", "--test-threads=1", "--color", "never"]);
  }
}
console.log("Windows native case selection rejects missing, ignored and zero-test output");
