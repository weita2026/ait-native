#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";
const CONTRACT_PATH = path.join(ROOT, "lib", "npm-payload-contract.json");
const CONTRACT_KEYS = [
  "family_version",
  "payloads",
  "schema",
  "top_level_package",
];
const PAYLOAD_KEYS = [
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
];
const TARGETS = new Map([
  ["aarch64-apple-darwin", ["darwin", "arm64"]],
  ["x86_64-apple-darwin", ["darwin", "x64"]],
  ["aarch64-unknown-linux-gnu", ["linux", "arm64"]],
  ["x86_64-unknown-linux-gnu", ["linux", "x64"]],
  ["aarch64-pc-windows-msvc", ["win32", "arm64"]],
  ["x86_64-pc-windows-msvc", ["win32", "x64"]],
]);
const COMPONENTS = new Map([
  ["ait", ["ait", "ait-core", "Apache-2.0"]],
  ["ait-server", ["ait-server", "ait-server", "AGPL-3.0-only"]],
]);

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function assertExactKeys(value, keys, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail(`${label} fields must be exactly ${expected.join(", ")}`);
  }
}

async function readJson(filePath, label) {
  let value;
  try {
    value = JSON.parse(await readFile(filePath, "utf8"));
  } catch (error) {
    fail(`${label} is not valid JSON: ${error.message}`);
  }
  return value;
}

async function regularFile(filePath, label) {
  let entry;
  try {
    entry = await lstat(filePath);
  } catch (error) {
    if (error?.code === "ENOENT") {
      fail(`${label} is missing: ${filePath}`);
    }
    throw error;
  }
  if (!entry.isFile() || entry.isSymbolicLink()) {
    fail(`${label} must be a regular file: ${filePath}`);
  }
  return entry;
}

function validateContract(contract) {
  assertExactKeys(contract, CONTRACT_KEYS, "npm payload contract");
  if (contract.schema !== "ait.node.npm-platform-packages/v1") {
    fail("unsupported npm payload contract schema");
  }
  if (contract.top_level_package !== "ait-native") {
    fail("npm payload contract top-level package drift");
  }
  if (contract.family_version !== "1.0.0-rc.1") {
    fail("npm payload contract family version drift");
  }
  if (!Array.isArray(contract.payloads) || contract.payloads.length !== 12) {
    fail("npm payload contract must declare exactly twelve payload packages");
  }

  const packages = new Set();
  const selections = new Set();
  for (const [index, payload] of contract.payloads.entries()) {
    const label = `npm payload contract row ${index}`;
    assertExactKeys(payload, PAYLOAD_KEYS, label);
    const platform = TARGETS.get(payload.target);
    const component = COMPONENTS.get(payload.component);
    if (platform === undefined || component === undefined) {
      fail(`${label} has an unsupported target or component`);
    }
    const [expectedOs, expectedCpu] = platform;
    const [expectedCommand, expectedRepository, expectedLicense] = component;
    const packagePrefix = payload.component === "ait" ? "ait" : "server";
    const expectedPackage = `ait-native-${packagePrefix}-${expectedOs}-${expectedCpu}`;
    const expectedExecutable =
      expectedOs === "win32"
        ? `bin/${expectedCommand}.exe`
        : `bin/${expectedCommand}`;
    if (
      payload.os !== expectedOs ||
      payload.cpu !== expectedCpu ||
      payload.command !== expectedCommand ||
      payload.source_repository !== expectedRepository ||
      payload.license !== expectedLicense ||
      payload.package !== expectedPackage ||
      payload.executable !== expectedExecutable ||
      payload.version !== contract.family_version
    ) {
      fail(`${label} does not match its exact platform/component mapping`);
    }
    const selection = `${payload.component}\0${payload.target}`;
    if (packages.has(payload.package) || selections.has(selection)) {
      fail(`${label} duplicates a package or component/target selection`);
    }
    packages.add(payload.package);
    selections.add(selection);
  }
  for (const target of TARGETS.keys()) {
    for (const component of COMPONENTS.keys()) {
      if (!selections.has(`${component}\0${target}`)) {
        fail(`npm payload contract is missing ${component} for ${target}`);
      }
    }
  }
}

function parseOptions(argv) {
  const [action, ...tokens] = argv;
  if (action !== "check" && action !== "build") {
    fail(
      "usage: npm-payload-package.mjs {check|build} --component <id> --target <triple> --version <version> --receipt <path> --license <path> --notice <path> [--out-dir <path>]",
    );
  }
  if (tokens.length % 2 !== 0) {
    fail("every npm payload package option requires one value");
  }
  const allowed = new Set([
    "--component",
    "--target",
    "--version",
    "--receipt",
    "--license",
    "--notice",
    "--out-dir",
  ]);
  const options = {};
  for (let index = 0; index < tokens.length; index += 2) {
    const name = tokens[index];
    const value = tokens[index + 1];
    if (!allowed.has(name)) {
      fail(`unknown npm payload package option ${name}`);
    }
    if (Object.hasOwn(options, name)) {
      fail(`duplicate npm payload package option ${name}`);
    }
    if (value.length === 0) {
      fail(`npm payload package option ${name} cannot be empty`);
    }
    options[name] = value;
  }
  for (const required of [
    "--component",
    "--target",
    "--version",
    "--receipt",
    "--license",
    "--notice",
  ]) {
    if (!Object.hasOwn(options, required)) {
      fail(`missing required npm payload package option ${required}`);
    }
  }
  return { action, options };
}

function safeReceiptPath(receiptRoot, artifactPath) {
  if (
    typeof artifactPath !== "string" ||
    artifactPath.length === 0 ||
    artifactPath.includes("\\") ||
    path.posix.isAbsolute(artifactPath) ||
    path.posix.normalize(artifactPath) !== artifactPath ||
    artifactPath.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    fail(`receipt artifact path is unsafe: ${String(artifactPath)}`);
  }
  const resolved = path.resolve(receiptRoot, ...artifactPath.split("/"));
  if (!resolved.startsWith(`${receiptRoot}${path.sep}`)) {
    fail(`receipt artifact path escapes its bundle: ${artifactPath}`);
  }
  return resolved;
}

async function validateInputs(contract, options) {
  const component = options["--component"];
  const target = options["--target"];
  const version = options["--version"];
  const selection = contract.payloads.filter(
    (payload) => payload.component === component && payload.target === target,
  );
  if (selection.length !== 1) {
    fail(`npm payload contract does not select exactly one ${component}/${target} package`);
  }
  const payload = selection[0];
  if (version !== contract.family_version || version !== payload.version) {
    fail(`npm payload version ${version} does not match ${contract.family_version}`);
  }

  const receiptPath = path.resolve(options["--receipt"]);
  await regularFile(receiptPath, "component receipt");
  const receiptBytes = await readFile(receiptPath);
  const receipt = JSON.parse(receiptBytes.toString("utf8"));
  if (
    receipt.contract !== "ait.release.adapter.receipt/v1" ||
    receipt.repo_name !== payload.source_repository ||
    !/^SNP-[0-9A-F]{12}$/.test(receipt.snapshot_id ?? "") ||
    receipt.version !== version ||
    receipt.metadata?.package?.version !== version ||
    receipt.target !== target ||
    !Array.isArray(receipt.artifacts)
  ) {
    fail("component receipt identity, version, target, or contract is invalid");
  }
  const componentArtifacts = receipt.artifacts.filter(
    (artifact) =>
      artifact?.component === component &&
      artifact.role === "component-artifact",
  );
  if (
    componentArtifacts.length !== 1 ||
    componentArtifacts[0].ecosystem !== "native" ||
    componentArtifacts[0].kind !== "native-executable" ||
    componentArtifacts[0].target !== target
  ) {
    fail("component receipt must select exactly one matching native executable");
  }
  const artifact = componentArtifacts[0];
  if (
    !/^[0-9a-f]{64}$/.test(artifact.sha256 ?? "") ||
    !Number.isSafeInteger(artifact.size_bytes) ||
    artifact.size_bytes <= 0
  ) {
    fail("component receipt artifact digest or size is invalid");
  }
  const receiptRoot = path.dirname(receiptPath);
  const sourcePath = safeReceiptPath(receiptRoot, artifact.path);
  const sourceEntry = await regularFile(sourcePath, "receipt-owned executable");
  const realReceiptRoot = await realpath(receiptRoot);
  const realSourcePath = await realpath(sourcePath);
  if (!realSourcePath.startsWith(`${realReceiptRoot}${path.sep}`)) {
    fail("receipt-owned executable escapes its bundle through a symlink");
  }
  const sourceBytes = await readFile(sourcePath);
  if (
    sourceEntry.size !== artifact.size_bytes ||
    sourceBytes.length !== artifact.size_bytes ||
    sha256(sourceBytes) !== artifact.sha256
  ) {
    fail("receipt-owned executable size or SHA-256 drift");
  }

  const licensePath = path.resolve(options["--license"]);
  await regularFile(licensePath, "component license");
  const licenseBytes = await readFile(licensePath);
  if (licenseBytes.length === 0) {
    fail("component license cannot be empty");
  }
  const noticePath = path.resolve(options["--notice"]);
  await regularFile(noticePath, "component NOTICE");
  const noticeBytes = await readFile(noticePath);
  if (noticeBytes.length === 0) {
    fail("component NOTICE cannot be empty");
  }

  return {
    artifact,
    licenseBytes,
    noticeBytes,
    payload,
    receipt,
    receiptSha256: sha256(receiptBytes),
    sourceBytes,
    sourcePath,
  };
}

function run(command, args, cwd) {
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
    fail(
      `${command} ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

async function buildPackage(inputs, outDir) {
  const { artifact, licenseBytes, noticeBytes, payload, receipt, receiptSha256, sourcePath } = inputs;
  const outputRoot = path.resolve(outDir);
  await mkdir(outputRoot, { recursive: true });
  const tarballName = `${payload.package}-${payload.version}.tgz`;
  const tarballPath = path.join(outputRoot, tarballName);
  try {
    await lstat(tarballPath);
    fail(`refusing to overwrite existing npm payload tarball: ${tarballPath}`);
  } catch (error) {
    if (error?.code !== "ENOENT") {
      throw error;
    }
  }

  const temporaryRoot = await mkdtemp(path.join(outputRoot, ".ait-npm-payload-"));
  try {
    const packageRoot = path.join(temporaryRoot, "package");
    const executablePath = path.join(
      packageRoot,
      ...payload.executable.split("/"),
    );
    await mkdir(path.dirname(executablePath), { recursive: true });
    await copyFile(sourcePath, executablePath);
    if (payload.os !== "win32") {
      await chmod(executablePath, 0o755);
    }
    const copiedBytes = await readFile(executablePath);
    if (
      copiedBytes.length !== artifact.size_bytes ||
      sha256(copiedBytes) !== artifact.sha256
    ) {
      fail("copied npm payload executable drifted from its component receipt");
    }

    await writeFile(path.join(packageRoot, "LICENSE"), licenseBytes);
    await writeFile(path.join(packageRoot, "NOTICE"), noticeBytes);
    const packageJson = {
      name: payload.package,
      version: payload.version,
      description: `Implementation-only ${payload.component} payload for ${payload.target}`,
      license: payload.license,
      os: [payload.os],
      cpu: [payload.cpu],
      files: [
        "bin",
        "provenance.json",
        "LICENSE",
        "NOTICE",
      ],
      aitNativePayload: {
        schema: "ait.node.npm-platform-payload/v1",
        component: payload.component,
        target: payload.target,
        executable: payload.executable,
        source_repository: payload.source_repository,
        source_snapshot: receipt.snapshot_id,
      },
    };
    await writeFile(
      path.join(packageRoot, "package.json"),
      `${JSON.stringify(packageJson, null, 2)}\n`,
    );
    const provenance = {
      schema: "ait.node.npm-platform-payload-provenance/v1",
      family_version: payload.version,
      package: payload.package,
      target: payload.target,
      os: payload.os,
      cpu: payload.cpu,
      component: payload.component,
      source_repository: payload.source_repository,
      source_snapshot: receipt.snapshot_id,
      license: payload.license,
      license_file: {
        path: "LICENSE",
        sha256: sha256(licenseBytes),
        size_bytes: licenseBytes.length,
      },
      notice_file: {
        path: "NOTICE",
        sha256: sha256(noticeBytes),
        size_bytes: noticeBytes.length,
      },
      source_receipt: {
        contract: receipt.contract,
        sha256: receiptSha256,
      },
      source_artifact: {
        path: artifact.path,
        sha256: artifact.sha256,
        size_bytes: artifact.size_bytes,
      },
      installed_path: payload.executable,
    };
    await writeFile(
      path.join(packageRoot, "provenance.json"),
      `${JSON.stringify(provenance, null, 2)}\n`,
    );

    const expectedFiles = new Set([
      "LICENSE",
      payload.executable,
      "package.json",
      "provenance.json",
      "NOTICE",
    ]);
    const dryRun = run(
      NPM,
      ["pack", "--ignore-scripts", "--dry-run", "--json", packageRoot],
      ROOT,
    );
    const inventory = JSON.parse(dryRun.stdout);
    if (inventory.length !== 1) {
      fail("npm payload dry-run must return exactly one package");
    }
    const packedFiles = new Set(inventory[0].files.map((entry) => entry.path));
    if (
      packedFiles.size !== expectedFiles.size ||
      [...expectedFiles].some((entry) => !packedFiles.has(entry))
    ) {
      fail("npm payload tarball inventory does not match the exact contract");
    }

    const packed = run(
      NPM,
      [
        "pack",
        "--ignore-scripts",
        "--json",
        "--pack-destination",
        outputRoot,
        packageRoot,
      ],
      ROOT,
    );
    const packedResult = JSON.parse(packed.stdout);
    if (
      packedResult.length !== 1 ||
      packedResult[0].filename !== tarballName
    ) {
      fail("npm payload tarball filename drift");
    }
    const tarballEntry = await regularFile(tarballPath, "npm payload tarball");
    const tarballBytes = await readFile(tarballPath);
    return {
      action: "build",
      package: `${payload.package}@${payload.version}`,
      component: payload.component,
      target: payload.target,
      artifact: tarballPath,
      sha256: sha256(tarballBytes),
      size_bytes: tarballEntry.size,
      source_snapshot: receipt.snapshot_id,
      source_sha256: artifact.sha256,
      status: "pass",
    };
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function main() {
  const { action, options } = parseOptions(process.argv.slice(2));
  const contract = await readJson(CONTRACT_PATH, "npm payload contract");
  validateContract(contract);
  const inputs = await validateInputs(contract, options);
  if (action === "check") {
    process.stdout.write(
      `${JSON.stringify({
        action,
        package: `${inputs.payload.package}@${inputs.payload.version}`,
        component: inputs.payload.component,
        target: inputs.payload.target,
        source_snapshot: inputs.receipt.snapshot_id,
        source_sha256: inputs.artifact.sha256,
        status: "pass",
      })}\n`,
    );
    return;
  }
  const result = await buildPackage(
    inputs,
    options["--out-dir"] ?? path.join(ROOT, "dist", "npm-payloads"),
  );
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error.message}\n`);
  process.exitCode = 1;
});
