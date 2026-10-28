// Run the original Core workflow cases with the exact installed candidate CLI.
// Cargo builds only the test harness; AIT_TEST_CLI_BIN selects CLI children.
import { createHash } from "node:crypto";
import { lstatSync, mkdirSync, readFileSync, realpathSync } from "node:fs";
import path from "node:path";

export const installedCoreCases = Object.freeze([
  ["workflow_recovery_cli", "native_lifecycle_preserves_content_history_and_cleanup_without_git"],
  ["workflow_recovery_cli", "four_process_tasks_preserve_ownership_through_overlapping_finish_and_retry"],
  ["workflow_friction_cli", "dirty_and_clean_finish_close_the_bound_plan_once_and_preserve_siblings"],
  ["workflow_friction_cli", "concurrent_plan_items_reject_contention_and_preserve_shared_docs_closeout"],
  ["snapshot_task_binding_cli", "parallel_same_task_attempts_create_only_one_successful_receipt"],
  ["snapshot_task_binding_cli", "old_receipt_retry_preserves_the_newer_target_and_files"],
  ["workflow_package_frozen_candidate_cli", "sealed_batch_survives_later_local_finish_and_explicit_rebase_then_next_batch_is_independent"],
  ["workflow_package_frozen_candidate_cli", "fresh_incompatible_target_and_competing_finish_never_rebase_or_replace_ci"],
  ["workflow_package_accepted_retry_cli", "accepted_ready_and_finish_keep_the_original_batch_without_pin_after_newer_head_and_restart"],
  ["workflow_package_accepted_retry_cli", "accepted_partial_and_lost_responses_resolve_the_original_candidate_without_local_mappings"],
  ["workflow_package_accepted_retry_cli", "accepted_retry_does_not_require_materializable_local_source"],
  ["workflow_package_accepted_retry_cli", "reviewed_routed_server_preserves_sealed_mode_across_real_cli_processes", "paired"],
  ["workflow_package_accepted_retry_cli", "reviewed_routed_server_restart_reprepares_the_original_sealed_candidate", "paired"],
]);

export function requireExecutedCoreCase(stdout, selector) {
  const lines = String(stdout ?? "").replaceAll("\r\n", "\n").split("\n").map(line => line.trim());
  const executed = lines.filter(line => /^test .+ \.\.\. ok$/.test(line))
    .map(line => line.slice(5, -7));
  const summaries = lines.filter(line => line.startsWith("test result:"));
  if (lines.filter(line => line === "running 1 test").length !== 1 ||
      executed.length !== 1 || executed[0] !== selector || summaries.length !== 1 ||
      !/^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out;/.test(summaries[0])) {
    throw new Error(`installed Core case did not execute and pass exactly once: ${selector}`);
  }
}

function regular(file, label) {
  if (!path.isAbsolute(file ?? "")) throw new Error(`${label} must be absolute`);
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink()) throw new Error(`${label} must be a regular file`);
  return file;
}

function directory(root, label) {
  if (!path.isAbsolute(root ?? "")) throw new Error(`${label} must be absolute`);
  const stat = lstatSync(root);
  if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error(`${label} must be a real directory`);
  return realpathSync(root);
}

function json(file, label) {
  try { return JSON.parse(readFileSync(regular(file, label), "utf8")); }
  catch (error) { throw new Error(`${label} is invalid: ${error.message}`); }
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(regular(file, "installed binary"))).digest("hex");
}

export function runInstalledCoreCases({ recorder, sourceRoot, aitBin, serverBin, runnerBin,
  runtimeRoot, version, sourceCommit }) {
  const source = directory(sourceRoot, "candidate public source");
  const core = directory(path.join(source, "ait-core"), "candidate Core source");
  const runtime = directory(runtimeRoot, "clean-host runtime");
  for (const [file, label] of [[aitBin, "installed CLI"], [serverBin, "installed Server"],
    [runnerBin, "installed Runner"]]) regular(file, label);
  if (!/^[0-9a-f]{40}$/.test(sourceCommit ?? "")) throw new Error("candidate source commit is invalid");
  const head = recorder.run("git", ["-C", source, "rev-parse", "HEAD"],
    { label: "candidate public source commit" }).stdout.trim();
  if (head !== sourceCommit) throw new Error("candidate public source commit differs from frozen candidate");
  const familyFile = path.join(source, "ait-release-family.json");
  const family = json(familyFile, "candidate family manifest");
  const mapping = json(path.join(source, "ait-monorepo-source.json"), "candidate source mapping");
  const coreRelease = json(path.join(core, "ait-release.json"), "candidate Core release adapter");
  const snapshots = [...new Set((family.components ?? []).filter(row => row.source_repository === "ait-core")
    .map(row => row.source_snapshot))];
  if (family.schema !== "ait.release.family/v3" || family.family?.version !== version ||
      family.family?.tag !== `v${version}` || snapshots.length !== 1 ||
      !/^SNP-[0-9A-F]{12}$/.test(snapshots[0]) ||
      mapping.schema !== "ait.release.monorepo-source/v1" ||
      mapping.family_version !== version || mapping.family_tag !== `v${version}` ||
      mapping.family_manifest_sha256 !== sha256(familyFile) ||
      mapping.subtrees?.find(row => row.source_repository === "ait-core" && row.path === "ait-core")?.source_snapshot !== snapshots[0] ||
      coreRelease.package?.name !== "ait-core" || coreRelease.package?.version !== version) {
    throw new Error("candidate Core source differs from the frozen family identity");
  }
  const cli = realpathSync(aitBin), server = realpathSync(serverBin), runner = realpathSync(runnerBin);
  const ciRoot = path.join(runtime, "core-native-ci");
  mkdirSync(ciRoot, { recursive: true });
  const env = { ...process.env, AIT_TEST_CLI_BIN: cli,
    CARGO_TARGET_DIR: path.join(runtime, "core-cargo-target") };
  for (const key of ["TEST_REVIEWED_SERVER_BIN", "TEST_NATIVE_RUNNER_BIN", "TEST_SERVER_CI_RAM_ROOT"]) {
    delete env[key];
  }
  const pairedEnv = { ...env, TEST_REVIEWED_SERVER_BIN: server,
    TEST_NATIVE_RUNNER_BIN: runner, TEST_SERVER_CI_RAM_ROOT: ciRoot };
  const cases = [];
  for (const [target, selector, mode] of installedCoreCases) {
    const result = recorder.run("cargo", ["test", "--locked", "--manifest-path", path.join(core, "rust", "Cargo.toml"),
      "--profile", "ait-ci", "-p", "ait-cli", "--test", target, selector, "--", "--exact",
      ...(mode === "paired" ? ["--ignored"] : []), "--test-threads=1", "--nocapture"],
    { cwd: core, env: mode === "paired" ? pairedEnv : env, timeout: 20 * 60 * 1000,
      label: `installed Core ${target}/${selector}` });
    requireExecutedCoreCase(result.stdout, selector);
    cases.push({ target, selector, mode: mode ?? "fixture" });
  }
  return { source_commit: sourceCommit, core_snapshot: snapshots[0],
    cli_sha256: sha256(cli), server_sha256: sha256(server), runner_sha256: sha256(runner), cases };
}
