import assert from "node:assert/strict";
import { lstat, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VERSION = "1.0.0-rc.1";
const TARGETS = new Map([
  ["aarch64-apple-darwin", ["darwin", "arm64"]],
  ["x86_64-apple-darwin", ["darwin", "x64"]],
  ["aarch64-unknown-linux-gnu", ["linux", "arm64"]],
  ["x86_64-unknown-linux-gnu", ["linux", "x64"]],
  ["aarch64-pc-windows-msvc", ["win32", "arm64"]],
  ["x86_64-pc-windows-msvc", ["win32", "x64"]],
]);

async function json(relativePath) {
  return JSON.parse(await readFile(path.join(ROOT, relativePath), "utf8"));
}

test("top-level package is one portable command-only npm acquisition path", async () => {
  const packageJson = await json("package.json");

  assert.equal(packageJson.name, "ait-native");
  assert.equal(packageJson.private, undefined);
  assert.equal(packageJson.version, VERSION);
  assert.equal(packageJson.license, "Apache-2.0");
  assert.deepEqual(packageJson.bin, {
    ait: "bin/ait.mjs",
    "ait-server": "bin/ait-server.mjs",
  });
  assert.deepEqual(packageJson.exports, {});
  assert.equal(packageJson.types, undefined);
  assert.equal(packageJson.main, undefined);
  assert.equal(packageJson.dependencies, undefined);
  assert.equal(packageJson.os, undefined);
  assert.equal(packageJson.cpu, undefined);
  assert.deepEqual(packageJson.files, ["bin", "lib", "LICENSE", "NOTICE"]);
  for (const hook of ["preinstall", "install", "postinstall", "prepack"]) {
    assert.equal(packageJson.scripts[hook], undefined);
  }

  assert.match(await readFile(path.join(ROOT, "LICENSE"), "utf8"), /Apache License/);
  assert.match(await readFile(path.join(ROOT, "NOTICE"), "utf8"), /ait-node/);
  await assert.rejects(lstat(path.join(ROOT, "native")), { code: "ENOENT" });
  await assert.rejects(lstat(path.join(ROOT, "src")), { code: "ENOENT" });
});

test("payload contract declares two exact independently licensed packages per target", async () => {
  const packageJson = await json("package.json");
  const contract = await json("lib/npm-payload-contract.json");
  assert.deepEqual(Object.keys(contract).sort(), [
    "family_version",
    "payloads",
    "schema",
    "top_level_package",
  ]);
  assert.equal(contract.schema, "ait.node.npm-platform-packages/v1");
  assert.equal(contract.family_version, VERSION);
  assert.equal(contract.top_level_package, "ait-native");
  assert.equal(contract.payloads.length, 12);

  const packages = new Set();
  const selections = new Set();
  for (const payload of contract.payloads) {
    assert.deepEqual(Object.keys(payload).sort(), [
      "command",
      "component",
      "cpu",
      "executable",
      "license",
      "os",
      "package",
      "source_repository",
      "target",
      "version",
    ]);
    const platform = TARGETS.get(payload.target);
    assert.notEqual(platform, undefined);
    assert.deepEqual([payload.os, payload.cpu], platform);
    assert.equal(payload.version, VERSION);
    assert.equal(packageJson.optionalDependencies[payload.package], VERSION);
    assert.equal(packages.has(payload.package), false);
    packages.add(payload.package);
    selections.add(`${payload.component}/${payload.target}`);

    if (payload.component === "ait") {
      assert.equal(payload.command, "ait");
      assert.equal(payload.source_repository, "ait-core");
      assert.equal(payload.license, "Apache-2.0");
      assert.equal(payload.package, `ait-native-ait-${payload.os}-${payload.cpu}`);
    } else {
      assert.equal(payload.component, "ait-server");
      assert.equal(payload.command, "ait-server");
      assert.equal(payload.source_repository, "ait-server");
      assert.equal(payload.license, "AGPL-3.0-only");
      assert.equal(payload.package, `ait-native-server-${payload.os}-${payload.cpu}`);
    }
    const extension = payload.os === "win32" ? ".exe" : "";
    assert.equal(payload.executable, `bin/${payload.command}${extension}`);
  }
  assert.equal(packages.size, 12);
  assert.equal(Object.keys(packageJson.optionalDependencies).length, 12);
  for (const target of TARGETS.keys()) {
    assert.equal(selections.has(`ait/${target}`), true);
    assert.equal(selections.has(`ait-server/${target}`), true);
  }
});

test("launchers select only package-owned platform bytes", async () => {
  const source = await readFile(path.join(ROOT, "bin", "launch.mjs"), "utf8");
  assert.match(source, /node:child_process/);
  assert.match(source, /process\.platform/);
  assert.match(source, /process\.arch/);
  assert.match(source, /require\.resolve/);
  assert.match(source, /isSymbolicLink/);
  assert.doesNotMatch(source, /https?:\/\//);
  assert.doesNotMatch(source, /\b(fetch|cargo|cmake|gradle|dotnet)\b/i);
  assert.doesNotMatch(source, /spawnSync\([^)]*npm/i);
  assert.doesNotMatch(
    source,
    /pyproject|package\.json.*fixture|composer\.json|pom\.xml|csproj|CMakeLists/i,
  );

  assert.equal(
    (await readFile(path.join(ROOT, "bin", "ait.mjs"), "utf8")).includes(
      'launch("ait")',
    ),
    true,
  );
  assert.equal(
    (
      await readFile(path.join(ROOT, "bin", "ait-server.mjs"), "utf8")
    ).includes('launch("ait-server")'),
    true,
  );
});

test("receipt packager has no native build or publication path", async () => {
  const source = await readFile(
    path.join(ROOT, "release", "npm-payload-package.mjs"),
    "utf8",
  );
  assert.match(source, /ait\.release\.adapter\.receipt\/v1/);
  assert.match(source, /source_snapshot/);
  assert.match(source, /source_receipt/);
  assert.match(source, /isSymbolicLink/);
  assert.match(source, /SHA-256 drift/);
  assert.doesNotMatch(source, /npm\s+publish/i);
  assert.doesNotMatch(source, /\b(cargo|cmake|gradle|dotnet|napi)\b/i);
});

test("there is no JavaScript API, addon, or project detection surface", async () => {
  const packageJson = await json("package.json");
  assert.deepEqual(packageJson.exports, {});
  await assert.rejects(lstat(path.join(ROOT, "native", "ait-napi.node")), {
    code: "ENOENT",
  });
  await assert.rejects(lstat(path.join(ROOT, "ait-external.toml")), {
    code: "ENOENT",
  });
  await assert.rejects(lstat(path.join(ROOT, "ait-external.lock")), {
    code: "ENOENT",
  });
});

test("Windows CI mirrors the attempt-owned command-envelope validation", async () => {
  const entrypoint = await readFile(path.join(ROOT, "ci", "run.ps1"), "utf8");

  assert.match(entrypoint, /@\("patchset", "repo", "all"\)/);
  assert.match(entrypoint, /AIT_RUNNER_ATTEMPT_ROOT/);
  assert.match(entrypoint, /"ait-node-ci\." \+ \[Guid\]::NewGuid\(\)/);
  assert.match(entrypoint, /npm_config_cache/);
  assert.match(entrypoint, /npm\.cmd/);
  assert.match(entrypoint, /node\.exe/);
  assert.match(entrypoint, /"test"/);
  assert.match(entrypoint, /"run", "check"/);
  assert.match(entrypoint, /"pack", "--ignore-scripts", "--dry-run"/);
  assert.match(entrypoint, /"build", "portable", "1\.0\.0-rc\.1"/);
  assert.match(entrypoint, /"smoke", "portable", "1\.0\.0-rc\.1"/);
  assert.match(entrypoint, /Remove-Item -LiteralPath \$ciRoot -Recurse -Force/);
  assert.doesNotMatch(entrypoint, /Invoke-Expression|Start-Process|cmd\.exe/);
});
