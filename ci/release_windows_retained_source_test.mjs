#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import * as fs from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";
import { requiresRetainedSource, selectNativeCase, requirePassedCase, runNativeWindowsQa, selectCoreEditRootCases, runCoreEditRootCases, CORE_EDIT_ROOT_PREFIX, CORE_CLEANUP_PREFIX, FIXTURE_LOCAL_FINISH_PREFIX, CORE_WORKSPACE_LOCK_CASE, CORE_WORKSPACE_LOCK_PREFIX, runCoreWorkspaceLockCases, CORE_WINDOWS_CI_FIXTURE_CASE, runCoreWindowsCiFixtureCase, selectNativePrefixCases, runCoreCloseoutPrerequisites, SERVER_CASES, CORE_CASE } from "./release_windows_retained_source.mjs";
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
const finishNames = ["closes_the_worktree_with_one_apply_receipt", "preserves_nonempty_failed_cleanup"].map(name => FIXTURE_LOCAL_FINISH_PREFIX + name);
const listing = names => names.map(name => `${name}: test`).join("\r\n") + "\r\n";
const passed = names => names.map(name => `test ${name} ... ok`).join("\r\n") +
  `\r\ntest result: ok. ${names.length} passed; 0 failed; 0 ignored; 0 measured\r\n`;
for (const failure of [null, "missing", "duplicate-list", "prefixed-identity", "wrong-identity", "ignored", "zero", "partial", "duplicate-summary", "command-failure"]) {
  const calls = [];
  const execute = () => runCoreWindowsCiFixtureCase((label, manifest, flags) => {
    calls.push({ label, manifest, flags });
    if (label.endsWith("-case-list")) {
      return listing(failure === "missing" ? [] : failure === "duplicate-list" ? [CORE_WINDOWS_CI_FIXTURE_CASE, CORE_WINDOWS_CI_FIXTURE_CASE] :
        [failure === "prefixed-identity" ? "unreviewed::" + CORE_WINDOWS_CI_FIXTURE_CASE : CORE_WINDOWS_CI_FIXTURE_CASE]);
    }
    if (failure === "command-failure") throw new Error("fixture command failed");
    let output = passed([CORE_WINDOWS_CI_FIXTURE_CASE]);
    if (failure === "wrong-identity") output = output.replace(CORE_WINDOWS_CI_FIXTURE_CASE, "another_case");
    if (failure === "ignored") output = output.replace(" ... ok", " ... ignored");
    if (failure === "zero") output = "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
    if (failure === "partial") output = output.replace("1 passed; 0 failed; 0 ignored;", "1 passed; 0 failed; 1 ignored;");
    if (failure === "duplicate-summary") output += "test result: ok. 1 passed; 0 failed; 0 ignored;\n";
    return output;
  }, "/fixed/ait-core/rust/Cargo.toml");
  if (failure === null) execute(); else assert.throws(execute);
  assert.equal(calls.length, ["missing", "duplicate-list", "prefixed-identity"].includes(failure) ? 1 : 2);
  assert.deepEqual(calls[0].flags, ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli", CORE_WINDOWS_CI_FIXTURE_CASE, "--", "--list"]);
  if (calls.length === 2) assert.deepEqual(calls[1].flags, ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli", CORE_WINDOWS_CI_FIXTURE_CASE,
    "--", "--exact", "--test-threads=1", "--color", "never"]);
}
// A failed portability prerequisite must stop before Server/Runner execution.
{
  const source = fs.readFileSync(new URL("./release_windows_retained_source.mjs", import.meta.url), "utf8");
  const reached = [];
  const context = vm.createContext({ path, runCoreWindowsCiFixtureCase,
    runCoreEditRootCases() { reached.push("edit-root"); },
    runCoreCloseoutPrerequisites() { reached.push("closeout"); } });
  vm.runInContext(source.slice(source.indexOf("function executeCases("), source.indexOf("// Explicit non-release QA")) + "\nthis.execute = executeCases;", context);
  assert.throws(() => context.execute({ root: "/fixed", cargo(label) {
    reached.push(label);
    return label.endsWith("-case-list") ? listing([CORE_WINDOWS_CI_FIXTURE_CASE]) : passed(["another_case"]);
  } }), /did not execute and pass/);
  assert.deepEqual(reached, ["edit-root", "closeout", "core-windows-ci-fixture-case-list", "core-windows-ci-fixture"]);
}
const workspaceNames = ["workspace_command_lock_path_matches_hash_contract_shape", "workspace_command_lock_records_metadata_after_blocking_acquire",
  "nested_rust_workspace_command_borrows_outer_lock", "workspace_lock_delegation_preserves_physical_exclusion",
  "workspace_lock_conflicts_with_existing_whole_file_locks", "workspace_lock_metadata_cannot_overlap_the_lock_byte",
  "workspace_lock_token_is_restored_after_owned_lock_drop", "legacy_direct_child_lock_borrow_requires_actual_held_lock",
  "legacy_same_process_lock_borrow_requires_actual_held_lock"].map(name => CORE_WORKSPACE_LOCK_PREFIX + name);
for (const failure of [null, "missing", "duplicate-list", "wrong-list", "wrong-identity", "ignored", "zero", "partial", "duplicate-summary"]) {
  const calls = [];
  const execute = () => runCoreWorkspaceLockCases((label, manifest, flags) => {
    calls.push({ label, flags });
    if (label.endsWith("-case-list")) return failure === "missing" ? "unrelated: test\n" :
      listing(failure === "duplicate-list" ? [...workspaceNames.slice(1), workspaceNames[1]] :
        failure === "wrong-list" ? [...workspaceNames.slice(1), CORE_WORKSPACE_LOCK_PREFIX + "unreviewed"] : workspaceNames);
    let output = passed(workspaceNames);
    if (failure === "wrong-identity") output = output.replace(CORE_WORKSPACE_LOCK_CASE, "another_case");
    if (failure === "ignored") output = output.replace(" ... ok", " ... ignored");
    if (failure === "zero") output = "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
    if (failure === "partial") output = output.replace("9 passed; 0 failed; 0 ignored;", "8 passed; 0 failed; 1 ignored;");
    if (failure === "duplicate-summary") output += "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
    return output;
  }, "/fixed/ait-core/rust/Cargo.toml");
  if (failure === null) execute(); else assert.throws(execute);
  assert.equal(calls.length, ["missing", "duplicate-list", "wrong-list"].includes(failure) ? 1 : 2);
  if (calls.length === 2) assert.deepEqual(calls[1].flags,
    ["-p", "ait-cli", "--lib", CORE_WORKSPACE_LOCK_PREFIX, "--", "--test-threads=1", "--color", "never"]);
}
assert.deepEqual(selectNativePrefixCases(listing(cleanupNames), CORE_CLEANUP_PREFIX, 3), cleanupNames);
for (const names of [[], cleanupNames.slice(1), [...cleanupNames, cleanupNames[0]], [...cleanupNames, CORE_CLEANUP_PREFIX + "extra"]]) {
  assert.throws(() => selectNativePrefixCases(listing(names), CORE_CLEANUP_PREFIX, 3));
}
for (const failure of [null, "missing-list", "ignored", "zero", "wrong-identity", "partial-summary", "duplicate-summary",
  "fixture-failure", "fixture-ignored", "fixture-partial", "fixture-duplicate", "wrong-fixture-list", "second-fixture-failure"]) {
  const calls = [];
  const execute = () => runCoreCloseoutPrerequisites((label, manifest, flags) => {
    calls.push({ label, manifest, flags });
    if (label === "core-workspace-lock-case-list") return listing(workspaceNames);
    if (label === "core-workspace-lock-reentry") return passed(workspaceNames);
    const names = label.startsWith("core-directory") ? cleanupNames : finishNames;
    if (label.endsWith("-case-list")) {
      if (failure === "missing-list") return "unrelated: test\n";
      if (failure === "wrong-fixture-list" && label.startsWith("core-fixture")) return listing([finishNames[0], FIXTURE_LOCAL_FINISH_PREFIX + "unexpected"]);
      return listing(names);
    }
    const fixtureIndex = /^core-fixture-local-finish-case-(\d)$/.exec(label);
    if (fixtureIndex && failure === "fixture-failure") throw new Error("fixture failed");
    if (fixtureIndex?.[1] === "2" && failure === "second-fixture-failure") throw new Error("second fixture failed");
    const selected = fixtureIndex ? [finishNames[Number(fixtureIndex[1]) - 1]] : names;
    let output = passed(selected);
    if (label === "core-directory-link-cleanup") {
      if (failure === "ignored") output = output.replace(`test ${names[0]} ... ok`, `test ${names[0]} ... ignored`);
      if (failure === "zero") output = "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
      if (failure === "wrong-identity") output = output.replace(names[0], "another_case");
      if (failure === "partial-summary") output = output.replace("3 passed; 0 failed; 0 ignored;", "2 passed; 0 failed; 1 ignored;");
      if (failure === "duplicate-summary") output += "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
    }
    if (fixtureIndex && failure === "fixture-ignored") output = output.replace(`test ${selected[0]} ... ok`, `test ${selected[0]} ... ignored`);
    if (fixtureIndex && failure === "fixture-partial") output = output.replace("1 passed; 0 failed; 0 ignored;", "0 passed; 0 failed; 1 ignored;");
    if (fixtureIndex && failure === "fixture-duplicate") output += "test result: ok. 0 passed; 0 failed; 0 ignored;\n";
    return output;
  }, "/fixed/ait-core/rust/Cargo.toml");
  if (failure === null) execute(); else assert.throws(execute);
  assert.equal(calls.length, failure === null || failure === "second-fixture-failure" ? 7 : failure === "missing-list" ? 1 :
    failure === "wrong-fixture-list" ? 5 : failure.startsWith("fixture-") ? 6 : 2);
  for (const call of calls) {
    assert.equal(call.flags.includes("--exact"), /^core-fixture-local-finish-case-\d$/.test(call.label));
    assert.equal(call.flags.includes("--nocapture"), false, "keep the captured libtest stream contract");
  }
  assert.deepEqual(calls[0].flags, ["-p", "ait-cli", "--lib", CORE_CLEANUP_PREFIX, "--", "--list"]);
  if (calls.length >= 6) {
    assert.deepEqual(calls[4].flags, ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli", FIXTURE_LOCAL_FINISH_PREFIX, "--", "--list"]);
    for (const [index, call] of calls.slice(5).entries()) assert.deepEqual(call.flags,
      ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli", finishNames[index],
        "--", "--exact", "--test-threads=1", "--color", "never"]);
  }
}
// This tests the harness's real child I/O, not native Windows qualification.
// A child must be able to read its retained output before it exits.
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "ait-native-command-capture."));
try {
  const source = fs.readFileSync(new URL("./release_windows_retained_source.mjs", import.meta.url), "utf8");
  const contextSource = source.slice(source.indexOf("function nativeCommandEvent("), source.indexOf("function executeCases("));
  let now = 0;
  const calls = [];
  const context = vm.createContext({ ...fs, assert, path, Buffer, performance, spawn,
    Date: class extends Date { static now() { return now; } },
    process: { platform: "win32", arch: "x64", env: process.env, stdout: process.stdout, pid: process.pid },
    spawnSync(command, args, options) {
      calls.push({ command, timeout: options.timeout });
      return command === "rustc" ? spawnSync(process.execPath, ["-e", "process.stdout.write('host: x86_64-pc-windows-msvc\\n')"], options)
        : spawnSync(command, args, options);
    },
  });
  vm.runInContext(contextSource + "\nthis.makeContext = nativeContext;", context);
  const logs = path.join(scratch, "logs");
  const native = context.makeContext({ target: "x86_64-pc-windows-msvc", root: scratch, logs });
  const stdout = "retained stdout before exit\n", stderr = "retained stderr before exit\n";
  const script = (label, directory = logs) => `const fs=require('node:fs');fs.writeSync(1,${JSON.stringify(stdout)});fs.writeSync(2,${JSON.stringify(stderr)});` +
    `if(fs.readFileSync(${JSON.stringify(path.join(directory, label + ".stdout"))},'utf8')!==${JSON.stringify(stdout)})throw Error('stdout was not retained while running');` +
    `if(!fs.readFileSync(${JSON.stringify(path.join(directory, label + ".stderr"))},'utf8').includes(${JSON.stringify(stderr)}))throw Error('stderr was not retained while running');`;
  assert.equal(native.run("live-output", process.execPath, ["-e", script("live-output")]), stdout);
  assert.throws(() => native.run("failed-child", process.execPath, ["-e", script("failed-child") + "process.exit(17)"]), /failed-child failed/);
  assert.equal(fs.readFileSync(path.join(logs, "failed-child.stdout"), "utf8"), stdout);
  assert(fs.readFileSync(path.join(logs, "failed-child.stderr"), "utf8").includes(stderr));
  assert.throws(() => native.run("unavailable-command", path.join(scratch, "absent-command"), []), /unavailable-command failed/);
  assert.match(fs.readFileSync(path.join(logs, "unavailable-command.stderr"), "utf8"), /ENOENT/);
  const budget = calls[0].timeout;
  assert(budget > 0 && budget <= 105 * 60 * 1000, "reserve time before the hosted job timeout to retain diagnostics");
  now = budget - 200;
  assert.throws(() => native.run("timed-out-child", process.execPath,
    ["-e", script("timed-out-child") + "setInterval(()=>{},1000)"]), /timed-out-child failed/);
  assert.equal(fs.readFileSync(path.join(logs, "timed-out-child.stdout"), "utf8"), stdout);
  assert.match(fs.readFileSync(path.join(logs, "timed-out-child.stderr"), "utf8"), /retained stderr before exit[\s\S]*ETIMEDOUT/);
  now = budget + 1;
  const before = calls.length;
  assert.throws(() => native.run("past-budget", process.execPath, ["-e", "process.exit(0)"]), /budget exhausted/);
  assert.equal(calls.length, before, "an exhausted operation must not launch another command");

  // Simulate only the Windows query process. Real child I/O and the watcher
  // stop/reap handshake still run; actual CIM data remains a native-host gate.
  context.process.env = { ...process.env, SystemRoot: path.join(scratch, "missing-windows"), SYSTEMROOT: path.join(scratch, "missing-windows") };
  const unavailableLogs = path.join(scratch, "unavailable-observation");
  const noObserver = context.makeContext({ target: "x86_64-pc-windows-msvc", root: scratch, logs: unavailableLogs });
  const caseOne = "core-fixture-local-finish-case-1";
  assert.equal(noObserver.run(caseOne, process.execPath, ["-e", script(caseOne, unavailableLogs)]), stdout);
  assert(calls.at(-1).timeout > 0 && calls.at(-1).timeout <= 10 * 60 * 1000);
  const unavailableLog = fs.readFileSync(path.join(unavailableLogs, caseOne + ".stderr"), "utf8");
  assert.match(unavailableLog, /observation-unavailable/);
  assert.match(unavailableLog, /launch-requested[\s\S]*command-returned/);

  const systemRoot = path.join(scratch, "Windows ' fixture");
  const powershell = path.join(systemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe");
  fs.mkdirSync(path.dirname(powershell), { recursive: true }); fs.writeFileSync(powershell, "fixture");
  const observers = [];
  let forcedObserverStops = 0, confirmedStops = 0;
  const observationPowerShell = process.env.NATIVE_OBSERVATION_POWERSHELL ||
    (process.platform === "win32" && path.join(process.env.SystemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe"));
  let stackAttributionChecked = false;
  function checkStackAttribution(program) {
    if (!observationPowerShell || stackAttributionChecked) return;
    assert(path.isAbsolute(observationPowerShell) && fs.statSync(observationPowerShell).isFile());
    const helper = program.slice(program.indexOf("function StackForProcess("), program.indexOf("function CaptureStack("));
    const capture = program.match(/^\s*\$stackCapture = @\{ identity = \$target.identity; pid = \$target.process.Id; status = \(CaptureStack \$target\) \}/m)?.[0];
    const row = program.match(/\$pidStack = StackForProcess[^\n]+\n\s*\$alive \+= @\{[^\n]+/m)?.[0];
    assert(helper && capture && row, "exercise the actual rendered PowerShell attribution and row construction");
    // Run the production PowerShell fragments, not a JavaScript reimplementation.
    // Synthetic entries isolate attribution from Windows CIM/CDB availability.
    const testProgram = `Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
${helper}
function CaptureStack($entry) { return $entry.captureStatus }
function Rows($entries, $stackCapture) {
  $alive = @()
  foreach ($entry in $entries) {
    $key = [string]$entry.process.Id
    $argv = $entry.row.CommandLine
    $wait = @()
    ${row}
  }
  return $alive
}
$entries = @(
  @{ identity = '2916:100'; process = @{ Id = 2916 }; row = @{ ParentProcessId = 100; Name = 'ait_cli-test.exe'; CommandLine = 'test' } },
  @{ identity = '4504:100'; process = @{ Id = 4504 }; row = @{ ParentProcessId = 2916; Name = 'ait-cli.exe'; CommandLine = 'ait-cli task finish' } },
  @{ identity = '1200:100'; process = @{ Id = 1200 }; row = @{ ParentProcessId = 100; Name = 'cargo.exe'; CommandLine = 'cargo test' } }
)
$unavailable = 'unavailable: no stack captured for this tracked process'
$statuses = @('raw best-effort CDB output: C:\\owned\\case.stack.txt',
  'unavailable: owned target exited before capture',
  'unavailable: matching installed CDB not found',
  'unavailable: CDB failed or produced no stack file',
  'unavailable: bounded CDB capture timed out')
foreach ($status in $statuses) {
  $target = $entries[1]
  $target.captureStatus = $status
  ${capture}
  $rows = @(Rows $entries $stackCapture)
  if ($rows.Count -ne 3) { throw 'all three alive processes must retain their own row' }
  foreach ($item in $rows) {
    $expected = if ($item.pid -eq 4504) { $status } else { $unavailable }
    if ($item.stack -ne $expected) { throw ('wrong stack attribution for PID ' + $item.pid) }
  }
  # A completed capture cannot transfer to surviving peers after its owner exits.
  foreach ($item in @(Rows @($entries[0], $entries[2]) $stackCapture)) {
    if ($item.stack -ne $unavailable) { throw 'exited owner stack transferred to another process' }
  }
  # The same numeric PID with another start time is a different tracked entity.
  $reused = @{ identity = '4504:200'; process = @{ Id = 4504 }; row = $target.row }
  if ((Rows @($reused) $stackCapture).stack -ne $unavailable) { throw 'stack transferred to a reused PID' }
}
foreach ($item in @(Rows $entries $null)) {
  if ($item.stack -ne $unavailable) { throw 'uncaptured process must be explicitly unavailable' }
}
[Console]::Out.WriteLine('PowerShell multi-process stack attribution: pass')`;
    const result = spawnSync(observationPowerShell, ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand",
      Buffer.from(testProgram, "utf16le").toString("base64")], { encoding: "utf8", timeout: 15000 });
    assert.equal(result.status, 0, `${result.error ?? ""}\n${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /PowerShell multi-process stack attribution: pass/);
    stackAttributionChecked = true;
    console.log(`PowerShell stack ownership regression: pass; executor-only ${process.platform}/${process.arch}`);
  }
  context.spawn = (command, args, options) => {
    assert.equal(command, powershell);
    const program = Buffer.from(args.at(-1), "base64").toString("utf16le");
    const rootId = Number(/^\$nodeRootId = (\d+)$/m.exec(program)?.[1]);
    const stop = /^\$stopFile = '((?:''|[^'])*)'$/m.exec(program)?.[1].replaceAll("''", "'");
    assert.equal(rootId, process.pid); assert(path.isAbsolute(stop));
    assert.match(program, /Get-CimInstance -ClassName Win32_Process/);
    assert.match(program, /ParentProcessId/);
    assert.match(program, /-pvr -pd -noshell -netsyms:no/);
    assert.doesNotMatch(program, /-pv -p /, "stack capture must not suspend the owned target");
    checkStackAttribution(program);
    const simulation = `const fs=require('node:fs');fs.writeSync(2,'[native-observe] ready root=${rootId} watcher='+process.pid+'\\n');setInterval(()=>{if(fs.existsSync(${JSON.stringify(stop)})){fs.writeSync(2,'[native-observe] stopped root=${rootId} watcher='+process.pid+'\\n');process.exit(0)}},10);`;
    const child = spawn(process.execPath, ["-e", simulation], options);
    const kill = child.kill.bind(child); child.kill = signal => { forcedObserverStops++; return kill(signal); };
    observers.push({ child, closed: new Promise(resolve => child.once("close", resolve)) });
    return child;
  };
  const originalSpawnSync = context.spawnSync;
  context.spawnSync = (command, args, options) => {
    if (command !== powershell) return originalSpawnSync(command, args, options);
    assert(args.includes("-Command")); assert.match(args.at(-1), /WaitForExit\(5000\)/);
    assert.doesNotMatch(args.at(-1), /Stop-Process|taskkill/);
    confirmedStops++; return { status: 0, stdout: "", stderr: "" };
  };
  context.process.env = { ...process.env, SystemRoot: systemRoot };
  const observedLogs = path.join(scratch, "observed ' logs");
  const observationStart = now;
  const observed = context.makeContext({ target: "x86_64-pc-windows-msvc", root: scratch, logs: observedLogs });
  const observedBudget = calls.at(-1).timeout;
  assert.equal(observed.run(caseOne, process.execPath, ["-e", script(caseOne, observedLogs)]), stdout);
  const caseTwo = "core-fixture-local-finish-case-2";
  const lockLabel = "core-workspace-lock-reentry";
  assert.equal(observed.run(lockLabel, process.execPath, ["-e", script(lockLabel, observedLogs)]), stdout);
  assert(calls.at(-1).timeout > 0 && calls.at(-1).timeout <= 120000, "workspace reentry has a short execution cap");
  const ciFixtureLabel = "core-windows-ci-fixture";
  assert.equal(observed.run(ciFixtureLabel, process.execPath, ["-e", script(ciFixtureLabel, observedLogs)]), stdout);
  assert(calls.at(-1).timeout > 0 && calls.at(-1).timeout <= 120000, "Windows CI fixture has a short execution cap");
  now = observationStart + observedBudget - 30000 - 200;
  assert.throws(() => observed.run(caseTwo, process.execPath,
    ["-e", script(caseTwo, observedLogs) + "setInterval(()=>{},1000)"]), /case-2 failed/);
  await Promise.all(observers.map(observer => observer.closed));
  assert.equal(confirmedStops, 3); assert.equal(forcedObserverStops, 0);
  for (const label of [caseOne, caseTwo, lockLabel]) {
    const retained = fs.readFileSync(path.join(observedLogs, label + ".stderr"), "utf8");
    assert.match(retained, /ready root=[\s\S]*command-returned[\s\S]*stopped root=/);
    assert.equal(fs.existsSync(path.join(observedLogs, label + ".observer-stop")), false);
    await new Promise(resolve => setTimeout(resolve, 30));
    assert.equal(fs.readFileSync(path.join(observedLogs, label + ".stderr"), "utf8"), retained, "observer must stop writing before diagnostic closeout");
  }
} finally { fs.rmSync(scratch, { recursive: true, force: true }); }
// Optional owning-Core probe: one real captured Rust libtest case writes via
// the reviewed direct-stderr writer, then waits for our release file. This
// checks the existing C7A file-descriptor boundary, not Windows qualification.
if (process.env.NATIVE_CAPTURE_TEST_BINARY) {
  const binary = process.env.NATIVE_CAPTURE_TEST_BINARY;
  const testName = process.env.NATIVE_CAPTURE_TEST_NAME;
  const marker = process.env.NATIVE_CAPTURE_MARKER;
  const executor = process.env.NATIVE_CAPTURE_EXECUTOR_SOURCE;
  assert(path.isAbsolute(binary) && fs.statSync(binary).isFile());
  assert(path.isAbsolute(executor ?? "") && fs.statSync(executor).isFile());
  assert(testName && marker, "supply the owning Core's exact probe identity and marker");
  const records = fs.mkdtempSync(path.join(os.tmpdir(), "ait-rust-captured-marker."));
  const release = path.join(records, "release");
  const logs = path.join(records, "logs");
  const hostStub = "process.stdout.write(" + JSON.stringify("host: x86_64-pc-windows-msvc\n") + ")";
  const wrapper = `import assert from 'node:assert/strict';import * as fs from 'node:fs';import path from 'node:path';import vm from 'node:vm';import {spawnSync} from 'node:child_process';
const source=fs.readFileSync(${JSON.stringify(executor)},'utf8');
const body=source.slice(source.indexOf('function nativeContext('),source.indexOf('function executeCases('));
const ctx=vm.createContext({...fs,assert,path,Date,process:{platform:'win32',arch:'x64',env:process.env},spawnSync(command,args,options){return command==='rustc'?spawnSync(process.execPath,['-e',${JSON.stringify(hostStub)}],options):spawnSync(command,args,options)}});
vm.runInContext(body+'\\nthis.makeContext=nativeContext;',ctx);
const native=ctx.makeContext({target:'x86_64-pc-windows-msvc',root:${JSON.stringify(records)},logs:${JSON.stringify(logs)}});
process.stdout.write(native.run('rust-live-marker',${JSON.stringify(binary)},${JSON.stringify([testName, "--exact", "--test-threads=1", "--color", "never"])}));`;
  const child = spawn(process.execPath, ["--input-type=module", "-e", wrapper], {
    env: { ...process.env, NATIVE_CAPTURE_RELEASE_FILE: release }, stdio: ["ignore", "pipe", "pipe"],
  });
  let ended = false, consoleOut = "", consoleError = "";
  child.stdout.on("data", data => { consoleOut += data; });
  child.stderr.on("data", data => { consoleError += data; });
  const completion = new Promise(resolve => child.once("close", (code, signal) => { ended = true; resolve({ code, signal }); }));
  let waitedForExit = false;
  async function waitForProbeExit() {
    waitedForExit = true;
    let timer;
    try {
      return await Promise.race([completion, new Promise(resolve => { timer = setTimeout(() => resolve(null), 22000); })]);
    } finally { clearTimeout(timer); }
  }
  const stderrFile = path.join(logs, "rust-live-marker.stderr"), stdoutFile = path.join(logs, "rust-live-marker.stdout");
  try {
    const limit = Date.now() + 8000;
    while (!ended && Date.now() < limit && (!fs.existsSync(stderrFile) || !fs.readFileSync(stderrFile, "utf8").includes(marker))) {
      await new Promise(resolve => setTimeout(resolve, 20));
    }
    assert(!ended, `Core probe returned before live observation: ${consoleOut}\n${consoleError}`);
    assert(!fs.existsSync(release), "the controlled case must remain blocked until observation");
    assert(fs.existsSync(stderrFile) && fs.readFileSync(stderrFile, "utf8").includes(marker), "direct-stderr marker was not retained while the captured Rust case was blocked");
    assert(!/^test result:/m.test(fs.readFileSync(stdoutFile, "utf8")), "a completed result cannot establish live capture");
    fs.writeFileSync(release, "release\n");
    const result = await waitForProbeExit();
    assert(result && result.code === 0, `controlled probe failed after release: ${consoleOut}\n${consoleError}`);
    requirePassedCase(consoleOut, testName);
    assert(/^test result: ok\. 1 passed; 0 failed; 0 ignored;/m.test(consoleOut));
    console.log(`Rust captured direct-stderr live marker: pass; executor-only ${process.platform}/${process.arch}; original logs=${logs}`);
  } finally {
    if (!fs.existsSync(release)) fs.writeFileSync(release, "release\n");
    if (!ended && !waitedForExit) await waitForProbeExit();
    if (!ended) child.kill();
    // The owning probe has its own 20-second bound. Preserve its original logs
    // for both Reviewers; do not replace them with another proof receipt.
  }
}
console.log("Windows native case selection and retained command diagnostics: pass");
