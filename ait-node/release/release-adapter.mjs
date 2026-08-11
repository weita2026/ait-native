#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createFixturePayloadTarballs } from "../scripts/fixture-payloads.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGE_NAME = "ait-native";
const PACKAGE_VERSION = "1.0.0-rc.1";
const PORTABLE_TARGET = "portable";
const TARBALL_NAME = `${PACKAGE_NAME}-${PACKAGE_VERSION}.tgz`;
const TARBALL_PATH = path.join(ROOT, "dist", TARBALL_NAME);
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";

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

async function readJson(filePath) {
  return JSON.parse(await readFile(filePath, "utf8"));
}

async function validateContract(target, version) {
  assert.equal(target, PORTABLE_TARGET, "npm envelope selector must be portable");
  assert.equal(version, PACKAGE_VERSION, "release version drift");

  const packageJson = await readJson(path.join(ROOT, "package.json"));
  const payloadContract = await readJson(
    path.join(ROOT, "lib", "npm-payload-contract.json"),
  );
  assert.equal(payloadContract.schema, "ait.node.npm-platform-packages/v1");
  assert.equal(payloadContract.family_version, version);
  assert.equal(payloadContract.top_level_package, PACKAGE_NAME);
  assert.equal(payloadContract.payloads.length, 12);

  const optionalDependencies = Object.fromEntries(
    payloadContract.payloads
      .map((payload) => [payload.package, payload.version])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  assert.equal(Object.keys(optionalDependencies).length, 12);
  assert.equal(
    payloadContract.payloads.every(
      (payload) => payload.version === version,
    ),
    true,
  );

  assert.equal(packageJson.name, PACKAGE_NAME);
  assert.equal(packageJson.version, version);
  assert.equal(packageJson.license, "Apache-2.0");
  assert.deepEqual(packageJson.bin, {
    ait: "bin/ait.mjs",
    "ait-server": "bin/ait-server.mjs",
  });
  assert.deepEqual(packageJson.exports, {});
  assert.equal(packageJson.main, undefined);
  assert.equal(packageJson.types, undefined);
  assert.equal(packageJson.dependencies, undefined);
  assert.equal(packageJson.os, undefined);
  assert.equal(packageJson.cpu, undefined);
  assert.deepEqual(packageJson.optionalDependencies, optionalDependencies);
  assert.deepEqual(packageJson.files, ["bin", "lib", "LICENSE", "NOTICE"]);
  for (const hook of ["preinstall", "install", "postinstall", "prepack"]) {
    assert.equal(packageJson.scripts?.[hook], undefined, `${hook} is forbidden`);
  }
  await assert.rejects(lstat(path.join(ROOT, "native")), { code: "ENOENT" });

  return { packageJson, payloadContract };
}

async function check(target, version) {
  await validateContract(target, version);
  run(NPM, ["test"]);
  run(NPM, ["run", "check"]);
  return {
    action: "check",
    package: `${PACKAGE_NAME}@${PACKAGE_VERSION}`,
    payload_package_count: 12,
    status: "pass",
    target,
  };
}

async function build(target, version) {
  await validateContract(target, version);
  await mkdir(path.dirname(TARBALL_PATH), { recursive: true });
  await rm(TARBALL_PATH, { force: true });

  const dryRun = run(NPM, [
    "pack",
    "--ignore-scripts",
    "--dry-run",
    "--json",
    ".",
  ]);
  const inventory = JSON.parse(dryRun.stdout);
  assert.equal(inventory.length, 1);
  assert.equal(inventory[0].name, PACKAGE_NAME);
  assert.equal(inventory[0].version, PACKAGE_VERSION);
  const packedPaths = new Set(inventory[0].files.map((entry) => entry.path));
  const expectedPaths = new Set([
    "LICENSE",
    "NOTICE",
    "bin/ait-server.mjs",
    "bin/ait.mjs",
    "bin/launch.mjs",
    "lib/npm-payload-contract.json",
    "package.json",
  ]);
  assert.equal(packedPaths.size, expectedPaths.size);
  for (const required of expectedPaths) {
    assert.equal(packedPaths.has(required), true, `missing packed file ${required}`);
  }
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
    "npm envelope contains a forbidden implementation file",
  );

  const packed = run(NPM, [
    "pack",
    "--ignore-scripts",
    "--json",
    "--pack-destination",
    path.dirname(TARBALL_PATH),
    ".",
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
    target,
  };
}

async function smoke(target, version) {
  const { payloadContract } = await validateContract(target, version);
  const tarball = await lstat(TARBALL_PATH);
  assert.equal(tarball.isFile(), true);
  assert.equal(tarball.isSymbolicLink(), false);

  const temporaryRoot = await mkdtemp(
    path.join(os.tmpdir(), "ait-node-release-smoke-"),
  );
  try {
    const installRoot = path.join(temporaryRoot, "install");
    const fixtureRoot = path.join(temporaryRoot, "payloads");
    const fixtures = await createFixturePayloadTarballs(fixtureRoot);
    await mkdir(installRoot, { recursive: true });
    run(NPM, [
      "install",
      "--ignore-scripts",
      "--offline",
      "--omit=optional",
      "--no-audit",
      "--no-fund",
      "--no-save",
      "--prefix",
      installRoot,
      TARBALL_PATH,
      ...fixtures.map((fixture) => fixture.tarball),
    ]);

    const installedPackage = await readJson(
      path.join(installRoot, "node_modules", PACKAGE_NAME, "package.json"),
    );
    assert.equal(installedPackage.name, PACKAGE_NAME);
    assert.equal(installedPackage.version, PACKAGE_VERSION);
    assert.equal(installedPackage.license, "Apache-2.0");
    for (const fixture of fixtures) {
      const installedPayload = await readJson(
        path.join(
          installRoot,
          "node_modules",
          fixture.package,
          "package.json",
        ),
      );
      assert.equal(installedPayload.name, fixture.package);
      assert.equal(installedPayload.version, version);
      const contractPayload = payloadContract.payloads.find(
        (payload) => payload.package === fixture.package,
      );
      assert.equal(installedPayload.license, contractPayload.license);
    }

    const installedSmoke = run(process.execPath, [
      path.join(ROOT, "scripts", "installed-smoke.mjs"),
      installRoot,
    ]);
    return {
      action: "smoke",
      installed: `${PACKAGE_NAME}@${PACKAGE_VERSION}`,
      implementation_packages: fixtures
        .map((fixture) => fixture.package)
        .sort(),
      installed_smoke: JSON.parse(installedSmoke.stdout),
      status: "pass",
      target,
    };
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function main() {
  const [action, target, version] = process.argv.slice(2);
  if (process.argv.length !== 5) {
    throw new Error(
      "usage: node release/release-adapter.mjs {check|build|smoke} portable 1.0.0-rc.1",
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
