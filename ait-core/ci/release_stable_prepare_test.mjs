#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, lstatSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  copyOrVerify,
  materializeDirectory,
  replaceComponentSources,
  stableAdvance,
  writeOrVerify,
} from "./release_stable_prepare.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const temporary = mkdtempSync(path.join(os.tmpdir(), "ait-release-prepare-test."));
const oldVersion = "1.1.2";
const version = "1.1.3";
const oldPin = "SNP-111111111111";
const pin = "SNP-222222222222";

function write(root, relative, text, mode = 0o644) {
  const file = path.join(root, relative);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, text, { mode });
}

function repeated(token, amount) {
  return Array.from({ length: amount }, (_, index) => `${index}:${token}`).join("\n") + "\n";
}

function packageLock(names) {
  return names.map((name) => `[[package]]\nname = "${name}"\nversion = "${oldVersion}"\n`).join("\n");
}

function external() {
  return `[[external]]\nname = "ait-core"\nsnapshot = "${oldPin}"\nversion = "${oldVersion}"\n`;
}

try {
  assert.equal(stableAdvance("1.1.2", "1.1.3"), true);
  assert.equal(stableAdvance("1.1.3", "1.2.0"), true);
  assert.equal(stableAdvance("1.1.2", "1.1.4"), false);

  const core = path.join(temporary, "core");
  write(core, "rust/Cargo.toml", repeated(oldVersion, 1));
  write(core, "rust/Cargo.lock", packageLock(["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]) + `\n[[package]]\nname = "ait-core"\nversion = "0.1.0"\n\n[[package]]\nname = "atomic-waker"\nversion = "${oldVersion}"\n`);
  for (const [relative, amount] of [["ait-release.json", 1], ["ait-release-family.json", 9], ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1]]) {
    write(core, relative, repeated(oldVersion, amount));
  }
  const dryRunCoreFiles = ["rust/Cargo.toml", "rust/Cargo.lock", "ait-release.json", "ait-release-family.json", "ci/native_bootstrap_matrix.json", "ci/release_repository_authorities.json"];
  const dryRunCoreBytes = new Map(dryRunCoreFiles.map((relative) => [relative, readFileSync(path.join(core, relative))]));
  assert.equal(replaceComponentSources(core, "core", oldVersion, version, null, false).length, 6);
  for (const [relative, bytes] of dryRunCoreBytes) assert.deepEqual(readFileSync(path.join(core, relative)), bytes);
  assert.equal(replaceComponentSources(core, "core", oldVersion, version).length, 6);
  const forwardCoreLock = readFileSync(path.join(core, "rust/Cargo.lock"), "utf8");
  assert.match(forwardCoreLock, /ait-napi"\nversion = "1\.1\.3"/);
  assert.match(forwardCoreLock, /ait-core"\nversion = "0\.1\.0"/);
  assert.match(forwardCoreLock, /atomic-waker"\nversion = "1\.1\.2"/);
  assert.equal(replaceComponentSources(core, "core", version, oldVersion).length, 6);
  const reverseCoreLock = readFileSync(path.join(core, "rust/Cargo.lock"), "utf8");
  assert.match(reverseCoreLock, /ait-napi"\nversion = "1\.1\.2"/);
  assert.match(reverseCoreLock, /ait-core"\nversion = "0\.1\.0"/);

  const server = path.join(temporary, "server");
  for (const relative of ["rust/crates/ait-server/Cargo.toml", "ait-release.json", "rust/crates/ait-server/src/router/http/binary_tests.rs"]) write(server, relative, repeated(oldVersion, 1));
  write(server, "rust/Cargo.lock", packageLock(["ait-server"]));
  assert.equal(replaceComponentSources(server, "server", oldVersion, version).length, 4);

  const runner = path.join(temporary, "runner");
  write(runner, "Cargo.toml", repeated(oldVersion, 1));
  write(runner, "Cargo.lock", packageLock(["ait-runner"]));
  write(runner, "ait-release.json", repeated(oldVersion, 1));
  write(runner, "ait-external.toml", external());
  write(runner, "ait-external.lock", external());
  write(runner, "tests/cli.rs", repeated(oldPin, 1));
  assert.equal(replaceComponentSources(runner, "runner", oldVersion, version, pin).length, 6);

  const python = path.join(temporary, "python");
  const pythonCounts = new Map([["pyproject.toml", 1], ["ait-release.json", 7], ["src/ait_python/__init__.py", 1], ["tests/test_package.py", 2], ["tests/test_release_adapter.py", 14]]);
  for (const [relative, amount] of pythonCounts) write(python, relative, repeated(oldVersion, amount));
  write(python, "ait-external.toml", external());
  write(python, "ait-external.lock", external());
  write(python, "tests/test_package.py", repeated(oldVersion, 2) + repeated(oldPin, 1));
  assert.equal(replaceComponentSources(python, "python", oldVersion, version, pin).length, 7);

  const node = path.join(temporary, "node");
  const nodeCounts = new Map([
    ["package.json", 7], ["ait-release.json", 8], ["lib/npm-payload-contract.json", 7], ["test/payload-package.test.js", 3],
    ["test/package.test.js", 1], ["test/cli.test.js", 2], ["test/release-adapter.test.js", 9], ["test/runtime.test.js", 1],
    ["ci/run.ps1", 2], ["ci/run.sh", 2], ["release/release-adapter.mjs", 2], ["release/npm-payload-package.mjs", 1],
    ["scripts/installed-smoke.mjs", 1], ["scripts/native-build.mjs", 1], ["src/runtime.js", 1],
  ]);
  for (const [relative, amount] of nodeCounts) write(node, relative, repeated(oldVersion, amount));
  write(node, "ait-external.toml", external());
  write(node, "ait-external.lock", external());
  write(node, "test/package.test.js", repeated(oldVersion, 1) + repeated(oldVersion.replaceAll(".", "\\."), 1) + repeated(oldPin, 2));
  write(node, "test/release-adapter.test.js", repeated(oldVersion, 9) + repeated(oldVersion.replaceAll(".", "\\."), 2));
  for (const [relative, amount] of new Map([["lib/npm-payload-contract.json", 6], ["ci/run.ps1", 1], ["ci/run.sh", 1], ["release/release-adapter.mjs", 1], ["release/npm-payload-package.mjs", 1], ["scripts/native-build.mjs", 1], ["src/runtime.js", 1]])) {
    write(node, relative, readFileSync(path.join(node, relative), "utf8") + repeated(oldPin, amount));
  }
  assert.equal(replaceComponentSources(node, "node", oldVersion, version, pin).length, 17);
  assert.doesNotMatch(readFileSync(path.join(node, "lib/npm-payload-contract.json"), "utf8"), /1\.1\.2|SNP-111111111111/);

  const bad = path.join(temporary, "bad-core");
  write(bad, "rust/Cargo.toml", repeated(oldVersion, 1));
  write(bad, "rust/Cargo.lock", packageLock(["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]) + `\n[[package]]\nname = "ait-core"\nversion = "0.1.0"\n`);
  for (const [relative, amount] of [["ait-release.json", 1], ["ait-release-family.json", 8], ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1]]) write(bad, relative, repeated(oldVersion, amount));
  assert.throws(() => replaceComponentSources(bad, "core", oldVersion, version), /token inventory differs/);

  const retainedSource = path.join(temporary, "retained-source.json");
  const retainedCopy = path.join(temporary, "retained-copy.json");
  writeFileSync(retainedSource, "{\"stable\":true}\n");
  copyOrVerify(retainedSource, retainedCopy, "retained test copy");
  copyOrVerify(retainedSource, retainedCopy, "retained test copy");
  assert.equal(readFileSync(retainedCopy, "utf8"), "{\"stable\":true}\n");
  const retainedText = path.join(temporary, "retained-text.json");
  writeOrVerify(retainedText, "{\"status\":\"pass\"}\n", "retained test text");
  writeOrVerify(retainedText, "{\"status\":\"pass\"}\n", "retained test text");

  const stagedDirectory = path.join(temporary, "staged-directory");
  let materializations = 0;
  const materialize = () => materializeDirectory(stagedDirectory, (pending) => {
    materializations += 1;
    mkdirSync(pending, { recursive: true });
    writeFileSync(path.join(pending, "receipt.json"), "{\"status\":\"pass\"}\n");
  }, (candidate) => assert.equal(readFileSync(path.join(candidate, "receipt.json"), "utf8"), "{\"status\":\"pass\"}\n"));
  materialize();
  materialize();
  assert.equal(materializations, 1, "retained directory must resume without rerunning its producer");

  const interruptedDirectory = path.join(temporary, "interrupted-directory");
  mkdirSync(`${interruptedDirectory}.pending`, { recursive: true });
  writeFileSync(path.join(`${interruptedDirectory}.pending`, "partial"), "partial\n");
  materializeDirectory(interruptedDirectory, (pending) => {
    mkdirSync(pending, { recursive: true });
    writeFileSync(path.join(pending, "complete"), "complete\n");
  }, (candidate) => assert.equal(readFileSync(path.join(candidate, "complete"), "utf8"), "complete\n"));
  assert.equal(existsSync(path.join(interruptedDirectory, "partial")), false);
  assert.equal(readFileSync(path.join(interruptedDirectory, "complete"), "utf8"), "complete\n");

  const normalizeSource = path.join(temporary, "normalize-source");
  const normalizeOutput = path.join(temporary, "normalize-output");
  write(normalizeSource, "plain", "plain\n", 0o600);
  write(normalizeSource, "bin/tool", "tool\n", 0o700);
  const normalized = spawnSync(process.execPath, [path.join(here, "release_source_normalize.mjs"), "--source", normalizeSource, "--output", normalizeOutput], { encoding: "utf8" });
  assert.equal(normalized.status, 0, normalized.stderr);
  assert.equal(lstatSync(path.join(normalizeOutput, "plain")).mode & 0o777, 0o644);
  assert.equal(lstatSync(path.join(normalizeOutput, "bin/tool")).mode & 0o777, 0o755);

  const source = readFileSync(path.join(here, "release_stable_prepare.mjs"), "utf8");
  assert.doesNotMatch(source, /change[_-]id/i);
  assert.match(source, /state\.components\[component\] = row;\n\s+save\(\);/);
  assert.match(source, /row\.progress = "finalizing";\n\s+save\(\);/);
  assert.match(source, /readJson\(state\.steps\.freeze_prior\.coordinator/);
  assert.match(source, /sourceCache\("prior", priorSnapshot,/);
  assert.doesNotMatch(source, /priorFamily\.components\.filter/);
  assert.match(source, /const webComponents = path\.join\(request\.records_root, "web-components\.json"\)/);
  assert.match(source, /let coordinator = state\.coordinator/);
  assert.match(source, /coordinator\.progress = "finalizing";\n\s+save\(\);/);
  assert.match(source, /refreshCoreArtifacts\("coordinator-core"\)/);
  const example = JSON.parse(readFileSync(path.join(here, "../release/stable-request.example.json"), "utf8"));
  assert.equal(example.paths.web_components, undefined);
  process.stdout.write("release stable preparation tests passed\n");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
