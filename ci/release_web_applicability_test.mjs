import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import os from "node:os";
import { correctedCohort, previousCohort, replacementIdentity, validateReplacementDiff, isApprovedReplacement, requireUnaffectedRandom } from "./release_web_applicability.mjs";
import { runInstalledTeamCases, teamCases } from "./release_core_team_cases.mjs";

// Isolated source/producer protocols. These fixtures are never release evidence.
const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-applicability-")));
const put = (file, text) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, text); };
const save = (file, value) => put(file, JSON.stringify(value) + "\n");
const sha = file => createHash("sha256").update(readFileSync(file)).digest("hex");
try {
  const cli = path.join(root, "old-cli"); put(cli, "fixture candidate\n");
  const prior = { components: previousCohort, executables: { ait: { path: cli, sha256: sha(cli) } } };
  const names = {
    core: ["src/agent_teams/activation.rs", "src/agent_teams/mod.rs", "src/binding_output.rs", "src/binding_team.rs", "src/config_surface.rs", "tests/team_bootstrap_cli.rs", "tests/team_caller_activation_cli.rs"].map(name => "rust/crates/ait-cli/" + name),
    node: ["ait-external.lock", "ait-external.toml", "ci/run.ps1", "ci/run.sh", "lib/npm-payload-contract.json", "release/npm-payload-package.mjs", "release/release-adapter.mjs", "scripts/native-build.mjs", "src/runtime.js", "test/package.test.js"],
    python: ["ait-external.lock", "ait-external.toml", "tests/test_package.py"],
  };
  const diffs = Object.fromEntries(Object.entries(names).map(([component, files]) => [component, {
    old_snapshot_id: previousCohort[component], new_snapshot_id: correctedCohort[component], added: [], deleted: [], rename_hints: [], directory_move_hints: [], mode_changed: [],
    files: files.map(file => ({ path: file, status: "modified", diff: { status: "text", text: `--- old\n+++ new\n-snapshot = "${previousCohort.core}"\n+snapshot = "${correctedCohort.core}"\n` } })),
  }]));
  const accepted = { release: { version: "1.2.0", channel: "stable" }, paths: { core_root: path.join(root, "core") },
    component_closeouts: Object.entries({ core: "RCT-1939", node: "RNT-0096", python: "RPT-0089" }).map(([id, task]) =>
      ({ id, task, patchset: task + "/P-01", snapshot: correctedCohort[id], remote: "origin", repository_root: path.join(root, id) })) };
  const reader = (directory, args) => {
    const component = path.basename(directory); assert.deepEqual(args, ["snapshot", "diff", previousCohort[component], correctedCohort[component], "--include-text", "--max-bytes", "1048576", "--json"]);
    return structuredClone(diffs[component]);
  };
  assert(replacementIdentity(accepted, prior, correctedCohort, reader));
  assert(!isApprovedReplacement({ ...correctedCohort, server: "SNP-999999999999" }, prior));
  assert(!isApprovedReplacement(correctedCohort, { components: { ...previousCohort, extra: "SNP-AAAAAAAAAAAA" } }));
  const badOwner = structuredClone(accepted); badOwner.component_closeouts[0].task = "RCT-1938";
  assert.throws(() => replacementIdentity(badOwner, prior, correctedCohort, reader));
  for (const mutate of [value => value.files.push({ path: "rust/crates/ait-core/src/workflow.rs", status: "modified" }),
    value => value.mode_changed.push("rust/crates/ait-cli/src/agent_teams/mod.rs"), value => value.old_snapshot_id = "SNP-AAAAAAAAAAAA"] ) {
    const value = structuredClone(diffs.core); mutate(value); assert.throws(() => validateReplacementDiff("core", value));
  }
  const changedLogic = structuredClone(diffs.node); changedLogic.files[0].diff.text += "+allow_other_snapshot = true\n";
  assert.throws(() => validateReplacementDiff("node", changedLogic), /more than exact guard literals/);
  const wrongLiteral = structuredClone(diffs.python); wrongLiteral.files[0].diff.text = wrongLiteral.files[0].diff.text.replace(correctedCohort.core, "SNP-AAAAAAAAAAAA");
  assert.throws(() => validateReplacementDiff("python", wrongLiteral), /Core pin\/guard literals/);
  const invocation = (id, args, expectation) => [id, { argv: [cli, ...args], row: ["1", "1", id, args.slice(0, 2).join(" "), expectation] }];
  requireUnaffectedRandom([invocation("help-init", ["init", "--help"], "text-success"),
    invocation("invalid-enable", ["config", "set", "--ait-web-test-invalid-fixture"], "nonzero-fail-closed"),
    invocation("read-config", ["config", "show", "--json"], "json-success")]);
  assert.throws(() => requireUnaffectedRandom([invocation("changed-init", ["init", "--json"], "json-success")]), /changed-init/);
  assert.throws(() => requireUnaffectedRandom([invocation("changed-ensure", ["team", "session", "ensure", "--agent", "0"], "json-success")]), /changed-ensure/);

  if (process.platform !== "win32") {
    const source = path.join(root, "source"), runtime = path.join(root, "runtime"), selected = path.join(root, "selected-cli"), marker = path.join(root, "candidate-used");
    mkdirSync(source); mkdirSync(runtime);
    put(selected, `#!${process.execPath}\nrequire('fs').appendFileSync(${JSON.stringify(marker)},'selected\\n');\n`); chmodSync(selected, 0o755);
    const family = { schema: "ait.release.family/v3", family: { version: "1.2.0", tag: "v1.2.0" },
      components: ["core", "node", "python", "server", "runner"].map(id => ({ source_repository: "ait-" + id, source_snapshot: correctedCohort[id] })) };
    save(path.join(source, "ait-release-family.json"), family);
    save(path.join(source, "ait-monorepo-source.json"), { schema: "ait.release.monorepo-source/v1", family_version: "1.2.0", family_tag: "v1.2.0",
      family_manifest_sha256: sha(path.join(source, "ait-release-family.json")), subtrees: family.components.map(row => ({ ...row, path: row.source_repository })) });
    save(path.join(source, "ait-core/ait-release.json"), { package: { name: "ait-core", version: "1.2.0" } });
    const git = args => { const result = spawnSync("git", ["-C", source, ...args], { encoding: "utf8" }); assert.equal(result.status, 0, result.stderr); return result.stdout.trim(); };
    git(["init", "-q"]); git(["config", "user.name", "Fixture"]); git(["config", "user.email", "fixture@example.test"]);
    git(["add", "."]); git(["commit", "-qm", "fixture"]); const sourceCommit = git(["rev-parse", "HEAD"]);
    const record = (expectedCli, duplicate = false, escape = null) => ({ run(command, args, options) {
      if (command === "cargo") {
        assert(args.includes("--no-run")); assert(!options.env["CARGO_BIN_EXE_ait-cli"]);
        const suite = args[args.indexOf("--test") + 1];
        // Real Cargo places libtest executables under build-dir/deps. A
        // target-only environment must reproduce the canonical-cache escape.
        const canonical = path.join(root, "canonical-build", "deps", suite);
        const build = options.env.CARGO_BUILD_BUILD_DIR ?? path.dirname(canonical);
        let executable = escape ? canonical : path.join(build, "ait-ci", "deps", suite);
        put(executable, `#!${process.execPath}\nconst r=require('child_process').spawnSync(process.env['CARGO_BIN_EXE_ait-cli'],['fixture-probe']);if(r.status!==0)process.exit(r.status??65);const selector=process.argv[3];process.stdout.write('running 1 test\\ntest '+selector+' ... ok\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.1s\\n');\n`); chmodSync(executable, 0o755);
        if (escape === "symlink") {
          const alias = path.join(options.env.CARGO_TARGET_DIR, "untrusted-" + suite);
          symlinkSync(executable, alias); executable = alias;
        }
        const artifact = { reason: "compiler-artifact", target: { name: suite }, profile: { test: true }, executable };
        return { stdout: [artifact, ...(duplicate ? [artifact] : [])].map(row => JSON.stringify(row)).join("\n") + "\n" };
      }
      if (command !== "git") assert.equal(options.env["CARGO_BIN_EXE_ait-cli"], expectedCli);
      const result = spawnSync(command, args, { ...options, encoding: "utf8" }); assert.equal(result.status, 0, options.label); return result;
    } });
    const options = { sourceRoot: source, aitBin: selected, runtimeRoot: runtime, version: "1.2.0", sourceCommit };
    const result = runInstalledTeamCases({ ...options, recorder: record(selected) });
    assert.equal(result.cli_sha256, sha(selected)); assert.equal(result.cases.length, teamCases.length);
    assert.equal(readFileSync(marker, "utf8").trim().split("\n").length, 16, "all affected fixtures must select the installed CLI");
    assert.throws(() => runInstalledTeamCases({ ...options, recorder: record(selected, true) }), /ambiguous/);
    for (const escape of ["canonical", "symlink"])
      assert.throws(() => runInstalledTeamCases({ ...options, recorder: record(selected, false, escape) }), /escaped its owned target/);
    assert.equal(readFileSync(marker, "utf8").trim().split("\n").length, 16, "escaped artifacts must not execute");
    const wrong = path.join(root, "wrong-cli"); put(wrong, "#!/bin/sh\nexit 42\n"); chmodSync(wrong, 0o755);
    assert.throws(() => runInstalledTeamCases({ ...options, aitBin: wrong, recorder: record(wrong) }));

    // Zero-dependency real Cargo regression, with Core's actual split config.
    // These small Rust fixtures test harness isolation and candidate selection;
    // they are not the product's sixteen preservation assertions.
    const core = path.join(source, "ait-core"), rust = path.join(core, "rust");
    put(path.join(core, ".cargo/config.toml"), '[build]\ntarget-dir = ".ait/cargo-target"\nbuild-dir = ".ait/cargo-build/canonical"\n');
    put(path.join(rust, "Cargo.toml"), '[package]\nname = "ait-cli"\nversion = "0.0.0"\nedition = "2021"\n[lib]\npath = "lib.rs"\n[profile.ait-ci]\ninherits = "dev"\n' +
      [...new Set(teamCases.map(([suite]) => suite))].map(suite => `[[test]]\nname = "${suite}"\npath = "tests/${suite}.rs"\n`).join(""));
    put(path.join(rust, "Cargo.lock"), 'version = 4\n[[package]]\nname = "ait-cli"\nversion = "0.0.0"\n');
    put(path.join(rust, "lib.rs"), "// Owned Cargo isolation fixture.\n");
    for (const suite of new Set(teamCases.map(([suite]) => suite)))
      put(path.join(rust, "tests", suite + ".rs"), teamCases.filter(([target]) => target === suite).map(([, selector]) =>
        `#[test]\nfn ${selector}() {\n let cli = std::env::var("CARGO_BIN_EXE_ait-cli").unwrap();\n assert!(std::process::Command::new(cli).arg("fixture-probe").status().unwrap().success());\n}\n`).join("\n"));
    git(["add", "."]); git(["commit", "-qm", "split Cargo build fixture"]); const realCommit = git(["rev-parse", "HEAD"]);
    let builds = 0, executions = 0;
    const realRecorder = { run(command, args, settings) {
      const result = spawnSync(command, args, { ...settings, encoding: "utf8" }); assert.equal(result.status, 0, result.stderr);
      if (command === "cargo") {
        builds++;
        assert.equal(settings.env.CARGO_BUILD_BUILD_DIR, path.join(settings.env.CARGO_TARGET_DIR, "build"));
        const artifacts = result.stdout.trim().split("\n").map(line => JSON.parse(line)).filter(row => row.reason === "compiler-artifact" && row.profile.test && row.executable);
        assert.equal(artifacts.length, 1); assert(realpathSync(artifacts[0].executable).startsWith(realpathSync(settings.env.CARGO_BUILD_BUILD_DIR) + path.sep));
      } else if (command !== "git") { executions++; assert.equal(settings.env["CARGO_BIN_EXE_ait-cli"], selected); }
      return result;
    } };
    const real = runInstalledTeamCases({ ...options, sourceCommit: realCommit, recorder: realRecorder });
    assert.equal(builds, 2); assert.equal(executions, 16); assert.equal(real.cases.length, 16);
    assert.equal(readFileSync(marker, "utf8").trim().split("\n").length, 32);
    assert(!existsSync(path.join(core, ".ait")), "real Cargo wrote to the canonical source cache");
    assert.equal(git(["status", "--porcelain", "--untracked-files=all"]), "");
  }
} finally { rmSync(root, { recursive: true, force: true }); }
console.log("bounded applicability and installed-candidate selection checks passed");
