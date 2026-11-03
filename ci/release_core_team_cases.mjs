// Existing Core fixtures, executed with the installed candidate selected at
// the test-process boundary. Cargo's own CLI build is never the tested child.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { lstatSync, mkdirSync, readFileSync, realpathSync, mkdtempSync, rmSync, writeFileSync, readdirSync } from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";
import { requireExecutedCoreCase } from "./release_core_installed_cases.mjs";
import { correctedCohort } from "./release_web_applicability.mjs";

export const teamCases = Object.freeze([
  ...["init_and_config_preserve_owner_developer_and_workflow_without_host_access",
    "unfinished_native_creation_is_preserved_until_explicit_ensure_recovery",
    "init_and_enable_preserve_a_developer_binding_without_an_owner",
    "new_init_four_seats_and_repeated_enable_preserve_newton"].map(selector => ["team_bootstrap_cli", selector]),
  ...["init_and_enable_preserve_unresolved_activation_without_host_access_or_implicit_ack",
    "existing_init_and_enable_preserve_archived_or_missing_owners",
    "existing_enable_preserves_uncertain_or_mismatched_owners_without_host_inspection",
    "pending_initial_offer_refresh_requires_explicit_ensure_from_its_original_caller",
    "enable_acknowledgment_and_changed_revision_use_the_busy_active_caller",
    "incomplete_offer_requires_same_caller_and_native_metadata_before_closeout",
    "wrong_workspace_archived_unknown_and_conflicting_native_ids_never_adopt",
    "two_repository_entrypoints_refresh_defaults_and_local_drift_without_crossing_bindings",
    "reviewer_default_refresh_preserves_custom_text_without_false_delivery",
    "customized_upgrade_conflict_preserves_and_syncs_local_role_without_false_delivery",
    "acknowledgment_rejects_unsynced_drift_and_host_failure_preserves_sync_outcome",
    "repository_name_controls_init_config_acknowledgment_and_current_binding_reconciliation"].map(selector => ["team_caller_activation_cli", selector]),
]);
export const teamCaptureProgram = "const [module,...args]=process.argv.slice(1); const {pathToFileURL}=await import('node:url'); const {captureInstalledTeamCases}=await import(pathToFileURL(module)); captureInstalledTeamCases(...args);";

const regular = file => {
  assert(path.isAbsolute(file)); const stat = lstatSync(file); assert(stat.isFile() && !stat.isSymbolicLink()); return readFileSync(file);
};
const sha = file => createHash("sha256").update(regular(file)).digest("hex");
const json = file => JSON.parse(regular(file));

export function runInstalledTeamCases({ recorder, sourceRoot, aitBin, runtimeRoot, version, sourceCommit }) {
  assert.notEqual(process.platform, "win32", "Core team fixtures are Unix-only; do not claim an unexecuted Windows result");
  const source = realpathSync(sourceRoot), runtime = realpathSync(runtimeRoot), cli = realpathSync(aitBin);
  regular(cli); assert.equal(version, "1.2.0"); assert.match(sourceCommit, /^[0-9a-f]{40}$/);
  assert.equal(recorder.run("git", ["-C", source, "rev-parse", "HEAD"], { label: "affected candidate source commit" }).stdout.trim(), sourceCommit);
  assert.equal(recorder.run("git", ["-C", source, "status", "--porcelain", "--untracked-files=all"], { label: "affected candidate source state" }).stdout.trim(), "");
  const familyPath = path.join(source, "ait-release-family.json"), family = json(familyPath);
  const mapping = json(path.join(source, "ait-monorepo-source.json"));
  assert.equal(family.schema, "ait.release.family/v3"); assert.equal(family.family.version, version); assert.equal(family.family.tag, "v" + version);
  assert.equal(mapping.schema, "ait.release.monorepo-source/v1"); assert.equal(mapping.family_version, version);
  assert.equal(mapping.family_tag, "v" + version); assert.equal(mapping.family_manifest_sha256, sha(familyPath));
  for (const component of ["core", "node", "python", "server", "runner"]) {
    const repository = "ait-" + component;
    assert.deepEqual([...new Set(family.components.filter(row => row.source_repository === repository).map(row => row.source_snapshot))], [correctedCohort[component]]);
    assert.equal(mapping.subtrees.find(row => row.source_repository === repository && row.path === repository)?.source_snapshot, correctedCohort[component]);
  }
  const core = path.join(source, "ait-core"), release = json(path.join(core, "ait-release.json"));
  assert.equal(release.package.name, "ait-core"); assert.equal(release.package.version, version);
  const target = path.join(runtime, "core-cargo-target"); mkdirSync(target, { recursive: true });
  // Core config separates intermediate build output from final targets. Keep
  // both inside this invocation's runtime, including libtest executables.
  const env = { ...process.env, AIT_TEST_CLI_BIN: cli, CARGO_TARGET_DIR: target,
    CARGO_BUILD_BUILD_DIR: path.join(target, "build") };
  delete env["CARGO_BIN_EXE_ait-cli"];
  for (const key of ["TEST_REVIEWED_SERVER_BIN", "TEST_NATIVE_RUNNER_BIN", "TEST_SERVER_CI_RAM_ROOT"]) delete env[key];
  const executables = new Map();
  for (const suite of new Set(teamCases.map(([suite]) => suite))) {
    const built = recorder.run("cargo", ["test", "--locked", "--manifest-path", path.join(core, "rust/Cargo.toml"),
      "--profile", "ait-ci", "-p", "ait-cli", "--test", suite, "--no-run", "--message-format=json"],
    { cwd: core, env, timeout: 20 * 60 * 1000, label: "build installed Core team harness " + suite });
    const artifacts = built.stdout.trim().split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line))
      .filter(row => row.reason === "compiler-artifact" && row.target?.name === suite && row.profile?.test === true && row.executable);
    assert.equal(artifacts.length, 1, "missing or ambiguous Core team test executable");
    const executable = realpathSync(artifacts[0].executable); regular(executable);
    assert(executable.startsWith(realpathSync(target) + path.sep), "Core team harness escaped its owned target");
    executables.set(suite, executable);
  }
  for (const [suite, selector] of teamCases) {
    const tested = recorder.run(executables.get(suite), ["--exact", selector, "--test-threads=1", "--nocapture"],
      { cwd: core, env: { ...env, "CARGO_BIN_EXE_ait-cli": cli }, timeout: 20 * 60 * 1000,
        label: `installed Core ${suite}/${selector}` });
    requireExecutedCoreCase(tested.stdout, selector);
  }
  return { source_commit: sourceCommit, core_snapshot: correctedCohort.core, cli_sha256: sha(cli),
    cases: teamCases.map(([target, selector]) => ({ target, selector, mode: "fixture" })) };
}

export function captureInstalledTeamCases(sourceRoot, aitBin, version, sourceCommit) {
  const output = process.env.AIT_WEB_TEST_RESULTS_DIR; assert(output && path.isAbsolute(output));
  const runtime = realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-installed-team-")));
  let sequence = 0;
  const recorder = { run(command, args, options) {
    const prefix = path.join(output, "binding." + String(++sequence).padStart(3, "0"));
    writeFileSync(prefix + ".argv", [command, ...args].join("\n") + "\n");
    const result = spawnSync(command, args, { ...options, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    writeFileSync(prefix + ".stdout", result.stdout ?? ""); writeFileSync(prefix + ".stderr", result.stderr ?? "");
    writeFileSync(prefix + ".exit", String(result.status ?? 65) + "\n");
    assert.equal(result.status, 0, `${options.label}: ${result.stderr ?? result.error}`); return result;
  } };
  try {
    const result = runInstalledTeamCases({ recorder, sourceRoot, aitBin, version, sourceCommit, runtimeRoot: runtime });
    process.stdout.write(JSON.stringify(result) + "\n");
  } finally { rmSync(runtime, { recursive: true, force: true }); }
}

export function verifyInstalledTeamCaptures(directory, input, candidate, accepted) {
  const capture = number => {
    const prefix = path.join(directory, "binding." + String(number).padStart(3, "0"));
    assert.equal(regular(prefix + ".exit").toString(), "0\n"); regular(prefix + ".stderr");
    return { argv: regular(prefix + ".argv").toString().trimEnd().split("\n"), stdout: regular(prefix + ".stdout").toString() };
  };
  const outer = JSON.parse(regular(path.join(directory, "ait.semantic.init.stdout")));
  const source = accepted.paths.public_source, core = path.join(source, "ait-core");
  assert.equal(outer.source_commit, candidate.release.source_commit); assert.equal(outer.core_snapshot, correctedCohort.core);
  assert.equal(outer.cli_sha256, input.executables.ait.sha256);
  assert.deepEqual(outer.cases, teamCases.map(([target, selector]) => ({ target, selector, mode: "fixture" })));
  assert.deepEqual(regular(path.join(directory, "ait.semantic.init.argv")).toString().trimEnd().split("\n"),
    [process.execPath, "--input-type=module", "-e", teamCaptureProgram, path.join(accepted.paths.native_root, "ci/release_core_team_cases.mjs"),
      source, input.executables.ait.path, input.release.version, candidate.release.source_commit]);
  assert.equal(regular(path.join(directory, "ait.semantic.init.stderr")).length, 0);
  const head = capture(1), clean = capture(2);
  assert.deepEqual(head.argv, ["git", "-C", source, "rev-parse", "HEAD"]); assert.equal(head.stdout.trim(), candidate.release.source_commit);
  assert.deepEqual(clean.argv, ["git", "-C", source, "status", "--porcelain", "--untracked-files=all"]); assert.equal(clean.stdout.trim(), "");
  const executables = new Map(); let sequence = 2;
  for (const suite of new Set(teamCases.map(([suite]) => suite))) {
    const built = capture(++sequence);
    assert.deepEqual(built.argv, ["cargo", "test", "--locked", "--manifest-path", path.join(core, "rust/Cargo.toml"),
      "--profile", "ait-ci", "-p", "ait-cli", "--test", suite, "--no-run", "--message-format=json"]);
    const artifacts = built.stdout.trim().split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line))
      .filter(row => row.reason === "compiler-artifact" && row.target?.name === suite && row.profile?.test === true && row.executable);
    assert.equal(artifacts.length, 1); assert(path.isAbsolute(artifacts[0].executable));
    assert.notEqual(artifacts[0].executable, input.executables.ait.path); executables.set(suite, artifacts[0].executable);
  }
  for (const [suite, selector] of teamCases) {
    const tested = capture(++sequence);
    assert.deepEqual(tested.argv, [executables.get(suite), "--exact", selector, "--test-threads=1", "--nocapture"]);
    requireExecutedCoreCase(tested.stdout, selector);
  }
  const expected = ["cases.tsv", ...["argv", "stdout", "stderr"].map(suffix => "ait.semantic.init." + suffix),
    ...Array.from({ length: sequence }, (_, index) => ["argv", "stdout", "stderr", "exit"].map(suffix =>
      "binding." + String(index + 1).padStart(3, "0") + "." + suffix)).flat()];
  assert.deepEqual(readdirSync(directory).sort(), expected.sort(), "fresh installed binding capture inventory differs");
}
