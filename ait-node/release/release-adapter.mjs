#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createFixturePayloadTarballs } from "../scripts/fixture-payloads.mjs";
import { hostTarget, nativeBuild, TARGETS } from "../scripts/native-build.mjs";
import { spawnNpmSync } from "../scripts/npm-command.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGE_NAME = "ait-native";
const PACKAGE_VERSION = "1.0.0-rc.2";
const PORTABLE_TARGET = "portable";
const TARBALL_NAME = `${PACKAGE_NAME}-${PACKAGE_VERSION}.tgz`;
const TARBALL_PATH = path.join(ROOT, "dist", TARBALL_NAME);
const ADDON_OUTPUT_ROOT = path.join(ROOT, "dist", "npm-addons");

function run(command, args, cwd = ROOT) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    env: {
      ...process.env,
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_update_notifier: "false",
    },
    windowsHide: true,
  });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

function runNpm(args, cwd = ROOT) {
  const result = spawnNpmSync(args, {
    cwd,
    encoding: "utf8",
    env: {
      ...process.env,
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_update_notifier: "false",
    },
    windowsHide: true,
  });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `npm ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

async function readJson(filePath) {
  return JSON.parse(await readFile(filePath, "utf8"));
}

function targetArtifact(payload) {
  return path.join(
    ADDON_OUTPUT_ROOT,
    `${payload.package}-${payload.version}.tgz`,
  );
}

async function validateContract(target, version) {
  assert.equal(version, PACKAGE_VERSION, "release version drift");
  assert.equal(
    target === PORTABLE_TARGET || TARGETS.has(target),
    true,
    `unsupported release target ${target}`,
  );

  const packageJson = await readJson(path.join(ROOT, "package.json"));
  const payloadContract = await readJson(
    path.join(ROOT, "lib", "npm-payload-contract.json"),
  );
  assert.equal(payloadContract.schema, "ait.node.napi-platform-packages/v1");
  assert.equal(payloadContract.family_version, version);
  assert.equal(payloadContract.top_level_package, PACKAGE_NAME);
  assert.equal(payloadContract.payloads.length, 6);

  const optionalDependencies = Object.fromEntries(
    payloadContract.payloads
      .map((payload) => [payload.package, payload.version])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  assert.equal(Object.keys(optionalDependencies).length, 6);
  assert.equal(
    payloadContract.payloads.every(
      (payload) =>
        payload.version === version &&
        payload.component === "ait-node" &&
        payload.binding_repository === "ait-core" &&
        payload.binding_snapshot === "SNP-4D8A3DA8FE1D" &&
        payload.addon === "native/ait_napi.node",
    ),
    true,
  );

  assert.equal(packageJson.name, PACKAGE_NAME);
  assert.equal(packageJson.version, version);
  assert.equal(packageJson.license, "Apache-2.0");
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
  assert.deepEqual(packageJson.optionalDependencies, optionalDependencies);
  assert.deepEqual(packageJson.files, [
    "bin/ait.mjs",
    "lib",
    "src",
    "LICENSE",
    "NOTICE",
  ]);
  for (const hook of ["preinstall", "install", "postinstall", "prepack"]) {
    assert.equal(packageJson.scripts?.[hook], undefined, `${hook} is forbidden`);
  }
  const selectedPayload =
    target === PORTABLE_TARGET
      ? null
      : payloadContract.payloads.find((payload) => payload.target === target);
  assert.equal(
    target === PORTABLE_TARGET || selectedPayload !== undefined,
    true,
    `missing addon contract for ${target}`,
  );
  return { packageJson, payloadContract, selectedPayload };
}

async function check(target, version) {
  await validateContract(target, version);
  const nativeTarget = target === PORTABLE_TARGET ? hostTarget() : target;
  await nativeBuild("build", nativeTarget);
  runNpm(["test"]);
  runNpm(["run", "check"]);
  return {
    action: "check",
    package: `${PACKAGE_NAME}@${PACKAGE_VERSION}`,
    payload_package_count: 6,
    runtime_transport: "direct-napi",
    status: "pass",
    target,
  };
}

async function buildPortable() {
  await mkdir(path.dirname(TARBALL_PATH), { recursive: true });
  await rm(TARBALL_PATH, { force: true });
  const stagingRoot = await mkdtemp(
    path.join(os.tmpdir(), "ait-node-envelope-"),
  );
  try {
    const packageRoot = path.join(stagingRoot, "package");
    await mkdir(packageRoot, { recursive: true });
    for (const entry of ["package.json", "LICENSE", "NOTICE", "bin", "lib", "src"]) {
      await cp(path.join(ROOT, entry), path.join(packageRoot, entry), {
        recursive: true,
      });
    }
    const readme = await readFile(path.join(ROOT, "release", "npm-readme.txt"));
    await writeFile(path.join(packageRoot, "README.md"), readme, { mode: 0o644 });

    const dryRun = runNpm([
      "pack",
      "--ignore-scripts",
      "--dry-run",
      "--json",
      packageRoot,
    ]);
    const inventory = JSON.parse(dryRun.stdout);
    assert.equal(inventory.length, 1);
    assert.equal(inventory[0].name, PACKAGE_NAME);
    assert.equal(inventory[0].version, PACKAGE_VERSION);
    const packedPaths = new Set(inventory[0].files.map((entry) => entry.path));
    const expectedPaths = new Set([
      "LICENSE",
      "NOTICE",
      "README.md",
      "bin/ait.mjs",
      "lib/npm-payload-contract.json",
      "package.json",
      "src/agent.js",
      "src/contract.js",
      "src/errors.js",
      "src/index.d.ts",
      "src/index.js",
      "src/runtime.js",
    ]);
    assert.deepEqual(packedPaths, expectedPaths);
    assert.equal(
      [...packedPaths].some(
        (entry) =>
          entry.endsWith(".node") ||
          entry.startsWith("native/") ||
          entry.startsWith("release/") ||
          entry.startsWith("scripts/") ||
          entry.startsWith("test/") ||
          entry.startsWith("ci/"),
      ),
      false,
      "portable npm envelope contains an implementation or release file",
    );

    const packed = runNpm([
      "pack",
      "--ignore-scripts",
      "--json",
      "--pack-destination",
      path.dirname(TARBALL_PATH),
      packageRoot,
    ]);
    const packedResult = JSON.parse(packed.stdout);
    assert.equal(packedResult.length, 1);
    assert.equal(packedResult[0].filename, TARBALL_NAME);
    const entry = await lstat(TARBALL_PATH);
    assert.equal(entry.isFile(), true);
    assert.equal(entry.isSymbolicLink(), false);
    const bytes = await readFile(TARBALL_PATH);
    return {
      action: "build",
      artifact: path.relative(ROOT, TARBALL_PATH),
      sha256: createHash("sha256").update(bytes).digest("hex"),
      size_bytes: bytes.length,
      status: "pass",
      target: PORTABLE_TARGET,
    };
  } finally {
    await rm(stagingRoot, { recursive: true, force: true });
  }
}

async function buildAddon(target, selectedPayload) {
  const built = await nativeBuild("build", target);
  const artifact = targetArtifact(selectedPayload);
  await rm(artifact, { force: true });
  const result = run(process.execPath, [
    path.join(ROOT, "release", "npm-payload-package.mjs"),
    "build",
    "--target",
    target,
    "--version",
    PACKAGE_VERSION,
    "--addon",
    built.addonPath,
    "--out-dir",
    ADDON_OUTPUT_ROOT,
  ]);
  const payload = JSON.parse(result.stdout);
  assert.equal(path.resolve(payload.artifact), artifact);
  return payload;
}

async function build(target, version) {
  const { selectedPayload } = await validateContract(target, version);
  return target === PORTABLE_TARGET
    ? buildPortable()
    : buildAddon(target, selectedPayload);
}

async function smokePortable(payloadContract) {
  const tarball = await lstat(TARBALL_PATH);
  assert.equal(tarball.isFile(), true);
  assert.equal(tarball.isSymbolicLink(), false);

  const temporaryRoot = await mkdtemp(
    path.join(os.tmpdir(), "ait-node-release-smoke-"),
  );
  try {
    const installRoot = path.join(temporaryRoot, "install");
    const fixtureRoot = path.join(temporaryRoot, "addon");
    const [fixture] = await createFixturePayloadTarballs(fixtureRoot);
    await mkdir(installRoot, { recursive: true });
    runNpm([
      "install",
      "--ignore-scripts",
      "--offline",
      "--no-audit",
      "--no-fund",
      "--no-save",
      "--prefix",
      installRoot,
      TARBALL_PATH,
      fixture.tarball,
    ]);

    const installedPackage = await readJson(
      path.join(installRoot, "node_modules", PACKAGE_NAME, "package.json"),
    );
    assert.equal(installedPackage.name, PACKAGE_NAME);
    assert.equal(installedPackage.version, PACKAGE_VERSION);
    assert.equal(installedPackage.license, "Apache-2.0");
    const installedPayload = await readJson(
      path.join(installRoot, "node_modules", fixture.package, "package.json"),
    );
    const contractPayload = payloadContract.payloads.find(
      (payload) => payload.package === fixture.package,
    );
    assert.equal(installedPayload.version, PACKAGE_VERSION);
    assert.equal(installedPayload.main, contractPayload.addon);

    const installedSmoke = run(process.execPath, [
      path.join(ROOT, "scripts", "installed-smoke.mjs"),
      installRoot,
    ]);
    const cli = run(process.execPath, [
      path.join(installRoot, "node_modules", PACKAGE_NAME, "bin", "ait.mjs"),
      "--version",
    ]);
    assert.equal(cli.stdout, `ait ${PACKAGE_VERSION}\n`);
    assert.equal(cli.stderr, "");
    return {
      action: "smoke",
      installed: `${PACKAGE_NAME}@${PACKAGE_VERSION}`,
      implementation_package: fixture.package,
      installed_smoke: JSON.parse(installedSmoke.stdout),
      command_stdout: cli.stdout.trim(),
      runtime_transport: "direct-napi",
      status: "pass",
      target: PORTABLE_TARGET,
    };
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function smokeAddon(target, payload) {
  const tarball = targetArtifact(payload);
  const entry = await lstat(tarball);
  assert.equal(entry.isFile(), true);
  assert.equal(entry.isSymbolicLink(), false);
  const temporaryRoot = await mkdtemp(
    path.join(os.tmpdir(), "ait-node-addon-smoke-"),
  );
  try {
    runNpm([
      "install",
      "--ignore-scripts",
      "--offline",
      "--no-audit",
      "--no-fund",
      "--no-save",
      "--prefix",
      temporaryRoot,
      tarball,
    ]);
    const packageRoot = path.join(temporaryRoot, "node_modules", payload.package);
    const addonSmoke = run(process.execPath, [
      "-e",
      [
        "const addon = require(process.argv[1]);",
        "process.stdout.write(JSON.stringify({",
        "  info: JSON.parse(addon.bindingInfoJson()),",
        "  run_cli_type: typeof addon.runCli,",
        "}));",
      ].join("\n"),
      packageRoot,
    ]);
    const { info, run_cli_type: runCliType } = JSON.parse(addonSmoke.stdout);
    assert.equal(info.contract, "ait.language.binding.v1");
    assert.equal(info.version, PACKAGE_VERSION);
    assert.equal(info.runtime_authority, "rust");
    assert.equal(info.node_binding, "napi");
    assert.equal(info.process_transport_allowed, false);
    assert.equal(runCliType, "function");
    return {
      action: "smoke",
      installed: `${payload.package}@${payload.version}`,
      binding_contract: info.contract,
      runtime_transport: "direct-napi",
      status: "pass",
      target,
    };
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function smoke(target, version) {
  const { payloadContract, selectedPayload } = await validateContract(
    target,
    version,
  );
  return target === PORTABLE_TARGET
    ? smokePortable(payloadContract)
    : smokeAddon(target, selectedPayload);
}

async function main() {
  const [action, target, version] = process.argv.slice(2);
  if (process.argv.length !== 5) {
    throw new Error(
      "usage: node release/release-adapter.mjs {check|build|smoke} <target|portable> 1.0.0-rc.2",
    );
  }
  const handlers = { build, check, smoke };
  assert.equal(Object.hasOwn(handlers, action), true, `unknown action ${action}`);
  const result = await handlers[action](target, version);
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error.message}\n`);
  process.exitCode = 1;
});
