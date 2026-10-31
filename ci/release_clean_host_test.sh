#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
tool=${repo_root}/ci/release_clean_host.mjs
phase_runner=${repo_root}/ci/release_clean_host_phase.mjs
workflow=${repo_root}/.github/workflows/ait-release-prepublish-clean-host.yml
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-clean-host-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-clean-host-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected clean-host test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

# Keep this focused part executable by the existing native Windows Q lanes.
# The remaining shell/fixture controls continue in the ordinary Unix lane.
node_entry_host=unix
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) node_entry_host=windows ;; esac
node --input-type=module - "${repo_root}/ci/release_language_repositories.mjs" \
  "${node_entry_host}" <<'NODE_ENTRY'
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";
import { pathToFileURL } from "node:url";
const modulePath = process.argv[2];
if (process.argv[3] === "windows") assert.equal(process.platform, "win32");
const { languageRepository, seedLanguageRepository, verifyLanguageRepository, runLanguageRepository } =
  await import(pathToFileURL(modulePath));
const source = readFileSync(modulePath, "utf8");
const helper = source.slice(source.indexOf("function foreignRuntimeWorkingDirectory("),
  source.indexOf("export function runLanguageRepository("));
const ordinary = path.win32.resolve("C:/bound worktree/project");
const namespaced = path.win32.toNamespacedPath(ordinary);
let mismatch = false;
let zeroInode = false, wrongDevice = false, notDirectory = false;
const inspected = [];
const sandbox = vm.createContext({ process: { platform: "win32" },
  statSync(file, options) {
    assert.equal(options.bigint, true); inspected.push(file);
    return { dev: wrongDevice && file === ordinary ? 2n : 1n,
      ino: zeroInode ? 0n : mismatch && file === ordinary ? 2n : 1n,
      isDirectory: () => !notDirectory };
  },
});
vm.runInContext(helper + "\nthis.cwd = foreignRuntimeWorkingDirectory;", sandbox);
for (const command of ["node", "java", "javac"]) {
  inspected.length = 0;
  assert.equal(sandbox.cwd(namespaced, command), ordinary);
  assert.deepEqual(inspected, [namespaced, ordinary]);
  mismatch = true;
  assert.throws(() => sandbox.cwd(namespaced, command), /physical worktree/);
  mismatch = false;
  for (const invalid of ["zeroInode", "wrongDevice", "notDirectory"]) {
    zeroInode = invalid === "zeroInode"; wrongDevice = invalid === "wrongDevice"; notDirectory = invalid === "notDirectory";
    assert.throws(() => sandbox.cwd(namespaced, command), /physical worktree/);
  }
  zeroInode = wrongDevice = notDirectory = false;
}
inspected.length = 0;
for (const [root, command] of [[ordinary, "node"], [ordinary, "java"], [namespaced, "cargo"],
  [namespaced, "go"], [namespaced, "java.exe"], [namespaced, "C:\\JDK\\bin\\java.exe"],
  [path.win32.toNamespacedPath("//server/share/project"), "java"],
  [path.win32.toNamespacedPath("//server/share/project"), "node"], ["/ordinary/project", "javac"]]) {
  assert.equal(sandbox.cwd(root, command), root);
}
assert.equal(inspected.length, 0);
sandbox.process.platform = "darwin";
for (const command of ["node", "java", "javac"]) assert.equal(sandbox.cwd(namespaced, command), namespaced);

// This portion uses the real shared runner and real Node, with no VM mocks.
const root = mkdtempSync(path.join(process.env.RUNNER_TEMP ?? os.tmpdir(), "ait node entry "));
try {
  const fixture = languageRepository("typescript_node");
  seedLanguageRepository(root, fixture);
  const manifest = JSON.parse(readFileSync(path.join(root, "qualification.json"), "utf8"));
  const roots = process.platform === "win32" ? [root, path.toNamespacedPath(root)] : [root];
  for (const inputRoot of roots) {
    rmSync(path.join(root, "build"), { recursive: true, force: true });
    const calls = [];
    const recorder = corruptTest => ({ run(command, args, options) {
      calls.push({ command, args, cwd: options.cwd, label: options.label });
      if (corruptTest && options.label.endsWith("repository test")) {
        writeFileSync(path.join(root, "build/node/result.txt"), "incorrect");
      }
      const result = spawnSync(command, args, { ...options, encoding: "utf8" });
      if (result.error) throw result.error;
      if (result.status !== 0) throw new Error(options.label + " failed with " + result.status + ": " + result.stderr);
      return result;
    } });
    verifyLanguageRepository(inputRoot, fixture);
    runLanguageRepository(recorder(false), inputRoot, fixture);
    assert.equal(readFileSync(path.join(root, "build/node/result.txt"), "utf8"), "42");
    assert.deepEqual(calls.map(call => call.args), [["--version"], manifest.build.slice(1), manifest.test.slice(1)]);
    assert(calls.every(call => call.command === "node" && call.cwd === root));
    assert.deepEqual(calls.map(call => call.label), ["typescript_node toolchain node version",
      "typescript_node repository build", "typescript_node repository test"]);
    calls.length = 0;
    assert.throws(() => runLanguageRepository(recorder(true), inputRoot, fixture), /repository test failed/);
    assert.equal(calls.at(-1).label, "typescript_node repository test");
    assert(calls.every(call => call.cwd === root));
    verifyLanguageRepository(inputRoot, fixture);
  }
  console.log("Node entry-path regression: actual " + process.platform + "/" + process.arch + " " + process.version +
    "; real build/test path forms=" + roots.length + "; test failures propagate");
} finally { rmSync(root, { recursive: true, force: true }); }

// JVM execution belongs to the existing real Windows Q lanes. The Unix lane
// checks the boundary above; it cannot substitute for a native Windows JDK.
if (process.platform === "win32") {
  const javaRoot = mkdtempSync(path.join(process.env.RUNNER_TEMP ?? os.tmpdir(), "ait java launch "));
  const fixture = languageRepository("java");
  const run = (command, args, cwd) => {
    const result = spawnSync(command, args, { cwd, env: process.env, encoding: "utf8",
      timeout: 120000, maxBuffer: 16 * 1024 * 1024 });
    return { command, args, cwd, status: result.status, signal: result.signal,
      stdout: result.stdout ?? "", stderr: result.stderr ?? "", error: result.error?.message ?? null };
  };
  try {
    const roots = [javaRoot, path.toNamespacedPath(javaRoot)];
    for (const command of ["javac", "java"]) {
      const resolved = run("where.exe", [command], javaRoot);
      assert.equal(resolved.status, 0, JSON.stringify(resolved));
      const executable = realpathSync(resolved.stdout.trim().split(/\r?\n/)[0]);
      const identity = run(command, command === "java" ? ["-XshowSettings:properties", "-version"] :
        ["-J-XshowSettings:properties", "-version"], javaRoot);
      console.log("JVM actual PATH identity " + JSON.stringify({ executable, resolution: resolved, identity }));
      assert.equal(identity.status, 0, JSON.stringify(identity));
      assert.match(identity.stderr + identity.stdout, /os\.arch\s*=\s*\S+/);
    }
    for (const inputRoot of roots) {
      const a = statSync(javaRoot, { bigint: true }), b = statSync(inputRoot, { bigint: true });
      assert(a.isDirectory() && b.isDirectory() && a.ino !== 0n && a.ino === b.ino && a.dev === b.dev);
      seedLanguageRepository(javaRoot, fixture);
      const manifest = JSON.parse(readFileSync(path.join(javaRoot, "qualification.json"), "utf8"));
      rmSync(path.join(javaRoot, "build"), { recursive: true, force: true });
      mkdirSync(path.join(javaRoot, "build"));
      const raw = [...manifest.toolchains, manifest.build, manifest.test].map(([command, ...args]) => run(command, args, inputRoot));
      console.log("JVM raw path control " + JSON.stringify({ inputRoot, raw,
        mainClass: path.join(javaRoot, "build/java/Main.class"),
        mainClassExists: existsSync(path.join(javaRoot, "build/java/Main.class")) }));
      // Raw controls may succeed or fail on a given JDK. The adapted shared
      // runner must execute all four unchanged commands successfully.
      for (const failure of [null, "compile", "classpath", "text", "binary"]) {
        seedLanguageRepository(javaRoot, fixture);
        rmSync(path.join(javaRoot, "build"), { recursive: true, force: true });
        if (failure === "compile") writeFileSync(path.join(javaRoot, "src/Main.java"), "invalid Java source");
        const calls = [];
        const recorder = { run(command, args, options) {
          if (options.label === "java repository test") {
            const mainClass = path.join(javaRoot, "build/java/Main.class");
            assert(statSync(mainClass).isFile(), "compiler must emit the intended Main.class");
            if (failure === "classpath") rmSync(mainClass);
            if (failure === "text") writeFileSync(path.join(javaRoot, "data/space name/naïve.txt"), "incorrect\n");
            if (failure === "binary") writeFileSync(path.join(javaRoot, "data/bytes.bin"), Buffer.from([1]));
          }
          const result = run(command, args, options.cwd); calls.push(result);
          if (result.error || result.status !== 0) throw new Error(options.label + " failed: " + JSON.stringify(result));
          return result;
        } };
        if (failure) {
          assert.throws(() => runLanguageRepository(recorder, inputRoot, fixture),
            failure === "compile" ? /java repository build failed/ : /java repository test failed/);
        } else {
          verifyLanguageRepository(inputRoot, fixture);
          const toolchains = runLanguageRepository(recorder, inputRoot, fixture);
          assert.equal(toolchains.length, 2);
          verifyLanguageRepository(inputRoot, fixture);
          assert.deepEqual(calls.map(call => [call.command, ...call.args]),
            [...manifest.toolchains, manifest.build, manifest.test]);
          console.log("JVM shared runner path pair " + JSON.stringify({ inputRoot, calls,
            mainClass: realpathSync(path.join(javaRoot, "build/java/Main.class")) }));
        }
        assert(calls.every(call => call.cwd === javaRoot));
        assert.equal(calls.length, failure === "compile" ? 3 : 4);
      }
    }
    console.log("JVM launch regression: actual win32/" + process.arch +
      "; ordinary/namespaced probes, compile, classpath and fixture bytes passed; compile/classpath/text/binary failures propagate");
  } finally { rmSync(javaRoot, { recursive: true, force: true }); }
}
NODE_ENTRY
if [[ ${node_entry_host} == windows ]]; then
  printf 'Focused native Windows Node and JVM launch regressions passed\n'
  exit 0
fi

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected clean-host failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

node --check "${tool}"
node --check "${phase_runner}"
node --check "${repo_root}/ci/release_core_installed_cases.mjs"
node --check "${repo_root}/ci/release_language_repositories.mjs"
libtest_root=${temporary_root}/libtest-selector
mkdir -p "${libtest_root}/tests"
cat >"${libtest_root}/Cargo.toml" <<'TOML'
[package]
name = "ait_clean_host_selector_fixture"
version = "0.1.0"
edition = "2021"
TOML
cat >"${libtest_root}/tests/selected.rs" <<'RUST'
#[test]
fn native_lifecycle_preserves_content_history_and_cleanup_without_git() {
    assert_eq!(2 + 2, 4);
}
RUST
CARGO_TARGET_DIR="${temporary_root}/libtest-target" cargo test --offline \
  --manifest-path "${libtest_root}/Cargo.toml" --test selected \
  native_lifecycle_preserves_content_history_and_cleanup_without_git -- \
  --exact --test-threads=1 --nocapture >"${temporary_root}/libtest-selector.stdout"
node --input-type=module - "${repo_root}/ci/release_core_installed_cases.mjs" \
  "${temporary_root}/libtest-selector.stdout" <<'NODE'
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const { requireExecutedCoreCase } = await import(pathToFileURL(process.argv[2]));
requireExecutedCoreCase(readFileSync(process.argv[3], "utf8"),
  "native_lifecycle_preserves_content_history_and_cleanup_without_git");
NODE
node --input-type=module - "${repo_root}/ci/release_core_installed_cases.mjs" \
  "${temporary_root}/core-cases.json" <<'NODE'
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const { installedCoreCases } = await import(pathToFileURL(process.argv[2]));
writeFileSync(process.argv[3], JSON.stringify(installedCoreCases.map(([target, selector, mode]) =>
  ({ target, selector, mode: mode ?? "fixture" }))));
NODE
node --input-type=module - "${repo_root}/ci/release_language_repositories.mjs" \
  "${temporary_root}/language-toolchains.json" <<'NODE'
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const { languageRepository, languageRepositoryNames } = await import(pathToFileURL(process.argv[2]));
writeFileSync(process.argv[3], JSON.stringify(Object.fromEntries(languageRepositoryNames.map(name =>
  [name, languageRepository(name).toolchains.map(([command, ...args]) =>
    ({ command, args, version_output: `${command} 1.2.3` }))]))));
NODE
node --input-type=module - "${repo_root}/ci/release_core_installed_cases.mjs" \
  "${temporary_root}" <<'NODE'
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
const { installedCoreCases, requireExecutedCoreCase, runInstalledCoreCases } = await import(pathToFileURL(process.argv[2]));
const root = process.argv[3], sourceRoot = path.join(root, "candidate-source"),
  coreRoot = path.join(sourceRoot, "ait-core"), runtimeRoot = path.join(root, "candidate-runtime");
mkdirSync(coreRoot, { recursive: true }); mkdirSync(runtimeRoot);
const family = JSON.stringify({ schema: "ait.release.family/v3", family: { version: "1.2.0", tag: "v1.2.0" },
  components: [{ source_repository: "ait-core", source_snapshot: "SNP-123456789ABC" }] });
writeFileSync(path.join(sourceRoot, "ait-release-family.json"), family);
writeFileSync(path.join(sourceRoot, "ait-monorepo-source.json"), JSON.stringify({
  schema: "ait.release.monorepo-source/v1", family_version: "1.2.0", family_tag: "v1.2.0",
  family_manifest_sha256: createHash("sha256").update(family).digest("hex"),
  subtrees: [{ source_repository: "ait-core", path: "ait-core", source_snapshot: "SNP-123456789ABC" }],
}));
writeFileSync(path.join(coreRoot, "ait-release.json"), JSON.stringify({ package: { name: "ait-core", version: "1.2.0" } }));
const [aitBin, serverBin, runnerBin] = ["ait", "ait-server", "ait-runner"].map(name => path.join(root, name));
for (const binary of [aitBin, serverBin, runnerBin]) writeFileSync(binary, nameBytes(binary), { mode: 0o755 });
function nameBytes(file) { return `binary:${path.basename(file)}\n`; }
const commit = "1".repeat(40), calls = [];
const input = { sourceRoot, aitBin, serverBin, runnerBin, runtimeRoot, version: "1.2.0", sourceCommit: commit };
const recorder = { run(command, args, options) {
  calls.push({ command, args, options });
  if (command === "git") return { stdout: `${commit}\n` };
  assert.equal(command, "cargo");
  const selector = args[args.indexOf("--test") + 2];
  return { stdout: `running 1 test\ntest ${selector} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.01s\n` };
} };
const result = runInstalledCoreCases({ ...input, recorder });
assert.equal(result.source_commit, commit);
assert.equal(result.core_snapshot, "SNP-123456789ABC");
assert.equal(calls.filter(call => call.command === "cargo").length, installedCoreCases.length);
for (const [index, [target, selector, mode]] of installedCoreCases.entries()) {
  const call = calls[index + 1];
  assert.deepEqual(call.args.slice(0, 2), ["test", "--locked"]);
  assert(call.args.includes(target) && call.args.includes(selector) && call.args.includes("--exact"));
  assert.equal(call.args.includes("--ignored"), mode === "paired");
  assert.equal(realpathSync(call.options.cwd), realpathSync(coreRoot));
  assert.equal(realpathSync(call.options.env.AIT_TEST_CLI_BIN), realpathSync(aitBin));
  assert.equal(call.options.env.TEST_REVIEWED_SERVER_BIN && realpathSync(call.options.env.TEST_REVIEWED_SERVER_BIN),
    mode === "paired" ? realpathSync(serverBin) : undefined);
  assert.equal(call.options.env.TEST_NATIVE_RUNNER_BIN && realpathSync(call.options.env.TEST_NATIVE_RUNNER_BIN),
    mode === "paired" ? realpathSync(runnerBin) : undefined);
}
assert.throws(() => runInstalledCoreCases({ ...input, sourceCommit: "2".repeat(40), recorder }),
  /public source commit differs/);
for (const output of [
  "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;\n",
  "running 1 test\ntest selected ... ignored\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;\n",
]) assert.throws(() => requireExecutedCoreCase(output, "selected"), /did not execute and pass/);
assert.throws(() => runInstalledCoreCases({ ...input, recorder: { run(command) {
  if (command === "git") return { stdout: `${commit}\n` };
  throw new Error("required Core case failed");
} } }), /required Core case failed/);
NODE
node --input-type=module - "${phase_runner}" "${repo_root}/ci/release_language_repositories.mjs" "${temporary_root}" <<'NODE'
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { pathToFileURL } from "node:url";

const { languageRepository, languageRepositoryNames, seedLanguageRepository,
  verifyLanguageRepository, runLanguageRepository } = await import(pathToFileURL(process.argv[3]));
assert.deepEqual(languageRepositoryNames, ["rust", "typescript_node", "go", "java"]);
for (const name of languageRepositoryNames) {
  const fixture = languageRepository(name), root = path.join(process.argv[4], `language-${name}`);
  seedLanguageRepository(root, fixture);
  const snapshot = { files: fixture.files.map(file => ({ path: file.name,
    sha256: createHash("sha256").update(file.bytes).digest("hex") })) };
  verifyLanguageRepository(root, fixture, snapshot);
  const calls = [];
  const toolchains = runLanguageRepository({ run(command, args, options) {
    calls.push({ command, args, cwd: options.cwd });
    if (fixture.toolchains.some(probe => probe[0] === command)) {
      return command === "java" ? { stdout: "", stderr: 'openjdk version "21.0.1"' } :
        { stdout: `${command} 1.2.3\n`, stderr: "" };
    }
    return { stdout: "", stderr: "" };
  } }, root, fixture);
  assert.equal(calls.length, fixture.toolchains.length + 2);
  assert.equal(calls[0].cwd, root);
  assert.deepEqual(toolchains.map(row => row.command), fixture.toolchains.map(row => row[0]));
  assert(toolchains.every(row => /[0-9]/.test(row.version_output)));
  assert.throws(() => runLanguageRepository({ run() { throw new Error("required tool unavailable"); } }, root, fixture),
    /required tool unavailable/);
  assert.throws(() => runLanguageRepository({ run() { return { stdout: "", stderr: "" }; } }, root, fixture),
    /toolchain version is unavailable/);
  assert.throws(() => verifyLanguageRepository(root, fixture, { files: [...snapshot.files,
    { path: "build/generated", sha256: "0".repeat(64) }] }), /generated output/);
  const changed = fixture.files.find(file => file.name === "data/bytes.bin");
  writeFileSync(path.join(root, "data/bytes.bin"), Buffer.from([0]));
  assert.throws(() => verifyLanguageRepository(root, fixture), /bytes changed/);
  writeFileSync(path.join(root, "data/bytes.bin"), changed.bytes);
}

const source = readFileSync(process.argv[2], "utf8");
const implementation = source.slice(source.indexOf("async function executeInstall("),
  source.indexOf("\nasync function executeUpgrade(", source.indexOf("async function executeInstall(")));
async function exercise(channel, version, failedLanguage = null) {
  const events = [], observations = {};
  const context = vm.createContext({ path, process: { env: {} }, languageRepositoryNames,
    verifyCandidateTag() {}, probePackageManager() {}, candidateRoot() { return true; },
    async installChannel() { return { ait: { command: "/installed/ait" }, server: { command: "/installed/ait-server" },
      runner: { command: "/installed/ait-runner" }, uninstall() { events.push("uninstall"); } }; },
    assertInstalledOrigin() {}, exactContextVersion() {}, exerciseBinding() {},
    markBindingChecks() {}, async exerciseLifecycle() {},
    mark(checks, key) { if (!Object.hasOwn(checks, key)) throw new Error(`undeclared ${key}`); checks[key] = true; },
    requiresLanguageCli(value) { return value.startsWith("1.2."); },
    languageRepository: name => languageRepository(name),
    verifyLanguageRepository(root, fixture) { events.push(`verify:${fixture.name}`); },
    runInstalledCoreCases(input) {
      assert.equal(input.aitBin, "/installed/ait");
      assert.equal(input.serverBin, "/installed/ait-server");
      assert.equal(input.runnerBin, "/installed/ait-runner");
      events.push("installed-core");
      return { core_snapshot: "SNP-123456789ABC" };
    },
    firstLand(recorder, ait, root, text, prior, fixture) {
      assert.equal(ait.command, "/installed/ait");
      if (fixture?.name === failedLanguage) throw new Error("required tool unavailable");
      events.push(fixture?.name ?? "first-land");
      return { root, snapshot_id: "SNP-123456789ABC",
        toolchains: fixture?.toolchains.map(([command, ...args]) =>
          ({ command, args, version_output: `${command} 1.2.3` })) };
    },
    requireRegularFile() {}, fail(message) { throw new Error(message); },
  });
  vm.runInContext(implementation + "\nthis.executeInstall = executeInstall;", context);
  const required = ["runner_target", "package_manager", "candidate_install", "installed_origin",
    "command_version", "first_land", "generated_workflow_current", "uninstall", "user_data_retained"];
  if (channel === "github" && version.startsWith("1.2.")) required.push("language_repositories", "installed_core_cases");
  const checks = Object.fromEntries(required.map(key => [key, false]));
  const invocation = context.executeInstall({ config: { release: { version } }, status: {},
    row: { channel, lifecycle: "product", service: null, id: `${channel}-fixture` },
    checks, recorder: { observations }, root: "/clean-host" });
  if (failedLanguage) {
    await assert.rejects(invocation, /required tool unavailable/);
    assert(!events.includes("uninstall"));
  } else {
    await invocation;
    assert(Object.values(checks).every(Boolean));
  }
  return { events, observations };
}
const current = await exercise("github", "1.2.0");
assert.deepEqual(current.events, ["first-land", ...languageRepositoryNames, "installed-core", "uninstall",
  ...languageRepositoryNames.map(name => `verify:${name}`)]);
assert.deepEqual(Array.from(current.observations.language_repositories, row => row.language), languageRepositoryNames);
assert.equal(current.observations.installed_core_cases.core_snapshot, "SNP-123456789ABC");
assert.deepEqual((await exercise("github", "1.1.4")).events, ["first-land", "uninstall"]);
assert.deepEqual((await exercise("npm", "1.2.0")).events, ["first-land", "uninstall"]);
await exercise("github", "1.2.0", "go");
NODE
test "$(grep -c 'CHECKSUM_ASSET_NAME.test(match\[2\])' "${phase_runner}")" = 2
node --input-type=module - "${phase_runner}" "${temporary_root}" <<'NODE'
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import vm from "node:vm";

const source = readFileSync(process.argv[2], "utf8");

// Exercise the actual apt command builder against a repository with newer native versions.
const aptFunction = source.slice(source.indexOf("function aptContext("), source.indexOf("\nfunction ", source.indexOf("function aptContext(") + 1));
const calls = [];
const stop = new Error("command captured");
const context = vm.createContext({
  debianVersion: (version) => version,
  runnerBundleVersion: (version) => version !== "1.1.0",
  localCandidateAsset: (root, name) => root + "/" + name,
  aptAcquireBounds: () => [],
});
vm.runInContext(aptFunction + "\nthis.apt = aptContext;", context);
function aptArgs(version, stage = null, lifecycle = "standalone-runner") {
  assert.throws(() => context.apt({ lifecycle, architecture: "arm64" }, version, {
    run(command, args) { calls.push({ command, args: Array.from(args) }); throw stop; },
  }, false, stage), (error) => error === stop);
  return calls.at(-1).args.slice(4);
}
assert.deepEqual(aptArgs("1.1.1"), ["ait-native=1.1.1", "ait-runner=1.1.1"]);
assert.deepEqual(aptArgs("1.1.0"), ["ait-runner=1.1.0"]);
assert.deepEqual(aptArgs("1.1.3", "/frozen"), ["/frozen/ait-native_1.1.3_arm64.deb", "/frozen/ait-runner_1.1.3_arm64.deb"]);
assert.deepEqual(aptArgs("1.1.1", null, "product"), ["ait-native=1.1.1"]);

// Exercise current and historical installed-package consumers. These fixtures
// test package/entrypoint guards, not the pending real extension or API gates.
const versionSource = source.slice(source.indexOf("function requiresRelease120("), source.indexOf("\nexport function requiresChatRetirement"));
const aliases = source.match(/^export function requires(?:ChatRetirement|LanguageCli)\(version\).+$/gm)
  .map(line => line.replace(/^export /, "")).join("\n");
// Run actual firstLand against the retained native result shapes. The command
// boundary is simulated; the lifecycle, authority and cleanup checks are real.
const firstLandStart = source.indexOf("function firstLand(");
const finishHelperSource = source.slice(source.indexOf("function firstTaskFinish("), firstLandStart);
const firstLandSource = source.slice(firstLandStart, source.indexOf("\nasync function ", firstLandStart));
const baselineId = "SNP-111111111111", landedId = "SNP-222222222222";
const finishResult = (closeout = {}) => ({
  command: "task.finish", contract: "ait-agent-action/v1", task_id: "LT-0001",
  target_line: "main", landed_snapshot_id: landedId,
  local_finish: { task_id: "LT-0001", land_ordinal: 0, status: "succeeded", change_ref: null,
    landed_snapshot_id: landedId, target_line: "main", pre_land_target_snapshot_id: baselineId },
  closeout: { change_status: null, task_status: "completed", plan_status: "synced",
    plan_markdown_status: "in_sync", line_status: "archived", worktree_status: "removed",
    status: "complete", ...closeout },
  team_handoff: { reason: "missing_binding", notification_sent: false },
  next_action: { command: "ait task finish LT-0001 --local" },
});
const cwdPartial = () => finishResult({ status: "partial", line_status: "failed", worktree_status: "failed",
  issues: { bound_worktree_cleanup: { error: "The process cannot access the file. (os error 32)" },
    bound_line_closeout: { reason: "feature_line_closeout_failed" } } });
let finishVectorIndex = 0;
function exerciseFirstLand({ platform = "linux", initial = finishResult(), resumed = finishResult(),
  version = "1.2.0", mutate = () => {}, inventory = rows => rows,
  retainRegistration = false, retainWorktree = false, archiveOnRecovery = true } = {}) {
  const parent = path.join(process.argv[3], `first-land-vector-${finishVectorIndex++}`);
  mkdirSync(parent);
  const root = path.join(parent, "repository"), worktree = path.join(parent, "lt-0001");
  const expectedText = "exact installed project change\n", calls = [], recorder = { observations: {} };
  let applyCount = 0, resumedCount = 0, inventoryCount = 0, manualRemovals = 0;
  let archived = false, registered = true;
  const hash = value => createHash("sha256").update(value).digest("hex");
  const sandbox = vm.createContext({ path, process: { platform }, existsSync, mkdirSync, readFileSync,
    writeFileSync, rmSync(...args) { manualRemovals++; return rmSync(...args); }, sha256Bytes: hash,
    sha256File: file => hash(readFileSync(file)),
    requireRegularFile(file) { assert(statSync(file).isFile()); },
    generatedWorkflowCurrent() {}, fail(message) { throw new Error(message); },
    runSpec(_recorder, _ait, args, options) {
      assert(args.includes("--full")); assert.equal(options.captureLimit, 128 * 1024);
      const result = sandbox.jsonSpec(_recorder, _ait, args, options);
      const full = { ...result, task_land_contract: { version: "task-land-plan-closeout/v1", scope: "local" },
        closeout_status: result.closeout.status, task_status: result.closeout.task_status,
        change_status: result.closeout.change_status, closeout_recovery: result.next_action,
        bound_worktree_cleanup: { status: result.closeout.worktree_status,
          ...result.closeout.issues?.bound_worktree_cleanup },
        bound_line_closeout: { status: result.closeout.line_status,
          ...result.closeout.issues?.bound_line_closeout },
        plan_checklist_closeout: { status: result.closeout.plan_status },
        plan_markdown_materialization: { status: result.closeout.plan_markdown_status } };
      delete full.closeout; delete full.next_action;
      return { status: result.closeout.status === "complete" ? 0 : 2, stdout: JSON.stringify(full), stderr: "" };
    },
    jsonSpec(_recorder, _ait, args, options) {
      calls.push({ args: Array.from(args), label: options.label, cwd: options.cwd });
      if (args[0] === "init") {
        mkdirSync(path.join(root, ".ait")); writeFileSync(path.join(root, ".ait/config.json"), "{}\n");
        writeFileSync(path.join(root, "AGENTS.md"), "current generated guidance\n");
        return {};
      }
      if (args[0] === "task" && args[1] === "start") {
        mkdirSync(worktree); writeFileSync(path.join(worktree, "existing-project.mjs"),
          readFileSync(path.join(root, "existing-project.mjs")));
        return { task_id: "LT-0001", edit_root: worktree, worktree_name: "lt-0001", head_snapshot_id: baselineId };
      }
      if (args[0] === "snapshot") return args[2] === baselineId ? {
        snapshot_id: baselineId, parent_snapshot_id: null, message: "Initialize Task base",
        files: [{ path: "existing-project.mjs", sha256: hash("export const existingProject = true;\n") }],
      } : { snapshot_id: landedId, parent_snapshot_id: baselineId, line_name: "feature/lt-0001" };
      if (args[0] === "task" && args[1] === "finish") {
        const recovery = options.label.includes("resume");
        if (recovery) {
          resumedCount++; assert(!args.includes("--message")); assert.equal(options.cwd, root);
        } else {
          applyCount++; writeFileSync(path.join(root, "first-land.txt"), expectedText);
          const sprint = path.join(root, "docs/sprints/clean_host.md");
          writeFileSync(sprint, readFileSync(sprint, "utf8").replace("- [ ]", "- [x]"));
          if (initial.closeout.status === "complete") {
            rmSync(worktree, { recursive: true }); registered = false;
          } else if (initial.closeout.change_status === "landed") {
            registered = false; // Historical Change closeout released the row first.
          }
        }
        const result = structuredClone(recovery ? resumed : initial);
        const status = result.closeout.status === "complete" ? 0 : 2;
        if (recovery && status === 0) {
          archived = archiveOnRecovery;
          registered = retainRegistration;
          if (!retainWorktree) rmSync(worktree, { recursive: true });
        }
        if (!(options.allowed ?? [0]).includes(status)) throw new Error(`${options.label} failed with ${status}`);
        return result;
      }
      if (args[0] === "task" && args[1] === "show") {
        const result = { task_id: "LT-0001", status: "completed", local_finish: structuredClone(initial.local_finish) };
        mutate(result, args); return result;
      }
      if (args[0] === "change") return { task_id: "LT-0001", status: "landed", landed_snapshot_id: landedId };
      if (args[0] === "worktree") {
        inventoryCount++;
        // Relevant fields from the original E Windows retained/detached row;
        // only its temporary fixture path and identity are rebound here.
        const row = { name: "lt-0001", path: worktree, alias_path: null, bound_task_id: "LT-0001",
          line_name: "feature/lt-0001", current_line: "feature/lt-0001",
          registered_line_name: "feature/lt-0001", workspace_status: "detached",
          head_snapshot_id: null, bound_task_status: null, exists: true,
          auto_created_for_task: true, root_source: "repo_internal_fallback" };
        return inventory(registered ? [row] : [], { recovered: resumedCount > 0, root, worktree });
      }
      if (args[0] === "line") {
        if (args[1] === "archive") archived = true;
        const result = { line_name: args[2], status: args[2] === "main" ? "active" : archived ? "archived" : "active",
          head_snapshot_id: landedId };
        mutate(result, args); return result;
      }
      throw new Error(`unexpected command: ${args}`);
    },
  });
  vm.runInContext(versionSource + "\n" + aliases + "\n" + finishHelperSource + "\n" + firstLandSource + "\nthis.firstLand = firstLand;", sandbox);
  let error;
  try { sandbox.firstLand(recorder, {}, root, expectedText, null, null, version); } catch (failure) { error = failure; }
  return { error, calls, applyCount, resumedCount, inventoryCount, manualRemovals, recorder, worktree };
}
const linuxFinish = exerciseFirstLand();
assert.equal(linuxFinish.error, undefined); assert.equal(linuxFinish.applyCount, 1);
const windowsFinish = exerciseFirstLand({ platform: "win32", initial: cwdPartial() });
assert.equal(windowsFinish.error, undefined); assert.equal(windowsFinish.applyCount, 1);
assert.equal(windowsFinish.resumedCount, 1); assert(!existsSync(windowsFinish.worktree));
assert.equal(windowsFinish.inventoryCount, 2); assert.equal(windowsFinish.manualRemovals, 0);
assert(windowsFinish.calls.findIndex(call => call.label === "candidate recovered Windows worktree inventory") >
  windowsFinish.calls.findIndex(call => call.label === "candidate task finish closeout resume"));
assert(!windowsFinish.calls.some(call => call.args[0] === "change" || call.args[1] === "archive"));
assert.equal(windowsFinish.recorder.observations.windows_partial_task_land_closeout.change_landed, false);
assert.equal(windowsFinish.recorder.observations.windows_partial_task_land_closeout.worktree_registration_absent, true);
for (const change of [
  row => { row.name = "lt-0002"; },
  row => { row.bound_task_id = "LT-0002"; },
  row => { row.path += "-foreign"; },
  row => { row.alias_path = row.path; },
  row => { row.line_name = "feature/lt-0002"; },
  row => { row.workspace_status = "dirty"; },
]) {
  const result = exerciseFirstLand({ platform: "win32", initial: cwdPartial(),
    inventory(rows, { recovered }) { if (!recovered) change(rows[0]); return rows; } });
  assert.match(result.error.message, /readback is inconsistent/);
  assert.equal(result.resumedCount, 0); assert.equal(result.manualRemovals, 0);
}
for (const inventory of [() => [], rows => [...rows, ...rows]]) {
  const result = exerciseFirstLand({ platform: "win32", initial: cwdPartial(), inventory });
  assert.match(result.error.message, /readback is inconsistent/); assert.equal(result.resumedCount, 0);
}
for (const options of [{ retainRegistration: true }, { retainWorktree: true }]) {
  const result = exerciseFirstLand({ platform: "win32", initial: cwdPartial(), ...options });
  assert.match(result.error.message, /left its worktree or registration behind/);
  assert.equal(result.manualRemovals, 0);
  assert.equal(result.recorder.observations.windows_partial_task_land_closeout, undefined);
}
const unarchived = exerciseFirstLand({ platform: "win32", initial: cwdPartial(), archiveOnRecovery: false });
assert.match(unarchived.error.message, /feature Line did not archive exactly/);
assert.equal(unarchived.recorder.observations.windows_partial_task_land_closeout, undefined);
const nextReceipt = finishResult(); nextReceipt.local_finish.land_ordinal = 1;
assert.match(exerciseFirstLand({ platform: "win32", initial: cwdPartial(), resumed: nextReceipt }).error.message,
  /changed its apply receipt/);
const legacyFinish = exerciseFirstLand({ platform: "win32", version: "1.1.4",
  initial: finishResult({ change_status: "landed", status: "partial", line_status: "failed", worktree_status: "failed" }) });
assert.equal(legacyFinish.error, undefined); assert.equal(legacyFinish.resumedCount, 0);
assert(legacyFinish.calls.some(call => call.args[0] === "change"));
const unknownPartial = finishResult({ status: "partial" });
unknownPartial.main_seed_sync = { status: "failed", error: "retained native seed diagnostic" };
assert.match(exerciseFirstLand({ platform: "darwin", initial: unknownPartial, resumed: unknownPartial }).error.message,
  /closeout resume failed with 2: .*retained native seed diagnostic/);
assert.match(exerciseFirstLand({ platform: "win32", initial: cwdPartial(), resumed: unknownPartial }).error.message,
  /closeout resume failed with 2/);
for (const field of ["local_line_sync", "repo_root_restore"]) {
  const partial = finishResult({ status: "partial" });
  partial[field] = { status: "failed", error: `retained ${field} diagnostic` };
  const result = exerciseFirstLand({ platform: "win32", initial: cwdPartial(), resumed: partial });
  assert.match(result.error.message, new RegExp(`closeout resume failed with 2: .*retained ${field} diagnostic`));
  assert.equal(result.manualRemovals, 0); assert(existsSync(result.worktree));
}
assert.match(exerciseFirstLand({ initial: finishResult({ change_status: "landed" }) }).error.message, /reported a Change/);
assert.match(exerciseFirstLand({ initial: { ...finishResult(), task_id: "LT-0002" } }).error.message,
  /changed Task authority/);
const foreignPartial = cwdPartial(); foreignPartial.closeout.issues.bound_worktree_cleanup.error = "Access denied. (os error 5)";
assert.match(exerciseFirstLand({ platform: "win32", initial: foreignPartial }).error.message, /authoritative closeout state/);
assert.match(exerciseFirstLand({ platform: "win32", initial: cwdPartial(),
  resumed: { ...finishResult(), landed_snapshot_id: "SNP-333333333333" } }).error.message, /changed authority/);
assert.match(exerciseFirstLand({ platform: "darwin", initial: unknownPartial,
  resumed: { ...finishResult(), landed_snapshot_id: "SNP-333333333333" } }).error.message, /changed authority/);
assert.match(exerciseFirstLand({ platform: "darwin", initial: unknownPartial,
  resumed: { ...finishResult(), task_id: "LT-0002" } }).error.message, /changed authority/);
assert.match(exerciseFirstLand({ platform: "darwin", initial: unknownPartial,
  resumed: { ...finishResult(), target_line: "other" } }).error.message, /changed authority/);
assert.match(exerciseFirstLand({ platform: "win32", initial: cwdPartial(),
  resumed: finishResult({ plan_status: "failed" }) }).error.message, /Task and Plan closeout/);
assert.match(exerciseFirstLand({ platform: "win32", initial: cwdPartial(),
  mutate(row, args) { if (args[2] === "main") row.head_snapshot_id = baselineId; } }).error.message, /readback is inconsistent/);
console.log("Actual firstLand current/legacy authority and partial-closeout vectors passed");
const nativeDiagnostic = JSON.stringify({ main_seed_sync: { status: "failed", error: "original seed error" },
  existing_native_details: "x".repeat(20_000) });
const recorderStart = source.indexOf("class Recorder {");
const recorderSource = source.slice(recorderStart, source.indexOf("\nfunction commandExists", recorderStart));
const commandTextSource = source.slice(source.indexOf("function tail("), recorderStart);
const capture = vm.createContext({ MAX_CAPTURE: 64 * 1024 * 1024, COMMAND_TIMEOUT_MS: 30_000,
  process: { env: {} }, sha256Bytes: value => createHash("sha256").update(value).digest("hex"),
  spawnSync() { return { status: 2, stdout: nativeDiagnostic, stderr: "" }; },
  fail(message) { throw new Error(message); } });
vm.runInContext(commandTextSource + "\n" + recorderSource + "\nthis.Recorder = Recorder;", capture);
const captured = new capture.Recorder();
captured.run("/installed/ait", ["task", "finish", "LT-0001", "--json", "--full"],
  { allowed: [0, 2], captureLimit: 128 * 1024 });
assert.equal(captured.commands[0].stdout_tail, nativeDiagnostic);
assert.equal(captured.commands[0].stdout_sha256, createHash("sha256").update(nativeDiagnostic).digest("hex"));
const pythonSource = source.slice(source.indexOf("function pythonContext("), source.indexOf("\nfunction npmPackageRoot("));
const pyRoot = path.join(process.argv[3], "python-language-cli"), pySite = path.join(pyRoot, "site");
const pyScripts = path.join(pyRoot, "bin"), pyModule = path.join(pySite, "ait_python");
const pyMetadata = path.join(pySite, "ait_native-1.2.0.dist-info");
for (const dir of [pyScripts, pyModule, pyMetadata]) mkdirSync(dir, { recursive: true });
writeFileSync(path.join(pyModule, "__init__.py"), "");
writeFileSync(path.join(pyModule, "cli.py"), "def main(): pass\n");
writeFileSync(path.join(pyScripts, "ait"), "#!/usr/bin/env python3\n");
writeFileSync(path.join(pyMetadata, "METADATA"), "Metadata-Version: 2.1\nName: ait-native\nVersion: 1.2.0\n");
const entrypoint = path.join(pyMetadata, "entry_points.txt"), record = path.join(pyMetadata, "RECORD");
const ownedFiles = "ait_python/__init__.py,,\nait_python/cli.py,,\n../bin/ait,,\n";
writeFileSync(entrypoint, "[console_scripts]\nait = ait_python.cli:main\n"); writeFileSync(record, ownedFiles);
const pythonCalls = [];
const pythonVm = vm.createContext({ path, existsSync, process: { platform: "linux" }, CHAT_RETIREMENT_MESSAGE: "retired",
  RETIRED_CHAT_COMMANDS: ["ait-agent", "ait-agent-worker"],
  venvPaths: () => ({ python: "python3", scripts: pyScripts }), pipInstall() {},
  requireRegularFile(file) { if (!existsSync(file) || !statSync(file).isFile()) throw new Error("missing installed file"); },
  commandSpec: file => ({ command: file }), fail(message) { throw new Error(message); }, validateRetiredBindingMetadata() {},
});
vm.runInContext(versionSource + "\n" + aliases + "\n" + pythonSource + "\nthis.pythonContext = pythonContext;", pythonVm);
function pythonPackage(version) {
  return pythonVm.pythonContext({ run(command, args, options) {
    pythonCalls.push(options.label);
    if (options.label === "PyPI language CLI entrypoint") {
      const result = spawnSync(command, Array.from(args), { encoding: "utf8",
        env: { ...process.env, PYTHONPATH: pySite, PYTHONNOUSERSITE: "1" } });
      assert.equal(result.status, 0, result.stderr); return result;
    }
    return { stdout: JSON.stringify({ runtime_authority: "rust", python_binding: "pyo3", process_transport_allowed: false, version }) };
  } }, pyRoot, version);
}
const pythonCurrent = pythonPackage("1.2.0"); assert.equal(pythonCurrent.server, undefined); pythonCurrent.binding();
assert(pythonCalls.includes("PyPI language CLI entrypoint"));
for (const invalid of ["missing-entrypoint", "wrong-module", "unowned-module", "unowned-launcher"]) {
  writeFileSync(entrypoint, invalid === "missing-entrypoint" ? "[console_scripts]\n" :
    "[console_scripts]\nait = " + (invalid === "wrong-module" ? "outside_package" : "ait_python.cli") + ":main\n");
  writeFileSync(record, ownedFiles.replace(invalid === "unowned-module" ? "ait_python/cli.py,,\n" :
    invalid === "unowned-launcher" ? "../bin/ait,,\n" : "not-present", ""));
  assert.throws(() => pythonPackage("1.2.0").binding(), invalid);
}
writeFileSync(entrypoint, "[console_scripts]\nait = ait_python.cli:main\n"); writeFileSync(record, ownedFiles);
writeFileSync(path.join(pyScripts, "ait-server"), "historical native bytes");
assert.throws(() => pythonPackage("1.2.0"), /standalone native command/);
assert(pythonPackage("1.1.4").server, "historical PyPI still requires its original native Server");
rmSync(path.join(pyScripts, "ait-server"));
assert.throws(() => pythonPackage("1.1.4"), /missing installed file/);

const nodeSource = source.slice(source.indexOf("function nodeContext("), source.indexOf("\nfunction brewEnvironment("));
const npmRoot = path.join(process.argv[3], "node-language-cli"), npmPackage = path.join(npmRoot, "package");
const npmPlatform = path.join(npmRoot, "platform");
const npmEntry = path.join(npmPackage, "bin/ait.mjs"), npmMetadata = path.join(npmPackage, "package.json");
const npmShim = path.join(npmRoot, "bin/ait");
mkdirSync(path.dirname(npmEntry), { recursive: true }); mkdirSync(path.join(npmPackage, "src")); mkdirSync(path.dirname(npmShim), { recursive: true });
mkdirSync(npmPlatform); writeFileSync(path.join(npmPlatform, "ait.node"), "loadable binding fixture");
writeFileSync(npmEntry, "// JavaScript frontend fixture\n"); writeFileSync(path.join(npmPackage, "src/index.js"), "");
writeFileSync(npmMetadata, JSON.stringify({ bin: { ait: "bin/ait.mjs" } })); symlinkSync(npmEntry, npmShim);
const nodeVm = vm.createContext({ path, process: { platform: "linux" }, existsSync, lstatSync, readdirSync, realpathSync, readFileSync, JSON,
  CHAT_RETIREMENT_MESSAGE: "retired", npmInstall() {},
  npmPackageRoot: (prefix, name) => prefix === npmRoot ? name === "@wa120/ait-native" ? npmPackage : npmPlatform : path.join(prefix, "lib/node_modules", name),
  npmTargetSuffix: () => "linux-x64-gnu",
  requireRegularFile(file) { if (!existsSync(file) || !statSync(file).isFile()) throw new Error("missing installed file"); },
  requireCommand: name => name, commandSpec: (command, argsPrefix = []) => ({ command, argsPrefix }),
  fail(message) { throw new Error(message); }, validateRetiredBindingMetadata() {},
});
vm.runInContext(versionSource + "\n" + aliases + "\n" + nodeSource + "\nthis.nodeContext = nodeContext;", nodeVm);
function nodePackage(version, prefix = npmRoot) {
  const calls = [];
  const installed = nodeVm.nodeContext({ run(command, args, options) {
    calls.push({ args: Array.from(args), label: options.label });
    return { stdout: options.label === "npm direct Node-API addon" ? JSON.stringify({ runtime_authority: "rust",
      node_binding: "napi", process_transport_allowed: false, version }) : `ait ${version}\n` };
  } }, prefix, version, {});
  installed.binding(); return calls;
}
const nodeCurrent = nodePackage("1.2.0");
assert.deepEqual(nodeCurrent.at(-1).args, [npmEntry, "--version"]);
assert.equal(nodeCurrent.some(call => call.args.some(arg => arg.includes(".runCli("))), false);
assert(nodePackage("1.1.4").some(call => call.args.some(arg => arg.includes(".runCli("))), "historical probe remains unchanged");
for (const installedRoot of [npmPackage, npmPlatform]) {
  const nested = path.join(installedRoot, "native", "bin"); mkdirSync(nested, { recursive: true });
  for (const command of ["ait", "ait-cli.exe", "ait-server", "ait-runner.exe", "ait-agent", "AIT-AGENT-WORKER.EXE"]) {
    const payload = path.join(nested, command); writeFileSync(payload, "standalone command fixture");
    assert.throws(() => nodePackage("1.2.0"), /standalone command payload/);
    nodePackage("1.1.4"); rmSync(payload);
  }
  const dangling = path.join(nested, "ait.exe"); symlinkSync(path.join(npmRoot, "absent"), dangling);
  assert.throws(() => nodePackage("1.2.0"), /standalone command payload/); rmSync(dangling);
}
rmSync(npmPlatform, { recursive: true });
assert.throws(() => nodePackage("1.2.0"), /selected language package/);
const npmNestedPlatform = path.join(npmPackage, "node_modules/@wa120/ait-native-linux-x64-gnu");
mkdirSync(npmNestedPlatform, { recursive: true });
writeFileSync(path.join(npmNestedPlatform, "ait.node"), "loadable binding fixture");
nodePackage("1.2.0");
writeFileSync(path.join(npmNestedPlatform, "ait.exe"), "standalone command fixture");
assert.throws(() => nodePackage("1.2.0"), /standalone command payload/);
rmSync(path.join(npmPackage, "node_modules"), { recursive: true });
mkdirSync(npmPlatform); writeFileSync(path.join(npmPlatform, "ait.node"), "loadable binding fixture");
nodePackage("1.2.0");
vm.runInContext('process.platform = "win32";', nodeVm);
writeFileSync(path.join(npmRoot, "ait.cmd"), "node /node_modules/@wa120/ait-native/bin/ait.mjs\n");
writeFileSync(path.join(npmRoot, "ait.exe"), "package-manager shim fixture");
nodePackage("1.2.0");
vm.runInContext('process.platform = "linux";', nodeVm);

// Exercise npm's real global-install dependency layout, rather than teaching
// the guard only a synthetic sibling layout. All packages/cache/prefixes stay
// under this test's disposable directory and installation is strictly offline.
const installRoot = path.join(process.argv[3], "actual-npm-install");
const platformSource = path.join(installRoot, "platform"), wrapperSource = path.join(installRoot, "wrapper");
const packedRoot = path.join(installRoot, "packed"), npmCache = path.join(installRoot, "cache");
for (const directory of [platformSource, wrapperSource, packedRoot]) mkdirSync(directory, { recursive: true });
function fixtureNpm(args, cwd = installRoot) {
  const result = spawnSync("npm", [...args, "--cache", npmCache], { cwd, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr); return result.stdout;
}
const platformName = "@wa120/ait-native-linux-x64-gnu";
writeFileSync(path.join(platformSource, "package.json"), JSON.stringify({ name: platformName, version: "1.2.0", files: ["ait.node"] }));
writeFileSync(path.join(platformSource, "ait.node"), "loadable binding fixture");
const platformTar = path.join(packedRoot, JSON.parse(fixtureNpm(["pack", "--ignore-scripts", "--json", "--pack-destination", packedRoot], platformSource))[0].filename);
mkdirSync(path.join(wrapperSource, "bin")); mkdirSync(path.join(wrapperSource, "src"));
writeFileSync(path.join(wrapperSource, "bin/ait.mjs"), "#!/usr/bin/env node\n");
writeFileSync(path.join(wrapperSource, "src/index.js"), "");
writeFileSync(path.join(wrapperSource, "package.json"), JSON.stringify({ name: "@wa120/ait-native", version: "1.2.0", files: ["bin", "src"],
  bin: { ait: "bin/ait.mjs" }, optionalDependencies: { [platformName]: "file:" + platformTar } }));
const wrapperTar = path.join(packedRoot, JSON.parse(fixtureNpm(["pack", "--ignore-scripts", "--json", "--pack-destination", packedRoot], wrapperSource))[0].filename);
for (const sibling of [false, true]) {
  const prefix = path.join(installRoot, sibling ? "sibling-prefix" : "wrapper-prefix");
  fixtureNpm(["install", "--global", "--ignore-scripts", "--offline", "--no-audit", "--no-fund", "--prefix", prefix,
    ...(sibling ? [platformTar] : []), wrapperTar]);
  const modules = path.join(prefix, "lib/node_modules");
  const globalPlatform = path.join(modules, platformName);
  const nestedPlatform = path.join(modules, "@wa120/ait-native/node_modules", platformName);
  if (sibling) assert(existsSync(globalPlatform), "the explicitly installed platform must be present");
  assert(existsSync(globalPlatform) || existsSync(nestedPlatform), "npm must install the selected platform dependency");
  nodePackage("1.2.0", prefix);
  for (const installed of [nestedPlatform, globalPlatform].filter(directory => existsSync(directory))) {
    const command = path.join(installed, "ait.exe"); writeFileSync(command, "standalone command fixture");
    assert.throws(() => nodePackage("1.2.0", prefix), /standalone command payload/); rmSync(command);
  }
}
writeFileSync(npmMetadata, "{}"); assert.throws(() => nodePackage("1.2.0"), /declared ait JavaScript entrypoint/);
writeFileSync(npmMetadata, JSON.stringify({ bin: { ait: "bin/ait.mjs" } }));
rmSync(npmShim); assert.throws(() => nodePackage("1.2.0"), /shim does not resolve/);
symlinkSync(path.join(npmPackage, "src/index.js"), npmShim); assert.throws(() => nodePackage("1.2.0"), /shim does not resolve/);
console.log("Language CLI installed-package guards and historical boundary: pass");

const declaration = source.match(/^const CHECKSUM_ASSET_NAME = \/(.*)\/;$/m);
assert.ok(declaration, "clean-host checksum asset-name rule is missing");
assert.ok(
  source.includes(`const OCI_SERVER_RUNTIME_ARGS = Object.freeze([
  "--listen",
  "0.0.0.0:8088",
  "--init-if-missing",
  "--defer-ci-admission",
]);`),
  "OCI server lifecycle must retain the exact documented runtime arguments",
);
assert.ok(
  source.includes(`          reference,
          ...OCI_SERVER_RUNTIME_ARGS,`),
  "OCI server lifecycle must pass the documented arguments after the image reference",
);
assert.ok(
  source.includes('const stderr = result.stderr || result.error?.message || "";'),
  "clean-host command failures must retain spawn and timeout diagnostics",
);
assert.ok(
  source.includes('landed.next_action?.command !== `ait task finish ${taskId} --local`'),
  "partial Task closeout must bind its Task-only recovery command",
);
assert.equal(
  source.includes('label: "candidate completed Windows worktree removal"'),
  false,
  "partial closeout must not require an already-removed worktree registration",
);
assert.ok(
  source.includes('["task", "finish", taskId, "--local", "--json"]'),
  "partial Task closeout must resume with the Task ID",
);
assert.ok(
  source.includes("agents_sha256: sha256File(agentsPath)"),
  "upgrade baseline must capture the prior repository workflow guidance",
);
assert.ok(
  source.includes("agentsSha256 !== priorState.agents_sha256"),
  "upgrade verification must compare the candidate repository guidance with its prior digest",
);
assert.ok(
  source.includes("candidate upgrade replaced the prior repository workflow guidance"),
  "upgrade verification must fail closed when candidate installation rewrites prior guidance",
);
assert.ok(
  source.includes("prior_workflow_guidance_preserved"),
  "upgrade evidence must record byte-for-byte prior workflow guidance preservation",
);
assert.ok(
  source.includes("minor === 1 && patch > 0"),
  "the exact published 1.1.0 baseline must remain pre-runner-bundle while 1.1.1+ uses the bundle",
);
assert.ok(
  source.includes('!["1.0.0-rc.6", "1.0.0-rc.10"].includes(priorVersion)'),
  "only the exact affected legacy versions may admit the Windows init regression",
);
assert.ok(
  source.includes('expected_regression: "legacy_windows_read_only_fsync"'),
  "bounded legacy Windows init evidence must identify the admitted regression",
);
assert.ok(
  source.includes("for (const component of githubNativeComponents)"),
  "GitHub installation must consume its shaped native component inventory",
);
assert.ok(
  /row\.channel === "github"[\s\S]*?version === "1\.0\.0-rc\.6"[\s\S]*?components = \[\s*"ait",\s*"ait-agent",\s*"ait-server",\s*"ait-runner",\s*"ait-python",\s*"ait-node",\s*\]/.test(source),
  "the exact rc.6 GitHub prior row must match its signed historical family manifest",
);
assert.ok(
  /if \(priorState\?\.available === true\) \{[\s\S]*?\} else \{\s*generatedWorkflowCurrent\(agents\);\s*\}/.test(source),
  "new installs must validate current generated guidance while upgrades preserve prior guidance",
);
for (const requiredWindowsCloseout of [
  '["task", "show", taskId, "--local", "--json"]',
  '["change", "show", taskId, "--local", "--json"]',
  '["line", "show", landed.target_line, "--json"]',
  '["worktree", "list", "--json"]',
  '["line", "archive", featureLineName, "--json"]',
  "windows_partial_task_land_closeout",
  "second_land_applied: false",
  "path.basename(worktree) !== started.worktree_name",
]) {
  assert.ok(
    source.includes(requiredWindowsCloseout),
    `Windows direct partial-closeout verification is missing: ${requiredWindowsCloseout}`,
  );
}
assert.equal(
  source.includes("landed.change_ref"),
  false,
  "Windows partial closeout must not require an internal Change reference from Task finish",
);
const rule = new RegExp(declaration[1]);
for (const name of [
  "ait-native_1.0.0~rc.11_amd64.deb",
  "ait-native-1.0.0-rc.11-x86_64.tar.gz",
  "wa120@ait+native_1.0.0.tgz",
]) {
  assert.equal(rule.test(name), true, `expected safe checksum asset name: ${name}`);
}
for (const name of [
  "",
  "/ait-native.deb",
  "../ait-native.deb",
  "assets/ait-native.deb",
  "assets\\ait-native.deb",
  "ait native.deb",
]) {
  assert.equal(rule.test(name), false, `expected unsafe checksum asset name: ${name}`);
}
NODE
node --input-type=module - "${workflow}" <<'MAC_SEED_OBSERVATION_TEST'
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import vm from 'node:vm';
import path from 'node:path';
const source = readFileSync(process.argv[2], 'utf8');
const install = source.slice(source.indexOf('  install:'), source.indexOf('  upgrade:'));
assert(!source.includes('  mac_seed_observation:'));
assert(!source.includes('steps.seed_observation.outputs'));
assert(!source.includes('needs.aggregate.outputs.mac_seed'));
const observe = install.indexOf('- name: Observe the original Mac seed closeout failure');
assert(observe > install.indexOf('--output "${PWD}/evidence/install.json"'));
assert(observe < install.indexOf('- name: Upload install-phase evidence'));
assert(install.includes("steps.phase.outcome == 'failure' && matrix.channel == 'github' && matrix.lifecycle == 'product' && matrix.target == 'aarch64-apple-darwin'"));
assert(install.slice(observe, install.indexOf('        run: |', observe)).includes('continue-on-error: true'));
assert(install.includes("test '${{ steps.phase.outcome }}' = success"));
assert(install.includes("test '${{ steps.upload.outcome }}' = success"));
// GitHub limits each decoded run value to 21,000 UTF-16 characters. Mixed
// text/expressions also become one escaped format expression in TemplateReader.
function observationStep(name) {
  const begin = install.indexOf('      - name: ' + name + '\n');
  assert(begin >= 0);
  const next = install.indexOf('\n      - name:', begin + 1);
  const step = install.slice(begin, next < 0 ? undefined : next);
  const marker = '        run: |\n', offset = step.indexOf(marker);
  assert(offset >= 0);
  const lines = [];
  for (const line of step.slice(offset + marker.length).split('\n')) {
    if (line === '') lines.push('');
    else if (line.startsWith('          ')) lines.push(line.slice(10));
    else break;
  }
  return { header: step.slice(0, offset), run: lines.join('\n').replace(/\n*$/, '\n') };
}
function requireLiteralObservationRun(body) {
  assert(body.length <= 21000, 'observation run exceeds 21000 characters');
  assert(!body.includes('${{'), 'observation run must remain literal');
}
const preparation = observationStep('Prepare the original Mac seed observer source');
const diagnostic = observationStep('Observe the original Mac seed closeout failure');
const prepareIndex = install.indexOf('- name: Prepare the original Mac seed observer source');
assert(prepareIndex > install.indexOf('--output "${PWD}/evidence/install.json"') && prepareIndex < observe);
for (const step of [preparation, diagnostic]) {
  requireLiteralObservationRun(step.run);
  assert(step.header.includes("steps.phase.outcome == 'failure' && matrix.channel == 'github' && matrix.lifecycle == 'product' && matrix.target == 'aarch64-apple-darwin'"));
  assert(step.header.includes('continue-on-error: true'));
  assert(step.run.includes('diagnostic="${RUNNER_TEMP}/mac-seed-observation"'));
}
assert(preparation.header.includes('id: seed_observer_source'));
assert(diagnostic.header.includes("steps.seed_observer_source.outcome == 'success'"));
assert(diagnostic.header.includes('AIT_SEED_OBSERVATION_CONFIG_SHA: ${{ inputs.candidate_authority_sha256 }}'));
assert(diagnostic.header.includes('AIT_SEED_OBSERVATION_STATUS_SHA: ${{ needs.stage.outputs.candidate_status_sha256 }}'));
assert(!diagnostic.header.includes('steps.python.outputs'));
assert(install.includes("id: python\n        if: matrix.channel == 'pypi'"));
requireLiteralObservationRun('x'.repeat(21000));
assert.throws(() => requireLiteralObservationRun('x'.repeat(21001)), /21000/);
assert.throws(() => requireLiteralObservationRun(preparation.run + diagnostic.run), /21000/);
assert.throws(() => requireLiteralObservationRun('echo "${{ inputs.candidate_authority_sha256 }}"'), /literal/);
// Two real shells share only RUNNER_TEMP. A controlled verifier records the
// actual argv produced by the unchanged stage invocation and SHA env bindings.
const flowRoot = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'ait-observer-env-flow-')));
try {
  mkdirSync(path.join(flowRoot, 'control/ci'), { recursive: true });
  writeFileSync(path.join(flowRoot, 'control/ci/release_prepublish_verify.mjs'),
    'process.stdout.write(JSON.stringify(process.argv.slice(2)));\n');
  const env = { ...process.env, RUNNER_TEMP: flowRoot,
    AIT_SEED_OBSERVATION_CONFIG_SHA: 'a'.repeat(64), AIT_SEED_OBSERVATION_STATUS_SHA: 'b'.repeat(64) };
  const run = (body, environment = env) => spawnSync('bash', ['--noprofile', '--norc', '-c', body],
    { cwd: flowRoot, env: environment, encoding: 'utf8' });
  const prepared = run(preparation.run);
  assert.equal(prepared.status, 0, prepared.stderr);
  const cSource = preparation.run.slice(preparation.run.indexOf('#include <stdio.h>'), preparation.run.lastIndexOf('\nC\n') + 1);
  assert.equal(readFileSync(path.join(flowRoot, 'mac-seed-observation/rename-observer.c'), 'utf8'), cSource);
  const prefixEnd = diagnostic.run.indexOf('node --input-type=module');
  assert(prefixEnd > 0);
  const prefix = diagnostic.run.slice(0, prefixEnd);
  const verified = run(prefix);
  assert.equal(verified.status, 0, verified.stderr);
  assert.deepEqual(JSON.parse(readFileSync(path.join(flowRoot, 'mac-seed-observation/candidate-verify.log'), 'utf8')),
    ['stage', '--root', path.join(flowRoot, 'candidate'), '--config-sha256', env.AIT_SEED_OBSERVATION_CONFIG_SHA,
      '--status-sha256', env.AIT_SEED_OBSERVATION_STATUS_SHA]);
  const missing = { ...env }; delete missing.AIT_SEED_OBSERVATION_STATUS_SHA;
  assert.notEqual(run(prefix, missing).status, 0, 'missing hash binding must fail closed');
  const failedPreparation = run(preparation.run);
  assert.notEqual(failedPreparation.status, 0, 'preparation failure must not be hidden');
} finally { rmSync(flowRoot, { recursive: true, force: true }); }
console.log('Observation run boundaries/literal bodies, separate-shell preparation and exact SHA environment flow passed');
const marker = "<<'NODE_SEED_OBSERVATION'\n";
const js = source.slice(source.indexOf(marker) + marker.length, source.indexOf('\n          NODE_SEED_OBSERVATION'))
  .split('\n').map(line => line.slice(10)).join('\n');
const pure = js.slice(js.indexOf('const digest ='), js.indexOf('// Runtime observation:'));
const sandbox = vm.createContext({ assert, createHash, path });
vm.runInContext(pure + '\nthis.api = { selectOriginal, validateCloseout, validateObservation, commandText };', sandbox);
const { selectOriginal, validateCloseout, validateObservation, commandText } = sandbox.api;
const copy = value => JSON.parse(JSON.stringify(value));
const repository = '/runner/temp/ait-clean-host-install-Ab1234/candidate-repository';
const cli = path.join(path.dirname(repository), 'github-bin/ait');
const task = 'LT-0001', snapshot = 'SNP-0123456789AB', target = 'main';
const worktree = '/Volumes/AIT_RAM/.ait-repos/0123456789ab/candidate-repository/lt-0001';
const seed = path.join(path.dirname(worktree), '.ait-internal/main-seed');
const receipt = { status: 'succeeded', task_id: task, landed_snapshot_id: snapshot,
  target_line: target, change_ref: null, pre_land_target_snapshot_id: 'SNP-ABCDEF012345' };
function closeout(recovery) {
  return { task_id: task, task: { task_id: task, status: 'completed', local_finish: receipt }, task_status: 'completed',
    apply_status: 'done', current_line: target, line_name: target, target_line: target, landed_snapshot_id: snapshot,
    change: null, change_id: null, change_ref: null, change_status: null, local_finish: receipt,
    execution_status: recovery ? 'already_landed' : 'applied', auto_snapshot: recovery ? null : {
      snapshot_id: snapshot, newly_created: true, files: [{ path: 'first-land.txt', sha256: 'a'.repeat(64) }] },
    automatic_reconciliation: { status: 'completed', mutated: false, receipt_store_mutated: false,
      actions: [], findings: [], summary: { remaining_count: 0 } },
    workspace_action: 'restored', repo_root_restore: { status: 'restored', target_snapshot_id: snapshot,
      workspace_root: repository, dirty_outside_paths: [], plan: { write_count: recovery ? 0 : 1,
        remove_count: 0, unchanged_count: recovery ? 1 : 0 } },
    bound_worktree_cleanup: recovery ? { task_id: task, status: 'skipped', reason: 'no_bound_worktree' } : {
      task_id: task, status: 'removed', worktree: { removed: true, deleted_path: true, workspace_status: 'clean', path: worktree } },
    bound_line_closeout: { status: 'archived', task_id: task, task_status: 'completed', target_line: target,
      expected_revision_snapshot_id: snapshot, local: { status: 'archived', line: { head_snapshot_id: snapshot, status: 'archived' } } },
    plan_checklist_closeout: { status: 'synced', sync: { status: 'ok' }, updated: !recovery },
    plan_markdown_materialization: { status: 'in_sync', drift: [], skipped: [] },
    main_seed_sync: { path: seed, name: 'main-seed', line_name: target, root_source: 'macos_ram_volume',
      seed_snapshot_id: snapshot, baseline_seed_snapshot_id: null, refresh_strategy: 'full_snapshot_materialization',
      fallback_used: true, fallback_reason: 'task_worktree_promotion_unavailable', copy_error: null, copy_strategy: null,
      materialized_write_count: 3, status: 'failed',
      error: `Failed to install staged main seed ${path.dirname(seed)}/.main-seed.tmp-1234-20261030T001313Z at ${seed}: Permission denied (os error 13)` },
    closeout_status: 'partial', closeout_recovery: { idempotent: true, command: `ait task finish ${task} --local` } };
}
function command(label, args, stdout = {}, status = 0, cwd = repository) {
  const text = JSON.stringify(stdout);
  return { label, command: commandText(cli, args), cwd, exit_status: status, stdout_tail: text,
    stdout_sha256: createHash('sha256').update(text).digest('hex'), stderr_tail: '' };
}
const row = { channel: 'github', lifecycle: 'product', target: 'aarch64-apple-darwin', os: 'macos', architecture: 'arm64' };
function phase() {
  return { contract: 'ait.release.clean-host.phase/v1', phase: 'install', status: 'fail', row,
    error: { category: 'lifecycle_failure' }, commands: [command('candidate ait init', ['init', '--json']),
      command('candidate task start', ['task', 'start', '--from', 'docs/sprints/clean_host.md#clean-host/first-land',
        '--intent', 'Prove exact clean-host first land', '--json'], { task_id: task, edit_root_source: 'managed',
        edit_root: worktree, head_snapshot_id: receipt.pre_land_target_snapshot_id }),
      command('candidate task finish', ['task', 'finish', task, '--message', 'Clean-host first Snapshot', '--local', '--json', '--full'], closeout(false), 2, worktree),
      command('candidate integrated Snapshot show', ['snapshot', 'show', snapshot, '--json'], {
        snapshot_id: snapshot, parent_snapshot_id: receipt.pre_land_target_snapshot_id, files: closeout(false).auto_snapshot.files }),
      command('candidate task finish closeout resume', ['task', 'finish', task, '--local', '--json', '--full'], closeout(true), 2)] };
}
// VM realm arrays must share the guard's realm for strict deep comparisons.
const inside = value => vm.runInContext('(' + JSON.stringify(value) + ')', sandbox);
const selected = selectOriginal(inside(phase()), inside(row));
assert.equal(selected.task, task);
for (const mutate of [p => { p.status = 'pass'; }, p => { p.commands.at(-1).exit_status = 0; },
  p => { p.commands[2].exit_status = 0; }, p => { p.commands[2].cwd = repository; },
  p => { p.commands.at(-1).command += ' --message changed'; },
  p => { p.commands.at(-1).stdout_tail = '{truncated'; },
  p => { p.commands[0].cwd = '/other/candidate-repository'; }]) {
  const changed = phase(); mutate(changed); assert.throws(() => selectOriginal(inside(changed), inside(row)));
}
for (const mutate of [v => { v.task_id = 'LT-0002'; }, v => { v.target_line = 'other'; },
  v => { v.auto_snapshot = {}; }, v => { v.local_finish = { ...v.local_finish, landed_snapshot_id: 'SNP-AAAAAAAAAAAA' }; },
  v => { v.automatic_reconciliation.mutated = true; }, v => { v.automatic_reconciliation.actions = [{}]; },
  v => { v.repo_root_restore.status = 'failed'; }, v => { v.repo_root_restore.plan.write_count = 1; },
  v => { v.bound_worktree_cleanup.reason = 'permission_failure'; }, v => { v.bound_line_closeout.status = 'failed'; },
  v => { v.plan_checklist_closeout.status = 'failed'; }, v => { v.plan_markdown_materialization.drift = [{}]; },
  v => { v.main_seed_sync.root_source = 'repo_local_fallback'; }, v => { v.main_seed_sync.status = 'skipped'; },
  v => { v.main_seed_sync.baseline_seed_snapshot_id = snapshot; }, v => { v.main_seed_sync.refresh_strategy = 'already_aligned'; },
  v => { v.main_seed_sync.error = v.main_seed_sync.error.replace('os error 13', 'os error 1'); },
  v => { v.main_seed_sync.path = '/other/main-seed'; }]) {
  const changed = closeout(true); mutate(changed); assert.throws(() => validateCloseout(inside(changed), selected, true));
}
const success = closeout(true); success.closeout_status = 'complete'; success.main_seed_sync.status = 'refreshed';
validateCloseout(inside(success), selected, true, false);
const pid = 4567;
const rename = [`LOADED pid=${pid} euid=501 egid=20 executable=${cli}`,
  `RENAME pid=${pid} euid=501 egid=20 sfd=-2 src=${path.dirname(seed)}/.main-seed.tmp-4567-20261030T001313Z dfd=-2 dst=${seed}`,
  'source path=/stage uid=501 gid=20 mode=755', 'source_parent path=/parent uid=501 gid=20 mode=755',
  'destination path=/seed lstat_errno=2', 'destination_parent path=/parent uid=501 gid=20 mode=755',
  `RESULT pid=${pid} result=-1 errno=13`].join('\n');
validateObservation(rename, pid, cli, seed);
for (const text of [rename.replace(`LOADED pid=${pid}`, 'LOADED pid=1'),
  rename.replace('source_parent path=', 'missing path='), rename.replace(`RESULT pid=${pid}`, 'RESULT pid=1'),
  rename.replace('/.main-seed.tmp-', '/other-'), rename.replace(`executable=${cli}`, 'executable=/other/ait')]) {
  assert.throws(() => validateObservation(text, pid, cli, seed));
}
assert.equal((js.match(/probe\('commands', cli, \['task', 'finish'/g) ?? []).length, 1);
assert(!js.includes('mkdtempSync') && !js.includes('chmodSync') && !js.includes('exerciseLifecycle'));
console.log('Same-context observation guards reject changed authority, unsafe closeout, wrong branch and unavailable capture; ordinary enforcement preserved');
// Execute the complete inline orchestration with controlled child/filesystem
// boundaries. This proves ordering and result preservation, not hosted capture.
function runtime(options = {}) {
  const p = phase(), bytes = Buffer.from('landed fixture\n');
  const files = [{ path: 'first-land.txt', sha256: createHash('sha256').update(bytes).digest('hex'), mode: '0o644' }];
  for (const item of p.commands) {
    const value = JSON.parse(item.stdout_tail);
    if (value.auto_snapshot) value.auto_snapshot.files = files;
    if (item.label === 'candidate integrated Snapshot show') value.files = files;
    item.stdout_tail = JSON.stringify(value);
    item.stdout_sha256 = createHash('sha256').update(item.stdout_tail).digest('hex');
  }
  const assetBytes = Buffer.from('exact frozen CLI'), assetDigest = createHash('sha256').update(assetBytes).digest('hex');
  const authority = { release: { id: 'REL-FAM-0123456789ABCDEF', version: '1.2.0', python_version: '1.2.0',
    channel: 'stable', tag: 'v1.2.0', source_commit: 'b'.repeat(40) } };
  const status = { candidate: { stage_receipt_sha256: 'c'.repeat(64) } };
  const env = { RUNNER_TEMP: '/runner/temp', GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1', GITHUB_JOB: 'install',
    ImageVersion: 'current', ImageOS: 'macos15', RUNNER_NAME: 'same-vm', AIT_SEED_OBSERVATION_ROW: 'github-arm',
    AIT_CLEAN_HOST_PYTHON: '', PATH: '/usr/bin:/bin', HOME: '/Users/runner', TMPDIR: '/runner/temp',
    UNRELATED_SECRET: 'must not enter discovery logs',
    AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST: 'sha256:' + 'd'.repeat(64) };
  if (options.unsetPython) delete env.AIT_CLEAN_HOST_PYTHON;
  p.row.id = 'github-arm'; p.row.runner = 'macos-15';
  p.release = { ...authority.release, verification_stage: 'pre_tag',
    endpoint_config_sha256: createHash('sha256').update(JSON.stringify(authority)).digest('hex'),
    operator_status_sha256: createHash('sha256').update(JSON.stringify(status)).digest('hex'),
    candidate_stage_receipt_sha256: status.candidate.stage_receipt_sha256,
    candidate_artifact_digest: options.wrongCandidate ? 'sha256:' + 'e'.repeat(64) : env.AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST };
  p.runner = { run_id: env.GITHUB_RUN_ID, run_attempt: '1', job: 'install', image_version: env.ImageVersion,
    image_os: env.ImageOS, runner_name: env.RUNNER_NAME, label: p.row.runner };
  const assetName = `ait-1.2.0-${p.row.target}`;
  p.observations = { github_assets: [{ name: assetName, source: 'frozen_candidate_stage', digest: 'sha256:' + assetDigest }] };
  const data = new Map(Object.entries({
    'evidence/install.json': JSON.stringify(p),
    'candidate/ait-release.clean-host-matrix.json': JSON.stringify({ rows: [p.row] }),
    'candidate/ait-release.endpoints.authority.json': JSON.stringify(authority),
    'candidate/ait-release.prepublish-candidate.json': JSON.stringify(status),
    [cli]: assetBytes, ['candidate/assets/' + assetName]: assetBytes,
    [path.join(repository, 'first-land.txt')]: bytes,
    [path.join(repository, '.ait/config.json')]: '{"frozen":true}',
    [path.join(repository, 'docs/sprints/clean_host.md')]: '# Clean Host First Land [plan-ref: clean-host/root]\n\n## Work\n\n- [x] Materialize the exact clean-host file. [ref: clean-host/first-land]\n',
  }).map(([key, value]) => [key, Buffer.from(value)]));
  const original = Buffer.from(data.get('evidence/install.json'));
  const calls = [], diagnostic = '/runner/temp/mac-seed-observation', proc = { platform: 'darwin', arch: 'arm64', env };
  const missing = file => Object.assign(new Error('not found: ' + file), { code: 'ENOENT' });
  const context = vm.createContext({ assert, createHash, path, Buffer, process: proc,
    console: { log() {}, error() {} },
    readFileSync(file, encoding) { if (!data.has(file)) throw missing(file); return encoding ? data.get(file).toString() : data.get(file); },
    writeFileSync(file, text, settings) { data.set(file, Buffer.from((settings?.flag === 'a' ? data.get(file)?.toString() ?? '' : '') + text)); },
    realpathSync: file => file,
    existsSync: file => data.has(file),
    statSync: file => ({ dev: file === '/Volumes/AIT_RAM' ? 2 : 1 }),
    lstatSync(file) { if (file === seed) throw missing(file); return { mode: 0o644, isFile: () => true }; },
    spawnSync(command, args, settings) {
      calls.push({ command, args, settings });
      const result = { pid: 4567, status: 0, signal: null, stdout: '', stderr: '' };
      if (command === '/usr/bin/clang' && options.compileFailure) { result.status = 1; result.stderr = 'controlled compiler failure'; }
      if (command === '/usr/bin/which') result.stdout = '/usr/bin/hdiutil\n';
      if (command === 'hdiutil') {
        assert.deepEqual(Array.from(args), ['info', '-plist']);
        const observer = !!settings.env.DYLD_INSERT_LIBRARIES;
        result.stdout = observer && options.discoveryMismatch ? '<plist><dict/></plist>' : '<plist>controlled writable ram image</plist>';
        if (options.discoveryFailure) { result.status = 1; result.stderr = 'controlled hdiutil failure'; }
      }
      if (command === '/usr/bin/plutil') {
        assert.deepEqual(Array.from(args), ['-extract', 'images', 'json', '-expect', 'array', '-o', '-', '-']);
        assert(!settings.env.AIT_CLEAN_HOST_PYTHON, 'GitHub observer has no PyPI runtime setup');
        result.stdout = JSON.stringify(settings.input.includes('writable ram image') ? [
          { 'image-path': options.nonRam ? '/tmp/disk.dmg' : 'ram://123456', writeable: !options.readonlyRam,
            'system-entities': [{ 'mount-point': options.wrongMount ? '/Volumes/other' : '/Volumes/AIT_RAM' }] }] : []);
        if (options.invalidPlist) { result.status = 1; result.stdout = ''; result.stderr = 'controlled XML parse failure'; }
        if (options.missingParser) { result.status = null; result.stdout = ''; result.error = new Error('spawnSync /usr/bin/plutil ENOENT'); }
        if (options.invalidParserJson) result.stdout = '{invalid JSON';
      }
      if (command === '/sbin/mount') result.stdout = options.noRam ? '' : '/dev/disk9 on /Volumes/AIT_RAM (apfs, local)\n';
      if (command === '/usr/sbin/lsof') {
        result.status = options.writer ? 0 : 1; result.stdout = options.writer ? 'p999\nf1\naw\n' : '';
      }
      if (command === cli) {
        assert.equal(settings.cwd, repository);
        if (args[0] !== 'task' || args[1] !== 'finish') assert.equal(settings.env.DYLD_INSERT_LIBRARIES, undefined);
        if (args[0] === 'status') result.stdout = JSON.stringify({ current_line: 'main', head_snapshot_id: snapshot,
          workspace_dirty: !!options.dirty, is_worktree: false, snapshot_count: 2 });
        else if (args[0] === 'line') result.stdout = JSON.stringify({ head_snapshot_id: snapshot });
        else if (args[0] === 'workflow') result.stdout = JSON.stringify({ status: 'completed', mutated: false,
          findings: [], summary: { remaining_count: 0, total_finding_count: 0 },
          inventory: { interrupted_operation_count: 0, worktree_count: 0, selected_change_count: 0 },
          sources: { workspace_lock: { active: false } } });
        else if (args[1] === 'show') result.stdout = JSON.stringify({ status: 'completed', local_finish: receipt });
        else {
          assert.deepEqual(args, vm.runInContext(JSON.stringify(['task', 'finish', task, '--local', '--json', '--full']), context));
          assert.equal(settings.env.DYLD_INSERT_LIBRARIES, diagnostic + '/rename-observer.dylib');
          assert.equal(settings.env.CORE_SEED_RENAME_LOG, diagnostic + '/rename.log');
          result.status = options.success ? 0 : 2;
          const response = closeout(true);
          if (options.success) { response.closeout_status = 'complete'; response.main_seed_sync.status = 'refreshed'; }
          if (options.childUnavailable) response.main_seed_sync = { status: 'skipped', reason: 'managed_ephemeral_root_unavailable' };
          result.stdout = JSON.stringify(response);
          if (!options.noCapture) data.set(diagnostic + '/rename.log', Buffer.from(rename));
        }
      }
      return result;
    },
  });
  vm.runInContext(js.slice(js.indexOf('const digest =')), context);
  const attempts = calls.filter(call => call.command === cli && call.args[0] === 'task' && call.args[1] === 'finish');
  assert.deepEqual(data.get('evidence/install.json'), original);
  assert.equal(JSON.parse(data.get('evidence/install.json')).status, 'fail');
  const discoveryCalls = calls.filter(call => call.command === 'hdiutil');
  if (discoveryCalls.length) {
    assert.equal(discoveryCalls.length, 2);
    assert.equal(discoveryCalls[0].settings.env.DYLD_INSERT_LIBRARIES, undefined);
    assert.equal(discoveryCalls[1].settings.env.DYLD_INSERT_LIBRARIES, diagnostic + '/rename-observer.dylib');
    assert.equal(discoveryCalls[1].settings.env.CORE_SEED_RENAME_LOG, diagnostic + '/rename.log');
    const logs = ['discovery-ordinary-parent', 'discovery-observer-parent'].map(name =>
      JSON.parse(data.get(diagnostic + '/' + name + '.log').toString()));
    for (const log of logs) {
      assert.match(log.scope, /parent probe only/);
      assert.deepEqual(Object.keys(log.launcher_env).sort(),
        ['PATH', 'HOME', 'TMPDIR', 'DYLD_INSERT_LIBRARIES', 'CORE_SEED_RENAME_LOG'].sort());
      assert(!JSON.stringify(log).includes(env.UNRELATED_SECRET));
      assert.equal(log.resolved.stdout, '/usr/bin/hdiutil\n');
      assert.equal(log.info_plist.exit, options.discoveryFailure ? 1 : 0);
      assert(log.info_plist.stdout, 'raw discovery output must survive parser unavailability');
      assert.equal(log.parse.command, '/usr/bin/plutil');
      if (options.missingParser) assert.match(log.parse.error, /plutil ENOENT/);
      if (options.invalidParserJson) assert(log.parse.error);
    }
    const attemptIndex = calls.findIndex(call => call.command === cli && call.args[1] === 'finish');
    if (attemptIndex >= 0) assert(calls.indexOf(discoveryCalls[1]) < attemptIndex);
  }
  return { attempts: attempts.length, exitCode: proc.exitCode ?? 0 };
}
for (const options of [{}, { unsetPython: true }, { success: true }]) assert.deepEqual(runtime(options), { attempts: 1, exitCode: 0 });
for (const options of [{ wrongCandidate: true }, { noRam: true }, { dirty: true }, { writer: true }, { compileFailure: true },
  { discoveryMismatch: true }, { discoveryFailure: true }, { invalidPlist: true }, { readonlyRam: true },
  { nonRam: true }, { wrongMount: true }, { missingParser: true }, { invalidParserJson: true }]) {
  assert.deepEqual(runtime(options), { attempts: 0, exitCode: 1 });
}
assert.deepEqual(runtime({ noCapture: true }), { attempts: 1, exitCode: 1 });
assert.deepEqual(runtime({ childUnavailable: true }), { attempts: 1, exitCode: 1 });
// Execute the embedded discovery function with real Mac system executables.
// No PyPI setup, parser mock or AIT command participates in these tool checks.
const xml = '<plist version="1.0"><dict><key>images</key><array><dict>' +
  '<key>image-path</key><string>ram://123456</string><key>writeable</key><true/>' +
  '<key>system-entities</key><array><dict><key>mount-point</key><string>/Volumes/AIT_RAM</string>' +
  '</dict></array></dict></array></dict></plist>';
if (process.platform === 'darwin') {
  const start = js.indexOf('function discovery('), end = js.indexOf('\ntry {', start);
  assert(start >= 0 && end > start);
  const discoverySource = js.slice(start, end);
  function realDiscovery(input, options = {}) {
    const logs = [], executed = [];
    const env = { ...process.env, AIT_CLEAN_HOST_PYTHON: '', UNRELATED_SECRET: 'exclude actual-tool secret' };
    if (options.unsetPython) delete env.AIT_CLEAN_HOST_PYTHON;
    delete env.DYLD_INSERT_LIBRARIES; delete env.CORE_SEED_RENAME_LOG;
    const context = vm.createContext({
      spawnSync(command, args, settings) {
        assert(!settings.env.AIT_CLEAN_HOST_PYTHON);
        executed.push(command);
        const argv = Array.from(args);
        if (command === options.missingTool) return spawnSync('/nonexistent/ait-observer-required-tool', argv, settings);
        if (command === 'hdiutil' && input !== undefined) return { status: options.failedDiscovery ? 1 : 0,
          stdout: input, stderr: options.failedDiscovery ? 'controlled discovery failure' : '' };
        return spawnSync(command, argv, settings);
      },
      log(name, text) { logs.push(JSON.parse(text)); },
    });
    vm.runInContext(discoverySource + '\nthis.check = discovery;', context);
    const available = context.check('actual-tool-flow', env);
    assert.equal(logs.length, 1);
    const value = logs[0];
    assert(!JSON.stringify(value).includes(env.UNRELATED_SECRET));
    assert.deepEqual(executed, ['/usr/bin/which', 'hdiutil', '/usr/bin/plutil']);
    if (input !== undefined && options.missingTool !== 'hdiutil') assert.equal(value.info_plist.stdout, input);
    return { available, value };
  }
  for (const unsetPython of [false, true]) {
    const actual = realDiscovery(undefined, { unsetPython });
    assert.equal(actual.value.info_plist.exit, 0, JSON.stringify(actual.value));
    assert.equal(actual.value.parse.exit, 0);
    console.log('Actual Mac discovery optional Python=' + (unsetPython ? 'unset' : 'empty') +
      '; writable target RAM observed=' + actual.available);
    const fixture = realDiscovery(xml, { unsetPython });
    assert.equal(fixture.available, true, JSON.stringify(fixture.value));
    assert.deepEqual(fixture.value.fields, [{ 'image-path': 'ram://123456', writeable: true,
      'mount-points': ['/Volumes/AIT_RAM'] }]);
  }
  for (const input of ['', 'invalid XML', '<plist><dict/></plist>',
    '<plist><dict><key>images</key><array/></dict></plist>',
    '<plist><dict><key>images</key><dict/></dict></plist>',
    xml.replace('<true/>', '<false/>'), xml.replace('<key>writeable</key><true/>', ''),
    xml.replace('<key>image-path</key><string>ram://123456</string>', ''),
    xml.replace('<key>mount-point</key><string>/Volumes/AIT_RAM</string>', ''),
    xml.replace('ram://123456', '/tmp/ordinary.dmg')]) {
    assert.equal(realDiscovery(input).available, false);
  }
  assert.equal(realDiscovery(xml, { failedDiscovery: true }).available, false);
  for (const missingTool of ['/usr/bin/which', 'hdiutil', '/usr/bin/plutil']) {
    const missing = realDiscovery(xml, { missingTool });
    assert.equal(missing.available, false);
    const error = missingTool === '/usr/bin/plutil' ? missing.value.parse :
      missingTool === 'hdiutil' ? missing.value.info_plist : missing.value.resolved;
    assert.match(error.error, /ENOENT/);
    if (missingTool === '/usr/bin/plutil') assert.equal(missing.value.info_plist.stdout, xml);
  }
  console.log('Actual Mac hdiutil/plutil flow passed with empty/unset optional Python; raw failed/malformed/missing-tool diagnostics retained');
}
console.log('Inline observation orchestration: matching parent discovery probes, single guarded CLI attempt, unsafe inputs refused and original failure bytes preserved');
MAC_SEED_OBSERVATION_TEST
test "$(grep -c 'name: Activate preinstalled Linux Homebrew' "${workflow}")" = 3
test "$(grep -c 'name: Bootstrap Windows Package Manager' "${workflow}")" = 3
grep -F 'test -x /home/linuxbrew/.linuxbrew/bin/brew' "${workflow}" >/dev/null
test "$(grep -Fc './control/ci/release_winget_bootstrap.ps1' "${workflow}")" = 3
test "$(grep -Fc "\$gitBash = 'C:\Program Files\Git\bin\bash.exe'" \
  "${workflow}")" = 3
test "$(grep -Fc 'Split-Path -Parent $gitBash | Out-File -FilePath $env:GITHUB_PATH' \
  "${workflow}")" = 3
test "$(grep -c '^        id: python$' "${workflow}")" = 2
test "$(grep -Fc 'AIT_CLEAN_HOST_PYTHON: ${{ steps.python.outputs.python-path }}' \
  "${workflow}")" = 2
python_command_source=$(sed -n '/^function pythonCommand() {$/,/^}$/p' "${phase_runner}")
grep -F 'process.env.AIT_CLEAN_HOST_PYTHON' <<<"${python_command_source}" >/dev/null
grep -F 'requireExecutableFile(configured, "configured clean-host Python")' \
  <<<"${python_command_source}" >/dev/null
grep -F 'process.platform === "win32"' <<<"${python_command_source}" >/dev/null
grep -F 'explicit setup-python output' <<<"${python_command_source}" >/dev/null
grep -F 'package_manager_commands = { python: pythonCommand() }' \
  "${phase_runner}" >/dev/null
grep -F 'ait-pre-tag-candidate-${{ inputs.release_id }}' "${workflow}" >/dev/null
grep -F 'source_commit: ${{ steps.request.outputs.source_commit }}' "${workflow}" >/dev/null
grep -F 'ref: ${{ needs.stage.outputs.source_commit }}' "${workflow}" >/dev/null
grep -F 'AIT_CLEAN_HOST_SOURCE_ROOT: ${{ github.workspace }}/source' "${workflow}" >/dev/null
grep -F 'ait-pre-tag-clean-host-${{ inputs.release_id }}' "${workflow}" >/dev/null
grep -F 'release commit has a release tag before candidate qualification' \
  "${repo_root}/.github/workflows/ait-release-component-receipts.yml" >/dev/null
grep -F 'Recheck tag absence at qualification closeout' "${workflow}" >/dev/null
grep -F 'release_prepublish_verify.mjs qualify' "${workflow}" >/dev/null
test "$(grep -c 'mark(checks, "immutable_image_digest")' "${phase_runner}")" = 2
grep -F 'platformPackageName = `@wa120/ait-native-${npmTargetSuffix(row)}`' \
  "${phase_runner}" >/dev/null
grep -F 'fail("npm uninstall retained the target platform package")' \
  "${phase_runner}" >/dev/null

active_family=${repo_root}/ait-release-family.json
published_legacy_family=${repo_root}/release/families/1.1.0/ait-release-family.json
if [[ ! -f ${published_legacy_family} &&
      -f ${repo_root}/ait-core/release/families/1.1.0/ait-release-family.json ]]; then
  published_legacy_family=${repo_root}/ait-core/release/families/1.1.0/ait-release-family.json
fi
test -f "${published_legacy_family}"
test ! -L "${published_legacy_family}"
test "$(sha256_file "${published_legacy_family}")" = \
  e85722913ed6724eb8f9cbb56fc2fd4a84ebcaad9fa84acb2e2971b2cc6c87fd

family_version=$(jq -er '.family.version' "${active_family}")
active_platforms=${temporary_root}/active-platforms.json
jq --arg version "${family_version}" '.version = $version' \
  "${repo_root}/ci/native_bootstrap_matrix.json" >"${active_platforms}"

active_matrix_revision=$(jq -er '
  . as $family |
  [.distributions[] |
    select(.role == "product" and
      (.channel == "homebrew" or .channel == "apt" or .channel == "winget")) |
    (.components | index("ait-runner") != null)] |
  if all and ([$family.components[].id] | index("ait-agent") == null and index("ait-agent-worker") == null) then
    if $family.family == ($family.family + {version: "1.2.0", channel: "stable", tag: "v1.2.0"})
    then "distribution-target-120-24-mac-runtime-deferred-2026-10-31.1"
    else "distribution-target-120-32-2026-10-24.1" end
  elif all then "distribution-target-runner-bundle-32-2026-08-26.1"
  elif any then error("active native product bundles are inconsistent")
  else "distribution-target-32-2026-08-17.2"
  end
' "${active_family}")

matrix=${temporary_root}/matrix.json
node "${tool}" matrix \
  --family "${active_family}" \
  --platforms "${active_platforms}" \
  --output "${matrix}" >/dev/null
required_rows=$(jq -r 'if .family.version == "1.2.0" and .family.channel == "stable" and .family.tag == "v1.2.0" then 24 else 32 end' "${active_family}")
jq -e --arg matrix_revision "${active_matrix_revision}" --argjson required_rows "${required_rows}" '
  .contract == "ait.release.clean-host.matrix/v1" and
  .matrix_revision == $matrix_revision and
  .row_count == $required_rows and (.rows | length) == $required_rows and
  ([.rows[].id] | unique | length) == $required_rows and
  .counts == {
    "apt:product": 2,
    "apt:standalone": 2,
    "github:product": (if $required_rows == 24 then 4 else 6 end),
    "homebrew:product": (if $required_rows == 24 then 2 else 4 end),
    "npm:product": (if $required_rows == 24 then 4 else 6 end),
    "oci:standalone": 4,
    "pypi:product": (if $required_rows == 24 then 4 else 6 end),
    "winget:product": 2
  } and
  ([.rows[].runner] | unique | sort) == ([
    (if $required_rows == 24 then empty else "macos-15", "macos-15-intel" end), "ubuntu-22.04",
    "ubuntu-22.04-arm", "windows-11-arm", "windows-2025"
  ] | sort)
' "${matrix}" >/dev/null

if [[ "${family_version}" == 1.2.* ]]; then
  jq -e --argjson targets "$((required_rows == 24 ? 4 : 6))" '
    ([.rows[] | select(.channel == "github" and .lifecycle == "product" and
      ((.required_checks.install | index("language_repositories")) != null and
       (.required_checks.install | index("installed_core_cases")) != null))] | length) == $targets and
    ([.rows[] | select(.channel != "github" or .lifecycle != "product") |
      select((.required_checks.install | index("language_repositories") != null) or
             (.required_checks.install | index("installed_core_cases") != null))] | length) == 0
  ' "${matrix}" >/dev/null
fi

jq -e '
  all(.rows[] | select(.channel == "pypi");
    .components == ["ait-python"] and .binding == "python" and .service == null and .lifecycle == "product")
' "${matrix}" >/dev/null
wrong_python_payload=${temporary_root}/wrong-python-native-payload.json
jq '(.distributions[] | select(.channel == "pypi")).components = ["ait", "ait-server", "ait-python"]' \
  "${active_family}" >"${wrong_python_payload}"
expect_failure python-native-payload node "${tool}" matrix \
  --family "${wrong_python_payload}" --platforms "${active_platforms}" \
  --output "${temporary_root}/wrong-python-matrix.json"

published_legacy_platforms=${temporary_root}/published-legacy-platforms.json
jq '.version = "1.1.0"' \
  "${repo_root}/ci/native_bootstrap_matrix.json" >"${published_legacy_platforms}"
published_legacy_matrix=${temporary_root}/published-legacy-matrix.json
node "${tool}" matrix \
  --family "${published_legacy_family}" \
  --platforms "${published_legacy_platforms}" \
  --output "${published_legacy_matrix}" >/dev/null
jq -e '
  .matrix_revision == "distribution-target-32-2026-08-17.2" and
  .row_count == 32 and (.rows | length) == 32 and
  ([.rows[].id] | unique | length) == 32
' "${published_legacy_matrix}" >/dev/null

legacy_future_family=${temporary_root}/legacy-future-family.json
jq '
  .family.version = "1.1.1" |
  .family.tag = "v1.1.1" |
  .components |= map(
    if .version_scheme == "family" then .version = "1.1.1"
    elif .version_scheme == "pep440" then .version = "1.1.1"
    else . end
  )
' "${published_legacy_family}" >"${legacy_future_family}"
legacy_future_platforms=${temporary_root}/legacy-future-platforms.json
jq '.version = "1.1.1"' \
  "${repo_root}/ci/native_bootstrap_matrix.json" >"${legacy_future_platforms}"
expect_failure future-legacy-native-bundle node "${tool}" matrix \
  --family "${legacy_future_family}" \
  --platforms "${legacy_future_platforms}" \
  --output "${temporary_root}/legacy-future-matrix.json"
grep -F '1.1+ clean-host qualification requires the native runner-bundle matrix' \
  "${temporary_root}/future-legacy-native-bundle.stderr" >/dev/null

altered_published_family=${temporary_root}/altered-published-family.json
jq '.compatibility.native_protocol = "altered"' \
  "${published_legacy_family}" >"${altered_published_family}"
expect_failure altered-published-native-bundle node "${tool}" matrix \
  --family "${altered_published_family}" \
  --platforms "${published_legacy_platforms}" \
  --output "${temporary_root}/altered-published-matrix.json"
grep -F 'legacy matrix is admitted only for the exact immutable published 1.1.0 family' \
  "${temporary_root}/altered-published-native-bundle.stderr" >/dev/null

runner_bundle_family=${temporary_root}/runner-bundle-family.json
jq '
  .family.version = "1.1.1" |
  .family.channel = "stable" |
  .family.tag = "v1.1.1" |
  .components |= map(
    if .version_scheme == "family" then .version = "1.1.1"
    elif .version_scheme == "pep440" then .version = "1.1.1"
    else . end
  ) |
  .distributions |= map(
    if .role == "product" and
       (.channel == "homebrew" or .channel == "apt" or .channel == "winget")
    then .components = ["ait", "ait-server", "ait-runner"]
    else . end
  )
' "${published_legacy_family}" >"${runner_bundle_family}"
runner_bundle_platforms=${temporary_root}/runner-bundle-platforms.json
jq '.version = "1.1.1"' \
  "${repo_root}/ci/native_bootstrap_matrix.json" >"${runner_bundle_platforms}"
runner_bundle_matrix=${temporary_root}/runner-bundle-matrix.json
node "${tool}" matrix \
  --family "${runner_bundle_family}" \
  --platforms "${runner_bundle_platforms}" \
  --output "${runner_bundle_matrix}" >/dev/null
jq -e '
  .matrix_revision == "distribution-target-runner-bundle-32-2026-08-26.1" and
  .row_count == 32 and (.rows | length) == 32 and
  ([.rows[].id] | unique | length) == 32 and
  ([.rows[] |
    select(.role == "product" and
      (.channel == "homebrew" or .channel == "apt" or .channel == "winget")) |
    .components] | all(. == ["ait", "ait-server", "ait-runner"])) and
  ([.rows[] |
    select(.channel == "apt" and .role == "standalone") |
    .components] == [["ait-runner"], ["ait-runner"]])
' "${runner_bundle_matrix}" >/dev/null

node --input-type=module - "${repo_root}" "${temporary_root}" "${active_family}" <<'RUNTIME_SCOPE_TEST'
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
const [root, temporary, familyFile] = process.argv.slice(2);
const { validateMatrix, requiredCleanHostRows } = await import(pathToFileURL(path.join(root, 'ci/release_clean_host.mjs')));
const read = file => JSON.parse(readFileSync(file, 'utf8'));
const family = read(familyFile), platforms = read(path.join(root, 'ci/native_bootstrap_matrix.json'));
assert.equal(platforms.targets.length, 6);
assert.equal(platforms.targets.filter(row => row.os === 'macos').length, 2);
for (const channel of ['github', 'pypi', 'npm', 'homebrew']) {
  const distribution = family.distributions.find(row => row.channel === channel && row.role === 'product');
  for (const target of ['aarch64-apple-darwin', 'x86_64-apple-darwin']) assert(distribution.targets.includes(target));
}
const workflow = readFileSync(path.join(root, '.github/workflows/ait-release-prepublish-clean-host.yml'), 'utf8');
const probeFilter = workflow.split('\n').find(line => line.includes('jq -c --slurpfile matrix')).match(/'([^']+)'/)[1];
let index = 0;
for (const release of [
  { version: '1.2.0', channel: 'stable', tag: 'v1.2.0' },
  { version: '1.2.0', channel: 'rc', tag: 'v1.2.0' },
  { version: '1.2.0-rc.1', channel: 'rc', tag: 'v1.2.0-rc.1' },
  { version: '1.2.1', channel: 'stable', tag: 'v1.2.1' },
]) {
  const scoped = release.version === '1.2.0' && release.channel === 'stable';
  const expected = scoped ? 24 : 32;
  assert.equal(requiredCleanHostRows(release), expected);
  const source = structuredClone(family); Object.assign(source.family, release);
  const platformFile = path.join(temporary, `scope-${index}.platforms.json`);
  const sourceFile = path.join(temporary, `scope-${index}.family.json`);
  const matrixFile = path.join(temporary, `scope-${index++}.matrix.json`);
  writeFileSync(sourceFile, JSON.stringify(source));
  writeFileSync(platformFile, JSON.stringify({ ...platforms, version: release.version }));
  const generated = spawnSync(process.execPath, [path.join(root, 'ci/release_clean_host.mjs'), 'matrix',
    '--family', sourceFile, '--platforms', platformFile, '--output', matrixFile], { encoding: 'utf8' });
  assert.equal(generated.status, 0, generated.stderr);
  const matrix = read(matrixFile);
  assert.equal(matrix.row_count, expected);
  validateMatrix(matrix, { release });
  const probe = spawnSync('jq', ['-c', '--slurpfile', 'matrix', matrixFile, probeFilter, platformFile], { encoding: 'utf8' });
  assert.equal(probe.status, 0, probe.stderr);
  const targets = JSON.parse(probe.stdout).include;
  assert.equal(targets.length, scoped ? 4 : 6);
  assert.equal(targets.some(row => row.os === 'macos'), !scoped);
  if (!scoped) continue;
  for (const mutate of [
    value => { value.rows.pop(); },
    value => { value.rows[0] = value.rows[1]; },
    value => { value.rows[0].id = 'substituted-row'; },
    value => { value.rows[0].required_checks.install.pop(); },
    value => { value.rows[0].target = 'aarch64-apple-darwin'; },
    value => { value.release.tag = 'v1.2.1'; },
  ]) {
    const invalid = structuredClone(matrix); mutate(invalid);
    assert.throws(() => validateMatrix(invalid, { release }));
  }
  for (const changed of [{ ...release, version: '1.2.1' }, { ...release, channel: 'rc' },
    { ...release, tag: 'v1.2.1' }, { version: '1.2.0', tag: 'v1.2.0' }]) {
    assert.equal(requiredCleanHostRows(changed), 32);
    assert.throws(() => validateMatrix(matrix, { release: changed }));
  }
}
RUNTIME_SCOPE_TEST

config=${temporary_root}/config.json
status=${temporary_root}/status.json
version=${family_version}
python_version=$(jq -er '.components[] | select(.id == "ait-python") | .version' \
  "${repo_root}/ait-release-family.json")
tag=v${version}
channel=$(jq -er '.family.channel' "${repo_root}/ait-release-family.json")
jq -n \
  --arg version "${version}" \
  --arg python_version "${python_version}" \
  --arg channel "${channel}" \
  --arg tag "${tag}" '
  {
    contract: "ait.release.family.endpoints/v1",
    release: {
      id: "REL-FAM-0123456789ABCDEF",
      version: $version,
      python_version: $python_version,
      channel: $channel,
      tag: $tag,
      source_commit: "1111111111111111111111111111111111111111"
    }
  }
' >"${config}"
jq -n \
  --arg version "${version}" \
  --arg tag "${tag}" '
  {
    contract: "ait.release.prepublish.candidate/v1",
    status: "frozen_candidate_pending_clean_host",
    release: {
      id: "REL-FAM-0123456789ABCDEF",
      version: $version,
      tag: $tag,
      source_commit: "1111111111111111111111111111111111111111"
    },
    candidate: {
      stage_receipt_sha256: "3333333333333333333333333333333333333333333333333333333333333333"
    },
    qualification_stage: "post_authorization",
    tag_state: "present",
    public_endpoint_writes: false
  }
' >"${status}"

export AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST=sha256:4444444444444444444444444444444444444444444444444444444444444444

config_sha=$(sha256_file "${config}")
status_sha=$(sha256_file "${status}")
release_binding=${temporary_root}/release-binding.json
jq -n \
  --arg version "${version}" \
  --arg python_version "${python_version}" \
  --arg channel "${channel}" \
  --arg tag "${tag}" \
  --arg config_sha "${config_sha}" \
  --arg status_sha "${status_sha}" '
  {
    id: "REL-FAM-0123456789ABCDEF",
    version: $version,
    python_version: $python_version,
    channel: $channel,
    tag: $tag,
    source_commit: "1111111111111111111111111111111111111111",
    endpoint_config_sha256: $config_sha,
    operator_status_sha256: $status_sha,
    verification_stage: "prepublication",
    candidate_stage_receipt_sha256: "3333333333333333333333333333333333333333333333333333333333333333",
    candidate_artifact_digest: "sha256:4444444444444444444444444444444444444444444444444444444444444444"
  }
' >"${release_binding}"

evidence_root=${temporary_root}/rows
mkdir "${evidence_root}"
while IFS= read -r row; do
  row_id=$(jq -er .id <<<"${row}")
  row_file=${temporary_root}/${row_id}.row.json
  printf '%s\n' "${row}" >"${row_file}"
  install=${temporary_root}/${row_id}.install.json
  upgrade=${temporary_root}/${row_id}.upgrade.json
  jq -n \
    --slurpfile release "${release_binding}" \
    --slurpfile row "${row_file}" \
    --slurpfile language "${temporary_root}/language-toolchains.json" \
    --slurpfile core "${temporary_root}/core-cases.json" '
    ($row[0]) as $r |
    {
      contract: "ait.release.clean-host.phase/v1",
      status: "pass",
      phase: "install",
      release: $release[0],
      row: $r,
      runner: {
        label: $r.runner,
        target_verified: true,
        github_hosted: true,
        run_id: "7001",
        run_attempt: "1",
        job: ("install-" + $r.id)
      },
      checks: (reduce $r.required_checks.install[] as $check
        ({}; .[$check] = true)),
      observations: (if ($r.required_checks.install | index("language_repositories")) != null then
        { language_repositories: (["rust", "typescript_node", "go", "java"] |
          map({language: ., root: ("/fixture/" + .), snapshot_id: "SNP-123456789ABC",
            toolchains: $language[0][.] })),
          installed_core_cases: { source_commit: $release[0].source_commit,
            core_snapshot: "SNP-123456789ABC", cli_sha256: ("a" * 64),
            server_sha256: ("b" * 64), runner_sha256: ("c" * 64), cases: $core[0] } }
        else {} end)
    }
  ' >"${install}"
  jq -n \
    --slurpfile release "${release_binding}" \
    --slurpfile row "${row_file}" '
    ($row[0]) as $r |
    {
      contract: "ait.release.clean-host.phase/v1",
      status: "pass",
      phase: "upgrade",
      release: $release[0],
      row: $r,
      runner: {
        label: $r.runner,
        target_verified: true,
        github_hosted: true,
        run_id: "7001",
        run_attempt: "1",
        job: ("upgrade-" + $r.id)
      },
      checks: (reduce $r.required_checks.upgrade[] as $check
        ({}; .[$check] = true))
    }
  ' >"${upgrade}"
  node "${tool}" combine \
    --matrix "${matrix}" \
    --config "${config}" \
    --status "${status}" \
    --install "${install}" \
    --upgrade "${upgrade}" \
    --output "${evidence_root}/${row_id}.json" >/dev/null
done < <(jq -c '.rows[]' "${matrix}")

if [[ "${family_version}" == 1.2.* ]]; then
  github_row_id=$(jq -er '.rows[] | select(.channel == "github" and .lifecycle == "product") | .id' "${matrix}" | head -1)
  jq '.observations.language_repositories |= .[:3]' \
    "${temporary_root}/${github_row_id}.install.json" >"${temporary_root}/missing-language.install.json"
  expect_failure missing-language-observation node "${tool}" combine \
    --matrix "${matrix}" --config "${config}" --status "${status}" \
    --install "${temporary_root}/missing-language.install.json" \
    --upgrade "${temporary_root}/${github_row_id}.upgrade.json" \
    --output "${temporary_root}/missing-language.row.json"
  grep -F 'install lacks four exact language repository results' \
    "${temporary_root}/missing-language-observation.stderr" >/dev/null
fi

aggregate=${temporary_root}/aggregate
node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${evidence_root}" \
  --output-root "${aggregate}" >/dev/null
jq -e --arg matrix_revision "${active_matrix_revision}" --argjson required_rows "${required_rows}" '
  .contract == "ait.release.clean-host.aggregate/v1" and
  .status == "qualified" and
  .matrix == {admitted_rows: $required_rows, evidence_files: $required_rows, expected_rows: $required_rows,
    revision: $matrix_revision} and
  .failures == [] and
  .promotion == {allowed: true, retry_same_candidate: false, terminal_for_release: false}
' "${aggregate}/ait-release.clean-host-status.json" >/dev/null
test "$(wc -l <"${aggregate}/SHA256SUMS" | tr -d '[:space:]')" = "$((required_rows + 1))"
test "$(find "${aggregate}/rows" -type f -name '*.json' | wc -l | tr -d '[:space:]')" = "${required_rows}"

missing_root=${temporary_root}/missing-rows
cp -R "${evidence_root}" "${missing_root}"
missing_id=$(jq -er '.rows[0].id' "${matrix}")
rm -- "${missing_root}/${missing_id}.json"
missing_aggregate=${temporary_root}/missing-aggregate
expect_failure missing-row node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${missing_root}" \
  --output-root "${missing_aggregate}"
jq -e --arg row_id "${missing_id}" '
  .status == "blocked" and
  .promotion == {allowed: false, retry_same_candidate: true, terminal_for_release: false} and
  any(.failures[]; .row_id == $row_id and .reason == "missing_row")
' "${missing_aggregate}/ait-release.clean-host-status.json" >/dev/null

duplicate_root=${temporary_root}/duplicate-rows
cp -R "${evidence_root}" "${duplicate_root}"
cp "${duplicate_root}/${missing_id}.json" "${duplicate_root}/duplicate.json"
duplicate_aggregate=${temporary_root}/duplicate-aggregate
expect_failure duplicate-row node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${duplicate_root}" \
  --output-root "${duplicate_aggregate}"
jq -e --arg row_id "${missing_id}" '
  .status == "blocked" and
  any(.failures[]; .row_id == $row_id and .reason == "filename_mismatch") and
  any(.failures[]; .row_id == $row_id and .reason == "duplicate_row") and
  (any(.failures[]; .row_id == $row_id and .reason == "missing_row") | not)
' "${duplicate_aggregate}/ait-release.clean-host-status.json" >/dev/null

invalid_root=${temporary_root}/invalid-rows
cp -R "${evidence_root}" "${invalid_root}"
jq '.status = "fail"' "${invalid_root}/${missing_id}.json" \
  >"${invalid_root}/${missing_id}.json.new"
mv "${invalid_root}/${missing_id}.json.new" "${invalid_root}/${missing_id}.json"
invalid_aggregate=${temporary_root}/invalid-aggregate
expect_failure invalid-row node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${invalid_root}" \
  --output-root "${invalid_aggregate}"
jq -e --arg row_id "${missing_id}" '
  .status == "blocked" and
  any(.failures[]; .row_id == $row_id and .reason == "invalid_or_failed_row") and
  (any(.failures[]; .row_id == $row_id and .reason == "missing_row") | not)
' "${invalid_aggregate}/ait-release.clean-host-status.json" >/dev/null

first_install=${temporary_root}/first.install.json
first_upgrade=${temporary_root}/first.upgrade.json
first_row=$(jq -c '.rows[0]' "${matrix}")
first_id=$(jq -r .id <<<"${first_row}")
jq '.phases.install' "${evidence_root}/${first_id}.json" >"${first_install}"
jq --arg job "$(jq -er '.runner.job' "${first_install}")" \
  '.phases.upgrade | .runner.job = $job' \
  "${evidence_root}/${first_id}.json" >"${first_upgrade}"
expect_failure reused-job node "${tool}" combine \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --install "${first_install}" \
  --upgrade "${first_upgrade}" \
  --output "${temporary_root}/reused-job.json"

printf 'release clean-host contract: pass\n'
