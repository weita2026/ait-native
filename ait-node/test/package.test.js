import assert from "node:assert/strict";
import { lstat, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { resolveNativeManifest } from "../scripts/native-build.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VERSION = "1.0.0-rc.12";
const CORE_SNAPSHOT = "SNP-A25D351DE720";
const PACKAGE_NAME = "@wa120/ait-native";
const PRODUCT_DESCRIPTION =
  "Agent-first, language-neutral workflow for verified repository changes";
const OFFICIAL_WEBSITE = "https://ait-native.dev/";
const REPOSITORY = {
  type: "git",
  url: "git+https://github.com/weita2026/ait-native.git",
  directory: "ait-node",
};
const TARGETS = new Map([
  ["aarch64-apple-darwin", { os: "darwin", cpu: "arm64", libc: null }],
  ["x86_64-apple-darwin", { os: "darwin", cpu: "x64", libc: null }],
  ["aarch64-unknown-linux-gnu", { os: "linux", cpu: "arm64", libc: "glibc" }],
  ["x86_64-unknown-linux-gnu", { os: "linux", cpu: "x64", libc: "glibc" }],
  ["aarch64-pc-windows-msvc", { os: "win32", cpu: "arm64", libc: null }],
  ["x86_64-pc-windows-msvc", { os: "win32", cpu: "x64", libc: null }],
]);

async function json(relativePath) {
  return JSON.parse(await readFile(path.join(ROOT, relativePath), "utf8"));
}

async function resolvedCoreSource() {
  const source = await resolveNativeManifest();
  return {
    ...source,
    coreRoot: path.resolve(
      path.dirname(source.manifestPath),
      "..",
      "..",
      "..",
    ),
  };
}

test("top-level package is one portable direct Node-API envelope", async () => {
  const packageJson = await json("package.json");

  assert.equal(packageJson.name, PACKAGE_NAME);
  assert.equal(packageJson.private, undefined);
  assert.equal(packageJson.version, VERSION);
  assert.equal(packageJson.description, PRODUCT_DESCRIPTION);
  assert.equal(packageJson.homepage, OFFICIAL_WEBSITE);
  assert.equal(packageJson.license, "Apache-2.0");
  assert.deepEqual(packageJson.repository, REPOSITORY);
  assert.deepEqual(packageJson.bin, { ait: "bin/ait.mjs" });
  assert.deepEqual(packageJson.exports, {
    ".": {
      types: "./src/index.d.ts",
      import: "./src/index.js",
      default: "./src/index.js",
    },
  });
  assert.equal(packageJson.types, "./src/index.d.ts");
  assert.equal(packageJson.main, undefined);
  assert.equal(packageJson.dependencies, undefined);
  assert.equal(packageJson.os, undefined);
  assert.equal(packageJson.cpu, undefined);
  assert.equal(packageJson.libc, undefined);
  assert.deepEqual(packageJson.files, [
    "bin/ait.mjs",
    "lib",
    "src",
    "LICENSE",
    "NOTICE",
  ]);
  for (const hook of ["preinstall", "install", "postinstall", "prepack"]) {
    assert.equal(packageJson.scripts[hook], undefined);
  }
  assert.equal(packageJson.scripts["native:build"], "node scripts/native-build.mjs build");

  assert.match(await readFile(path.join(ROOT, "LICENSE"), "utf8"), /Apache License/);
  const notice = await readFile(path.join(ROOT, "NOTICE"), "utf8");
  assert.match(notice, /ait-node/);
  assert.equal(
    notice.split("----- BEGIN GENERATED THIRD-PARTY NOTICES -----").length - 1,
    1,
  );
  assert.doesNotMatch(notice, /\/\.cargo\/registry\/|\/Users\/|\/Volumes\//);
  const { coreRoot } = await resolvedCoreSource();
  const cargoLock = await readFile(
    path.join(coreRoot, "rust", "Cargo.lock"),
    "utf8",
  );
  for (const block of cargoLock.split("[[package]]").slice(1)) {
    if (!/^source\s*=\s*"/m.test(block)) {
      continue;
    }
    const name = block.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
    const version = block.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
    assert.ok(name && version, "Cargo.lock package identity is incomplete");
    assert.match(
      notice,
      new RegExp(`^${escapeRegExp(name)}\\t${escapeRegExp(version)}\\t`, "m"),
      `NOTICE is missing ${name} ${version}`,
    );
  }
  for (const removed of ["bin/ait-server.mjs", "bin/launch.mjs"]) {
    await assert.rejects(lstat(path.join(ROOT, removed)), { code: "ENOENT" });
  }
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

test("platform contract declares one exact addon per target", async () => {
  const packageJson = await json("package.json");
  const contract = await json("lib/npm-payload-contract.json");
  assert.deepEqual(Object.keys(contract).sort(), [
    "family_version",
    "payloads",
    "schema",
    "top_level_package",
  ]);
  assert.equal(contract.schema, "ait.node.napi-platform-packages/v2");
  assert.equal(contract.family_version, VERSION);
  assert.equal(contract.top_level_package, PACKAGE_NAME);
  assert.equal(contract.payloads.length, 6);

  const packages = new Set();
  const selections = new Set();
  for (const payload of contract.payloads) {
    assert.deepEqual(Object.keys(payload).sort(), [
      "addon",
      "binding_repository",
      "binding_snapshot",
      "component",
      "cpu",
      "libc",
      "license",
      "os",
      "package",
      "target",
      "version",
    ]);
    assert.deepEqual(
      { os: payload.os, cpu: payload.cpu, libc: payload.libc },
      TARGETS.get(payload.target),
    );
    assert.equal(payload.component, "ait-node");
    assert.equal(payload.version, VERSION);
    assert.equal(payload.binding_repository, "ait-core");
    assert.equal(payload.binding_snapshot, CORE_SNAPSHOT);
    assert.equal(payload.license, "Apache-2.0");
    assert.equal(payload.addon, "native/ait_napi.node");
    assert.equal(
      payload.package,
      `@wa120/ait-native-${payload.os}-${payload.cpu}`,
    );
    assert.equal(packageJson.optionalDependencies[payload.package], VERSION);
    assert.equal(packages.has(payload.package), false);
    packages.add(payload.package);
    selections.add(payload.target);
  }
  assert.equal(packages.size, 6);
  assert.equal(selections.size, 6);
  assert.equal(Object.keys(packageJson.optionalDependencies).length, 6);
});

test("source build is locked to the restored ait-core binding", async () => {
  const manifest = await readFile(path.join(ROOT, "ait-external.toml"), "utf8");
  const lock = await readFile(path.join(ROOT, "ait-external.lock"), "utf8");

  for (const source of [manifest, lock]) {
    assert.match(source, /repository_index = 0/);
    assert.match(source, new RegExp(`snapshot = "${CORE_SNAPSHOT}"`));
    assert.match(source, /path = "rust\/crates\/ait-napi"/);
    assert.match(source, /package = "ait-napi"/);
  }
  const { layout, manifestPath } = await resolvedCoreSource();
  assert.match(layout, /^(?:materialized-external|public-monorepo)$/);
  const addonManifest = await lstat(manifestPath);
  assert.equal(addonManifest.isFile(), true);

  const localAddon = await lstat(path.join(ROOT, "native", "ait_napi.node"));
  assert.equal(localAddon.isFile(), true);
  assert.equal(localAddon.isSymbolicLink(), false);
  assert.ok(localAddon.size > 0);
});

test("published runtime loads an addon directly and has no process relay", async () => {
  const runtime = await readFile(path.join(ROOT, "src", "runtime.js"), "utf8");
  const agent = await readFile(path.join(ROOT, "src", "agent.js"), "utf8");
  const command = await readFile(path.join(ROOT, "bin", "ait.mjs"), "utf8");
  const source = `${runtime}\n${agent}\n${command}`;

  assert.match(runtime, /require\(addonPath\)/);
  assert.match(runtime, /process\.platform/);
  assert.match(runtime, /process\.arch/);
  assert.match(runtime, /process_transport_allowed/);
  assert.doesNotMatch(source, /node:child_process/);
  assert.doesNotMatch(source, /\bspawn(?:Sync)?\s*\(/);
  assert.doesNotMatch(source, /\bexec(?:File|Sync)?\s*\(/);
  assert.doesNotMatch(source, /pyproject|composer\.json|pom\.xml|csproj|CMakeLists/);
  assert.doesNotMatch(source, /https?:\/\//);
});

test("native build validates the locked external and public monorepo layouts", async () => {
  const build = await readFile(path.join(ROOT, "scripts", "native-build.mjs"), "utf8");

  assert.match(build, /SNP-A25D351DE720/);
  assert.match(build, /\.ait-external-marker\.json/);
  assert.match(build, /ait-monorepo-source\.json/);
  assert.match(build, /ait-release-family\.json/);
  assert.match(build, /rust.*crates.*ait-napi.*Cargo\.toml/s);
  assert.match(build, /--locked/);
  assert.doesNotMatch(build, /language.*detect|project.*detect/i);
});

test("cross-platform CI uses one logical runner and builds the addon first", async () => {
  const catalog = await json("ci/patch_ci.json");
  assert.deepEqual(catalog.suites[0].runner.commands, ["./ci/run"]);
  assert.match(catalog.suites[0].purpose, /direct Node-API package/);

  const unix = await readFile(path.join(ROOT, "ci", "run.sh"), "utf8");
  const windows = await readFile(path.join(ROOT, "ci", "run.ps1"), "utf8");
  for (const source of [unix, windows]) {
    assert.match(source, /native:build/);
    assert.match(source, /1\.0\.0-rc\.12/);
    assert.match(source, /ait-external/);
    assert.doesNotMatch(source, /1\.0\.0-rc\.1(?!\d)/);
  }
});
