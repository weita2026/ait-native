import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGER = path.join(ROOT, "release", "npm-payload-package.mjs");
const LOCAL_ADDON = path.join(ROOT, "native", "ait_napi.node");
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";
const require = createRequire(import.meta.url);

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function run(command, args, cwd = ROOT) {
  return spawnSync(command, args, {
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
}

async function hostPayload() {
  const contract = JSON.parse(
    await readFile(path.join(ROOT, "lib", "npm-payload-contract.json"), "utf8"),
  );
  const payloads = contract.payloads.filter(
    (payload) => payload.os === process.platform && payload.cpu === process.arch,
  );
  assert.equal(payloads.length, 1);
  return payloads[0];
}

function packagerArgs(action, payload, addon, outputRoot) {
  return [
    PACKAGER,
    action,
    "--target",
    payload.target,
    "--version",
    payload.version,
    "--addon",
    addon,
    "--out-dir",
    outputRoot,
  ];
}

test("ait-node builds and packages one exact direct addon for its native target", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-addon-pack-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const payload = await hostPayload();
  const outputRoot = path.join(root, "output");

  const checked = run(
    process.execPath,
    packagerArgs("check", payload, LOCAL_ADDON, outputRoot),
  );
  assert.equal(checked.status, 0, checked.stderr);
  const checkResult = JSON.parse(checked.stdout);
  assert.equal(checkResult.package, `${payload.package}@${payload.version}`);
  assert.equal(checkResult.binding_repository, "ait-core");
  assert.equal(checkResult.binding_snapshot, payload.binding_snapshot);

  const built = run(
    process.execPath,
    packagerArgs("build", payload, LOCAL_ADDON, outputRoot),
  );
  assert.equal(built.status, 0, built.stderr);
  const buildResult = JSON.parse(built.stdout);
  assert.equal(buildResult.component, "ait-node");
  assert.equal(buildResult.target, payload.target);
  assert.equal(buildResult.source_sha256, sha256(await readFile(LOCAL_ADDON)));
  const tarballEntry = await lstat(buildResult.artifact);
  assert.equal(tarballEntry.isFile(), true);
  assert.equal(tarballEntry.isSymbolicLink(), false);

  const installRoot = path.join(root, "install");
  const installed = run(NPM, [
    "install",
    "--ignore-scripts",
    "--offline",
    "--no-audit",
    "--no-fund",
    "--no-save",
    "--prefix",
    installRoot,
    buildResult.artifact,
  ]);
  assert.equal(installed.status, 0, installed.stderr);
  const packageRoot = path.join(installRoot, "node_modules", payload.package);
  const packageJson = JSON.parse(
    await readFile(path.join(packageRoot, "package.json"), "utf8"),
  );
  assert.equal(packageJson.name, payload.package);
  assert.equal(packageJson.version, payload.version);
  assert.equal(packageJson.license, payload.license);
  assert.deepEqual(packageJson.os, [payload.os]);
  assert.deepEqual(packageJson.cpu, [payload.cpu]);
  assert.equal(packageJson.main, payload.addon);
  assert.equal(packageJson.bin, undefined);
  assert.equal(packageJson.dependencies, undefined);
  assert.deepEqual(packageJson.files, [
    "native",
    "provenance.json",
    "LICENSE",
    "NOTICE",
  ]);
  assert.equal(
    packageJson.aitNativeAddon.schema,
    "ait.node.napi-platform-addon/v1",
  );
  assert.equal(
    packageJson.aitNativeAddon.binding_snapshot,
    payload.binding_snapshot,
  );

  const provenance = JSON.parse(
    await readFile(path.join(packageRoot, "provenance.json"), "utf8"),
  );
  assert.equal(
    provenance.schema,
    "ait.node.napi-platform-addon-provenance/v1",
  );
  assert.equal(provenance.package_source_repository, "ait-node");
  assert.equal(provenance.binding_repository, "ait-core");
  assert.equal(provenance.binding_snapshot, payload.binding_snapshot);
  assert.equal(provenance.installed_path, payload.addon);
  assert.equal(provenance.license_file.path, "LICENSE");
  assert.equal(provenance.notice_file.path, "NOTICE");
  const installedBytes = await readFile(
    path.join(packageRoot, ...payload.addon.split("/")),
  );
  assert.equal(provenance.source_artifact.sha256, sha256(installedBytes));

  const addon = require(packageRoot);
  const info = JSON.parse(addon.bindingInfoJson());
  assert.equal(info.node_binding, "napi");
  assert.equal(info.process_transport_allowed, false);
});

test("packager rejects missing addon, drift, wrong target, and a non-addon", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-addon-reject-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const payload = await hostPayload();
  const outputRoot = path.join(root, "output");

  const missingAddonArgs = packagerArgs(
    "check",
    payload,
    LOCAL_ADDON,
    outputRoot,
  );
  missingAddonArgs.splice(missingAddonArgs.indexOf("--addon"), 2);
  const missingAddon = run(process.execPath, missingAddonArgs);
  assert.notEqual(missingAddon.status, 0);
  assert.match(missingAddon.stderr, /missing required npm addon package option --addon/);

  const wrongVersion = packagerArgs("check", payload, LOCAL_ADDON, outputRoot);
  wrongVersion[wrongVersion.indexOf("--version") + 1] = "1.0.0-rc.99";
  const drift = run(process.execPath, wrongVersion);
  assert.notEqual(drift.status, 0);
  assert.match(drift.stderr, /version .* does not match/);

  const foreignPayload = (
    await hostPayload()
  );
  const contract = JSON.parse(
    await readFile(path.join(ROOT, "lib", "npm-payload-contract.json"), "utf8"),
  );
  const other = contract.payloads.find(
    (entry) => entry.target !== foreignPayload.target,
  );
  const wrongTarget = run(
    process.execPath,
    packagerArgs("check", other, LOCAL_ADDON, outputRoot),
  );
  assert.notEqual(wrongTarget.status, 0);
  assert.match(wrongTarget.stderr, /requires native host/);

  const textPath = path.join(root, "not-an-addon.node");
  await writeFile(textPath, "not a native addon\n");
  const nonAddon = run(
    process.execPath,
    packagerArgs("check", payload, textPath, outputRoot),
  );
  assert.notEqual(nonAddon.status, 0);
  assert.match(nonAddon.stderr, /cannot be loaded/);
});

test("packager rejects a symlinked built addon", { skip: process.platform === "win32" }, async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-addon-link-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const payload = await hostPayload();
  const linkedAddon = path.join(root, "linked.node");
  await mkdir(root, { recursive: true });
  await import("node:fs/promises").then(({ symlink }) => symlink(LOCAL_ADDON, linkedAddon));
  const linked = run(
    process.execPath,
    packagerArgs("check", payload, linkedAddon, path.join(root, "output")),
  );
  assert.notEqual(linked.status, 0);
  assert.match(linked.stderr, /regular non-symlink file/);
});
