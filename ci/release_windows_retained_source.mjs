#!/usr/bin/env node
// Native Windows protocol regressions. These outputs are not A120 approval or
// a substitute for the separately required actual independent-agent scenario.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, closeSync, existsSync, mkdirSync, openSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync, writeSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const SERVER_CASES = [
  "package_restart_reprepares_same_sealed_mode_with_new_ci_and_agent_review",
  "retained_reprepare_rejects_unknown_reset_and_unavailable_foreign_or_ambiguous_source_without_jobs",
  "retained_reprepare_requires_unambiguous_existing_headers_before_mutation",
  "retained_reprepare_does_not_guess_a_same_hash_job_as_the_original_replacement",
  "retained_reprepare_accepts_a_real_terminal_runner_failure_after_restart",
  "package_restart_failed_tests_cannot_be_upgraded_by_an_old_completion_proof",
  "package_restart_revocation_is_not_cleared_by_replaying_the_old_approval",
  "windows_retained_reprepare_holds_hostile_acl_and_junction_before_allocation",
];
export const WINDOWS_OWNER_CASES = [
  "windows_owned_source_pins_paths_and_archive_until_validation_finishes",
  "windows_acl_foreign_owner_and_reparse_paths_are_held",
  "windows_empty_cleanup_is_atomic_and_preserves_inserted_files_and_active_handles",
];
export const CORE_CASE = "reviewed_routed_server_restart_reprepares_the_original_sealed_candidate";
export const CORE_EDIT_ROOT_PREFIX = "task_worktree_layout::tests::explicit_task_edit_root_";
export const CORE_CLEANUP_PREFIX = "primitives::worktree::cleanup::tests::directory_link_cleanup_";
export const FIXTURE_LOCAL_FINISH_PREFIX = "fixture_local_finish_";
const FIXTURE_LOCAL_FINISH_CASES = [
  "fixture_local_finish_closes_the_worktree_with_one_apply_receipt",
  "fixture_local_finish_preserves_nonempty_failed_cleanup",
];
const WINDOWS_EDIT_ROOT_CASE = CORE_EDIT_ROOT_PREFIX + "inspects_complete_windows_drive_roots";

export function requiresRetainedSource(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-rc\.[1-9]\d*)?$/.exec(version ?? "");
  assert(match, "an exact selected family version is required");
  return Number(match[1]) > 1 || Number(match[1]) === 1 && Number(match[2]) >= 2;
}

export function selectNativeCase(listing, suffix) {
  const matches = listing.split(/\r?\n/).filter(line => line.endsWith(": test"))
    .map(line => line.slice(0, -6)).filter(name => name === suffix || name.endsWith("::" + suffix));
  assert.equal(matches.length, 1, `native retained-source case unavailable or ambiguous: ${suffix}; qualification remains HOLD`);
  return matches[0];
}

export function requirePassedCase(stdout, name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  assert(new RegExp(`^test ${escaped} \\.\\.\\. ok\\r?$`, "m").test(stdout),
    `native case did not execute and pass: ${name}; ignored/zero-test output is not a pass`);
}

export function runSingleCargoCase(flags, name) {
  const result = spawnSync("cargo", ["test", "--locked", ...flags, name,
    "--", "--exact", "--test-threads=1", "--color", "never"],
    { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  process.stdout.write(result.stdout ?? "");
  process.stderr.write(result.stderr ?? "");
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `native single case failed: ${name}`);
  requirePassedCase(result.stdout, name);
  assert.equal((result.stdout.match(/^test result:/gm) ?? []).length, 1,
    `native single case requires one complete summary: ${name}`);
  assert(/^test result: ok\. 1 passed; 0 failed; 0 ignored;/m.test(result.stdout),
    `native single case requires exactly one pass and zero ignored: ${name}`);
}

export function selectCoreEditRootCases(listing) {
  const names = listing.split(/\r?\n/).filter(line => line.endsWith(": test"))
    .map(line => line.slice(0, -6)).filter(name => name.startsWith(CORE_EDIT_ROOT_PREFIX));
  assert(names.includes(WINDOWS_EDIT_ROOT_CASE),
    "native Windows complete-root regression unavailable; qualification remains HOLD");
  assert.equal(new Set(names).size, names.length, "ambiguous Core edit-root case listing");
  return names;
}

export function runCoreEditRootCases(cargo, manifest) {
  const flags = ["-p", "ait-cli", "--lib", CORE_EDIT_ROOT_PREFIX];
  const names = selectCoreEditRootCases(cargo("core-explicit-edit-root-case-list", manifest,
    [...flags, "--", "--list"]));
  const stdout = cargo("core-explicit-edit-root", manifest,
    [...flags, "--", "--test-threads=1", "--color", "never"]);
  for (const name of names) requirePassedCase(stdout, name);
}

export function selectNativePrefixCases(listing, prefix, expectedCount) {
  const names = listing.split(/\r?\n/).filter(line => line.endsWith(": test"))
    .map(line => line.slice(0, -6)).filter(name => name.startsWith(prefix));
  assert.equal(names.length, expectedCount, `native prefix requires ${expectedCount} cases: ${prefix}`);
  assert.equal(new Set(names).size, names.length, `ambiguous native prefix listing: ${prefix}`);
  return names;
}

export function runCoreCloseoutPrerequisites(cargo, manifest) {
  for (const [label, flags, prefix, count] of [
    ["core-directory-link-cleanup", ["-p", "ait-cli", "--lib"], CORE_CLEANUP_PREFIX, 3],
    ["core-fixture-local-finish", ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli"], FIXTURE_LOCAL_FINISH_PREFIX, 2],
  ]) {
    const names = selectNativePrefixCases(cargo(label + "-case-list", manifest,
      [...flags, prefix, "--", "--list"]), prefix, count);
    if (prefix === FIXTURE_LOCAL_FINISH_PREFIX) {
      assert.deepEqual(names.slice().sort(), FIXTURE_LOCAL_FINISH_CASES.slice().sort(),
        "the two reviewed fixture-closeout identities are required");
      for (const [index, name] of FIXTURE_LOCAL_FINISH_CASES.entries()) {
        const stdout = cargo(label + "-case-" + (index + 1), manifest,
          [...flags, name, "--", "--exact", "--test-threads=1", "--color", "never"]);
        requirePassedCase(stdout, name);
        assert.equal((stdout.match(/^test result:/gm) ?? []).length, 1, `incomplete fixture summary: ${name}`);
        assert(/^test result: ok\. 1 passed; 0 failed; 0 ignored;/m.test(stdout),
          `fixture case must execute once without ignored output: ${name}`);
      }
      continue;
    }
    const stdout = cargo(label, manifest, [...flags, prefix, "--", "--test-threads=1", "--color", "never"]);
    for (const name of names) requirePassedCase(stdout, name);
    assert.equal((stdout.match(/^test result:/gm) ?? []).length, 1, `incomplete native group summary: ${prefix}`);
    assert(new RegExp(`^test result: ok\\. ${count} passed; 0 failed; 0 ignored;`, "m").test(stdout),
      `native group must execute all ${count} cases without ignored output: ${prefix}`);
  }
}

function nativeCommandEvent(descriptor, event, detail) {
  writeSync(descriptor, "[native-command] " + JSON.stringify({ time: new Date(Date.now()).toISOString(), event, ...detail }) + "\n");
}

function windowsObservationScript(rootProcessId, stopFile) {
  const literal = value => "'" + value.replaceAll("'", "''") + "'";
  return String.raw`$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$nodeRootId = ${rootProcessId}
$stopFile = ${literal(stopFile)}
$watcherId = $PID
$tracked = @{}
function Emit($value) {
  $value.time = [DateTime]::UtcNow.ToString('o')
  [Console]::Error.WriteLine('[native-observe] ' + ($value | ConvertTo-Json -Compress -Depth 8))
  [Console]::Error.Flush()
}
try {
  $rootProcess = [Diagnostics.Process]::GetProcessById($nodeRootId)
  $null = $rootProcess.Handle
  [Console]::Error.WriteLine("[native-observe] ready root=$nodeRootId watcher=$watcherId")
  [Console]::Error.Flush()
  do {
    $stopping = [IO.File]::Exists($stopFile) -or $rootProcess.HasExited
    try {
      $rows = @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 2 -ErrorAction Stop)
      $owned = @{}
      $owned[[string]$nodeRootId] = $true
      foreach ($key in @($tracked.Keys)) {
        if (-not $tracked[$key].process.HasExited) { $owned[$key] = $true }
      }
      do {
        $expanded = $false
        foreach ($row in $rows) {
          $key = [string]$row.ProcessId
          if ($row.ProcessId -ne $watcherId -and $owned.ContainsKey([string]$row.ParentProcessId) -and -not $owned.ContainsKey($key)) {
            $owned[$key] = $true
            $expanded = $true
          }
        }
      } while ($expanded)
      foreach ($row in $rows) {
        $key = [string]$row.ProcessId
        if ($row.ProcessId -eq $nodeRootId -or $row.ProcessId -eq $watcherId -or -not $owned.ContainsKey($key)) { continue }
        if (-not $tracked.ContainsKey($key)) {
          try {
            $observed = [Diagnostics.Process]::GetProcessById([int]$row.ProcessId)
            $null = $observed.Handle
            $tracked[$key] = @{ process = $observed; row = $row }
            $created = if ($row.CreationDate) { $row.CreationDate.ToUniversalTime().ToString('o') } else { 'unavailable' }
            $argv = if ($row.CommandLine) { $row.CommandLine } else { 'unavailable: CIM command line not returned' }
            Emit @{ event = 'started'; pid = $row.ProcessId; parent_pid = $row.ParentProcessId; name = $row.Name; argv = $argv; created = $created; executable = $row.ExecutablePath }
          } catch {
            Emit @{ event = 'unavailable'; pid = $row.ProcessId; detail = ('process exited or handle unavailable before sampling: ' + $_.Exception.Message) }
          }
        }
      }
      $alive = @()
      foreach ($key in @($tracked.Keys)) {
        $entry = $tracked[$key]
        try {
          $entry.process.Refresh()
          if ($entry.process.HasExited) {
            Emit @{ event = 'exited'; pid = [int]$key; exit_code = $entry.process.ExitCode; ended = $entry.process.ExitTime.ToUniversalTime().ToString('o') }
            $entry.process.Dispose()
            $tracked.Remove($key)
          } else {
            $wait = @()
            try {
              foreach ($thread in $entry.process.Threads) {
                $reason = $null
                if ([string]$thread.ThreadState -eq 'Wait') { $reason = [string]$thread.WaitReason }
                $wait += @{ thread_id = $thread.Id; state = [string]$thread.ThreadState; wait_reason = $reason }
              }
            } catch { $wait = @(@{ unavailable = $_.Exception.Message }) }
            $argv = if ($entry.row.CommandLine) { $entry.row.CommandLine } else { 'unavailable: CIM command line not returned' }
            $alive += @{ pid = [int]$key; parent_pid = $entry.row.ParentProcessId; name = $entry.row.Name; argv = $argv; wait = $wait; stack = 'unavailable: not collected by the read-only observer' }
          }
        } catch { Emit @{ event = 'unavailable'; pid = [int]$key; detail = $_.Exception.Message } }
      }
      Emit @{ event = 'snapshot'; stopping = $stopping; alive = $alive; coverage = 'sampled owned descendants; processes entirely between samples are unavailable' }
    } catch { Emit @{ event = 'unavailable'; detail = ('local process observation failed: ' + $_.Exception.Message) } }
    if (-not $stopping) { Start-Sleep -Milliseconds 500 }
  } while (-not $stopping)
} catch { Emit @{ event = 'unavailable'; detail = $_.Exception.Message } }
finally {
  foreach ($entry in $tracked.Values) { $entry.process.Dispose() }
  if ($rootProcess) { $rootProcess.Dispose() }
  [Console]::Error.WriteLine("[native-observe] stopped root=$nodeRootId watcher=$watcherId")
  [Console]::Error.Flush()
}
`;
}

function startWindowsObservation({ label, logs, descriptor, env }) {
  const systemRoot = env.SystemRoot ?? env.SYSTEMROOT;
  const command = systemRoot && path.isAbsolute(systemRoot) && path.join(systemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe");
  const unavailable = detail => nativeCommandEvent(descriptor, "observation-unavailable", { label, detail });
  if (!command || !existsSync(command) || !Number.isInteger(process.pid)) {
    unavailable("local Windows PowerShell or owning Node PID is unavailable");
    return () => {};
  }
  const stopFile = path.join(logs, label + ".observer-stop");
  const script = windowsObservationScript(process.pid, stopFile);
  const observer = spawn(command, ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")],
    { env, stdio: ["ignore", "ignore", descriptor], windowsHide: true });
  let launchError;
  observer.on("error", error => { launchError = String(error); });
  if (!observer.pid) { unavailable(launchError ?? "observer could not be launched"); return () => {}; }
  const stderrFile = path.join(logs, label + ".stderr");
  const contains = marker => readFileSync(stderrFile, "utf8").includes(marker);
  const pause = () => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
  const ready = `[native-observe] ready root=${process.pid} watcher=${observer.pid}`;
  const stopped = `[native-observe] stopped root=${process.pid} watcher=${observer.pid}`;
  let stoppedOnce = false;
  const stop = () => {
    if (stoppedOnce) return;
    stoppedOnce = true;
    writeFileSync(stopFile, "stop\n", { flag: "wx" });
    const limit = performance.now() + 4000;
    while (!contains(stopped) && performance.now() < limit) pause();
    if (!contains(stopped)) { unavailable("observer did not acknowledge stop; terminating only the owned observer"); observer.kill(); }
    // Keep the ChildProcess handle while the OS confirms this exact owned
    // watcher has exited. No Cargo/test/CLI process is killed by observation.
    const wait = `try { $watched = [Diagnostics.Process]::GetProcessById(${observer.pid}); if (-not $watched.WaitForExit(5000)) { exit 2 }; $watched.Dispose() } catch [ArgumentException] { exit 0 }`;
    const confirmation = spawnSync(command, ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", wait],
      { env, encoding: "utf8", timeout: 6500, windowsHide: true });
    rmSync(stopFile, { force: true });
    assert.equal(confirmation.status, 0, "owned process observer did not stop before diagnostic closeout");
  };
  const limit = performance.now() + 5000;
  while (!contains(ready) && !contains(stopped) && performance.now() < limit) pause();
  if (!contains(ready)) { unavailable(launchError ?? "observer unavailable before command launch; see its original stderr"); stop(); return () => {}; }
  return stop;
}

function nativeContext({ target, root, logs }) {
  const arches = { "x86_64-pc-windows-msvc": "x64", "aarch64-pc-windows-msvc": "arm64" };
  assert.equal(process.platform, "win32", "actual native Windows execution is required");
  assert.equal(process.arch, arches[target], "Node host architecture must match the selected Windows lane");
  assert(arches[target] && path.isAbsolute(root) && path.isAbsolute(logs));
  assert(!existsSync(logs), "preserve the original attempt's logs"); mkdirSync(logs, { recursive: true });
  const env = { ...process.env };
  // Leave time for the existing always-upload step before the 120-minute job
  // limit. Every command shares this budget; a new command never resets it.
  const deadline = Date.now() + 90 * 60 * 1000;
  function run(label, command, argv) {
    writeFileSync(path.join(logs, label + ".argv"), [command, ...argv].join("\n") + "\n");
    const stdoutFile = path.join(logs, label + ".stdout"), stderrFile = path.join(logs, label + ".stderr");
    const observe = /^core-fixture-local-finish-case-[12]$/.test(label);
    const cleanupReserve = observe ? 15000 : 0;
    const timeout = Math.min(deadline - Date.now() - cleanupReserve, observe ? 10 * 60 * 1000 : Infinity);
    if (timeout <= 0) {
      writeFileSync(stdoutFile, "");
      writeFileSync(stderrFile, "Native retained-source execution budget exhausted before command launch\n");
      throw new Error(`${label}: native retained-source execution budget exhausted`);
    }
    const descriptors = [];
    let result;
    let stopObservation = () => {};
    try {
      descriptors.push(openSync(stdoutFile, "wx"));
      descriptors.push(openSync(stderrFile, "wx"));
      nativeCommandEvent(descriptors[1], "launch-requested", { label, parent_pid: process.pid, command, argv, timeout_ms: timeout });
      if (observe) stopObservation = startWindowsObservation({ label, logs, descriptor: descriptors[1], env });
      const remaining = Math.min(deadline - Date.now() - cleanupReserve, timeout);
      assert(remaining > 0, `${label}: native retained-source execution budget exhausted before launch`);
      const started = Date.now();
      result = spawnSync(command, argv, { cwd: root, env, encoding: "utf8", timeout: remaining,
        stdio: ["ignore", ...descriptors] });
      nativeCommandEvent(descriptors[1], "command-returned", { label, pid: result.pid, status: result.status,
        signal: result.signal, error: result.error ? String(result.error) : null, elapsed_ms: Date.now() - started });
    } finally {
      try { stopObservation(); } finally { for (const descriptor of descriptors) closeSync(descriptor); }
    }
    if (result.error) appendFileSync(stderrFile, "\n" + String(result.error) + "\n");
    assert.equal(result.status, 0, `${label} failed; original output: ${logs}`);
    return readFileSync(stdoutFile, "utf8");
  }
  const rust = run("native-rust-host", "rustc", ["-vV"]);
  assert.equal(/^host: (.+)$/m.exec(rust)?.[1].trim(), target, "cross-compilation is not native evidence");
  const targetDir = logs + "-cargo-target";
  env.CARGO_BUILD_BUILD_DIR = logs + "-cargo-build";
  const cargo = (label, manifest, flags) => run(label, "cargo", ["test", "--locked", "--manifest-path", manifest,
    "--target", target, "--target-dir", targetDir, ...flags]);
  return { target, root, env, run, targetDir, cargo };
}

function executeCases({ target, root, env, run, targetDir, cargo }) {
  const serverManifest = path.join(root, "ait-server/rust/Cargo.toml");
  const coreManifest = path.join(root, "ait-core/rust/Cargo.toml");
  runCoreEditRootCases(cargo, coreManifest);
  runCoreCloseoutPrerequisites(cargo, coreManifest);
  const serverFlags = ["-p", "ait-server", "--lib"];
  const serverList = cargo("server-native-case-list", serverManifest, [...serverFlags, "--", "--list"]);
  const serverNames = SERVER_CASES.map(name => selectNativeCase(serverList, name));
  const ownerFlags = ["-p", "ait-server-core", "--lib"];
  const ownerList = cargo("server-windows-owner-case-list", serverManifest, [...ownerFlags, "--", "--list"]);
  const ownerNames = WINDOWS_OWNER_CASES.map(name => selectNativeCase(ownerList, name));
  for (const [names, suffixes, flags] of [[ownerNames, WINDOWS_OWNER_CASES, ownerFlags], [serverNames, SERVER_CASES, serverFlags]]) {
    for (let index = 0; index < names.length; index++) {
      requirePassedCase(cargo("server-" + suffixes[index], serverManifest,
        [...flags, names[index], "--", "--exact", "--test-threads=1", "--color", "never"]), names[index]);
    }
  }
  const coreFlags = ["-p", "ait-cli", "--test", "workflow_package_accepted_retry_cli"];
  const coreList = cargo("core-native-case-list", coreManifest, [...coreFlags, "--", "--list"]);
  const coreName = selectNativeCase(coreList, CORE_CASE);
  const ciRoot = env.TEST_SERVER_CI_RAM_ROOT;
  assert(ciRoot && path.isAbsolute(ciRoot) && statSync(ciRoot).isDirectory(),
    "native Windows CI runtime root TEST_SERVER_CI_RAM_ROOT is unavailable; qualification remains HOLD");
  for (const [label, manifest, pkg, bin] of [
    ["core-native-build", coreManifest, "ait-cli", "ait-cli"],
    ["server-native-build", serverManifest, "ait-server", "ait-server"],
    ["runner-native-build", path.join(root, "ait-runner/Cargo.toml"), "ait-runner", "ait-runner"],
  ]) run(label, "cargo", ["build", "--locked", "--manifest-path", manifest, "-p", pkg, "--bin", bin,
    "--target", target, "--target-dir", targetDir]);
  env.TEST_REVIEWED_SERVER_BIN = path.join(targetDir, target, "debug/ait-server.exe");
  env.TEST_NATIVE_RUNNER_BIN = path.join(targetDir, target, "debug/ait-runner.exe");
  requirePassedCase(cargo("core-native-runner-restart", coreManifest,
    [...coreFlags, coreName, "--", "--exact", "--ignored", "--test-threads=1", "--color", "never"]), coreName);
  process.stdout.write("Native retained-source protocol regressions completed; actual agent and platform qualification gates remain separate.\n");
}

// Explicit non-release QA runs the same guarded native cases without changing
// family versions or producing the release qualification carrier.
export function runNativeWindowsQa(input) {
  executeCases(nativeContext(input));
}

function main() {
  const args = process.argv.slice(2); assert.equal(args.length, 6);
  const input = Object.fromEntries([args.slice(0, 2), args.slice(2, 4), args.slice(4, 6)]);
  const root = input["--source-root"];
  const context = nativeContext({ target: input["--target"], root, logs: input["--output"] });
  const family = JSON.parse(readFileSync(path.join(root, "ait-release-family.json")));
  if (!requiresRetainedSource(family.family?.version)) {
    process.stdout.write("Selected earlier family retains its existing Windows init/run scope.\n"); return;
  }
  executeCases(context);
}

if (process.argv[1] && existsSync(process.argv[1]) && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { process.stderr.write(error.message + "\n"); process.exitCode = 75; }
}
