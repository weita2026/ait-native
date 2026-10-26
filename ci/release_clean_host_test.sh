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
test "$(grep -c 'CHECKSUM_ASSET_NAME.test(match\[2\])' "${phase_runner}")" = 2
node --input-type=module - "${phase_runner}" "${temporary_root}" <<'NODE'
import assert from "node:assert/strict";
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
  if all and ([$family.components[].id] | index("ait-agent") == null and index("ait-agent-worker") == null) then "distribution-target-120-32-2026-10-24.1"
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
jq -e --arg matrix_revision "${active_matrix_revision}" '
  .contract == "ait.release.clean-host.matrix/v1" and
  .matrix_revision == $matrix_revision and
  .row_count == 32 and (.rows | length) == 32 and
  ([.rows[].id] | unique | length) == 32 and
  .counts == {
    "apt:product": 2,
    "apt:standalone": 2,
    "github:product": 6,
    "homebrew:product": 4,
    "npm:product": 6,
    "oci:standalone": 4,
    "pypi:product": 6,
    "winget:product": 2
  } and
  ([.rows[].runner] | unique | sort) == ([
    "macos-15", "macos-15-intel", "ubuntu-22.04",
    "ubuntu-22.04-arm", "windows-11-arm", "windows-2025"
  ] | sort)
' "${matrix}" >/dev/null

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
    --slurpfile row "${row_file}" '
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
        ({}; .[$check] = true))
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

aggregate=${temporary_root}/aggregate
node "${tool}" aggregate \
  --matrix "${matrix}" \
  --config "${config}" \
  --status "${status}" \
  --evidence-root "${evidence_root}" \
  --output-root "${aggregate}" >/dev/null
jq -e --arg matrix_revision "${active_matrix_revision}" '
  .contract == "ait.release.clean-host.aggregate/v1" and
  .status == "qualified" and
  .matrix == {admitted_rows: 32, evidence_files: 32, expected_rows: 32,
    revision: $matrix_revision} and
  .failures == [] and
  .promotion == {allowed: true, retry_same_candidate: false, terminal_for_release: false}
' "${aggregate}/ait-release.clean-host-status.json" >/dev/null
test "$(wc -l <"${aggregate}/SHA256SUMS" | tr -d '[:space:]')" = 33
test "$(find "${aggregate}/rows" -type f -name '*.json' | wc -l | tr -d '[:space:]')" = 32

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
