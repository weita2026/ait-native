#!/usr/bin/env node
// Native Windows protocol regressions. These outputs are not A120 approval or
// a substitute for the separately required actual independent-agent scenario.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
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

function nativeContext({ target, root, logs }) {
  const arches = { "x86_64-pc-windows-msvc": "x64", "aarch64-pc-windows-msvc": "arm64" };
  assert.equal(process.platform, "win32", "actual native Windows execution is required");
  assert.equal(process.arch, arches[target], "Node host architecture must match the selected Windows lane");
  assert(arches[target] && path.isAbsolute(root) && path.isAbsolute(logs));
  assert(!existsSync(logs), "preserve the original attempt's logs"); mkdirSync(logs, { recursive: true });
  const env = { ...process.env };
  function run(label, command, argv) {
    writeFileSync(path.join(logs, label + ".argv"), [command, ...argv].join("\n") + "\n");
    const result = spawnSync(command, argv, { cwd: root, env, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
    writeFileSync(path.join(logs, label + ".stdout"), result.stdout ?? "");
    writeFileSync(path.join(logs, label + ".stderr"), result.stderr ?? String(result.error ?? ""));
    assert.equal(result.status, 0, `${label} failed; original output: ${logs}`);
    return result.stdout;
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
