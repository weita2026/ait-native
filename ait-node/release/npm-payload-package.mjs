#!/usr/bin/env node

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
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnNpmSync } from "../scripts/npm-command.mjs";
import { detectRuntimeLibc } from "../src/runtime.js";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CONTRACT_PATH = path.join(ROOT, "lib", "npm-payload-contract.json");
const PACKAGE_PATH = path.join(ROOT, "package.json");
const LICENSE_PATH = path.join(ROOT, "LICENSE");
const NOTICE_PATH = path.join(ROOT, "NOTICE");
const require = createRequire(import.meta.url);
const TARGETS = new Map([
  ["aarch64-apple-darwin", { os: "darwin", cpu: "arm64", libc: null }],
  ["x86_64-apple-darwin", { os: "darwin", cpu: "x64", libc: null }],
  ["aarch64-unknown-linux-gnu", { os: "linux", cpu: "arm64", libc: "glibc" }],
  ["x86_64-unknown-linux-gnu", { os: "linux", cpu: "x64", libc: "glibc" }],
  ["aarch64-pc-windows-msvc", { os: "win32", cpu: "arm64", libc: null }],
  ["x86_64-pc-windows-msvc", { os: "win32", cpu: "x64", libc: null }],
]);
const CONTRACT_KEYS = [
  "family_version",
  "payloads",
  "schema",
  "top_level_package",
];
const PAYLOAD_KEYS = [
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
];

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
  try {
    return JSON.parse(await readFile(filePath, "utf8"));
  } catch (error) {
    fail(`${label} is not valid JSON: ${error.message}`);
  }
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
    fail(`${label} must be a regular non-symlink file: ${filePath}`);
  }
  return entry;
}

export function validateContract(contract) {
  assertExactKeys(contract, CONTRACT_KEYS, "npm addon contract");
  if (
    contract.schema !== "ait.node.napi-platform-packages/v2" ||
    contract.top_level_package !== "@wa120/ait-native" ||
    contract.family_version !== "1.0.0" ||
    !Array.isArray(contract.payloads) ||
    contract.payloads.length !== 6
  ) {
    fail("npm addon contract identity is invalid");
  }
  const packages = new Set();
  const targets = new Set();
  for (const [index, payload] of contract.payloads.entries()) {
    const label = `npm addon contract row ${index}`;
    assertExactKeys(payload, PAYLOAD_KEYS, label);
    const platform = TARGETS.get(payload.target);
    if (platform === undefined) {
      fail(`${label} has an unsupported target`);
    }
    const expectedPackage = `@wa120/ait-native-${platform.os}-${platform.cpu}`;
    if (
      payload.os !== platform.os ||
      payload.cpu !== platform.cpu ||
      payload.libc !== platform.libc ||
      payload.component !== "ait-node" ||
      payload.package !== expectedPackage ||
      payload.version !== contract.family_version ||
      payload.binding_repository !== "ait-core" ||
      payload.binding_snapshot !== "SNP-43E84134DEC2" ||
      payload.license !== "Apache-2.0" ||
      payload.addon !== "native/ait_napi.node"
    ) {
      fail(`${label} does not match its direct Node-API mapping`);
    }
    if (packages.has(payload.package) || targets.has(payload.target)) {
      fail(`${label} duplicates a package or target`);
    }
    packages.add(payload.package);
    targets.add(payload.target);
  }
}

function validateSourcePackage(packageJson, contract) {
  assertExactKeys(
    packageJson.repository,
    ["directory", "type", "url"],
    "npm source repository",
  );
  if (
    packageJson.name !== contract.top_level_package ||
    packageJson.version !== contract.family_version ||
    packageJson.repository.type !== "git" ||
    packageJson.repository.url !==
      "git+https://github.com/weita2026/ait-native.git" ||
    packageJson.repository.directory !== "ait-node"
  ) {
    fail("npm source package identity is invalid");
  }
}

function npmTarballName(packageName, version) {
  const match = /^@([^/]+)\/([^/]+)$/.exec(packageName);
  if (match === null) {
    fail(`npm package name must be an exact scoped identity: ${packageName}`);
  }
  return `${match[1]}-${match[2]}-${version}.tgz`;
}

function parseOptions(argv) {
  const [action, ...tokens] = argv;
  if (action !== "check" && action !== "build") {
    fail(
      "usage: npm-payload-package.mjs {check|build} --target <triple> --version <version> --addon <path> [--out-dir <path>]",
    );
  }
  if (tokens.length % 2 !== 0) {
    fail("every npm addon package option requires one value");
  }
  const allowed = new Set(["--target", "--version", "--addon", "--out-dir"]);
  const options = {};
  for (let index = 0; index < tokens.length; index += 2) {
    const name = tokens[index];
    const value = tokens[index + 1];
    if (!allowed.has(name)) {
      fail(`unknown npm addon package option ${name}`);
    }
    if (Object.hasOwn(options, name)) {
      fail(`duplicate npm addon package option ${name}`);
    }
    if (value.length === 0) {
      fail(`npm addon package option ${name} cannot be empty`);
    }
    options[name] = value;
  }
  for (const required of ["--target", "--version", "--addon"]) {
    if (!Object.hasOwn(options, required)) {
      fail(`missing required npm addon package option ${required}`);
    }
  }
  return { action, options };
}

function selectedPayload(contract, target, version) {
  const matches = contract.payloads.filter((payload) => payload.target === target);
  if (matches.length !== 1) {
    fail(`npm addon contract does not select exactly one ${target} package`);
  }
  const payload = matches[0];
  if (version !== contract.family_version || version !== payload.version) {
    fail(`npm addon version ${version} does not match ${contract.family_version}`);
  }
  const hostLibc = detectRuntimeLibc();
  if (
    payload.os !== process.platform ||
    payload.cpu !== process.arch ||
    payload.libc !== hostLibc
  ) {
    fail(
      `target ${target} requires native host ${payload.os}/${payload.cpu}/${payload.libc ?? "none"}, got ${process.platform}/${process.arch}/${hostLibc ?? "none"}`,
    );
  }
  return payload;
}

function validateLoadedAddon(addonPath, payload) {
  let addon;
  try {
    addon = require(addonPath);
  } catch (error) {
    fail(`built addon cannot be loaded in the current Node.js process: ${error.message}`);
  }
  for (const name of [
    "bindingInfoJson",
    "agentWorkerCapabilitiesJson",
    "agentManagementJson",
    "agentWorkerTransactionJson",
    "runCli",
  ]) {
    if (typeof addon[name] !== "function") {
      fail(`built addon is missing required export ${name}`);
    }
  }
  let info;
  try {
    info = JSON.parse(addon.bindingInfoJson());
  } catch (error) {
    fail(`built addon bindingInfoJson is invalid: ${error.message}`);
  }
  if (
    info.contract !== "ait.language.binding.v1" ||
    info.version !== payload.version ||
    info.runtime_authority !== "rust" ||
    info.node_binding !== "napi" ||
    info.process_transport_allowed !== false
  ) {
    fail("built addon does not expose the exact direct Node-API contract");
  }
  return info;
}

async function validateInputs(contract, packageJson, options) {
  const payload = selectedPayload(
    contract,
    options["--target"],
    options["--version"],
  );
  const addonPath = path.resolve(options["--addon"]);
  const addonEntry = await regularFile(addonPath, "ait-node built addon");
  if (addonEntry.size === 0) {
    fail("ait-node built addon is empty");
  }
  const addonBytes = await readFile(addonPath);
  const bindingInfo = validateLoadedAddon(addonPath, payload);
  const licenseEntry = await regularFile(LICENSE_PATH, "ait-node license");
  const noticeEntry = await regularFile(NOTICE_PATH, "ait-node NOTICE");
  const licenseBytes = await readFile(LICENSE_PATH);
  const noticeBytes = await readFile(NOTICE_PATH);
  if (
    licenseEntry.size === 0 ||
    noticeEntry.size === 0 ||
    licenseBytes.length === 0 ||
    noticeBytes.length === 0
  ) {
    fail("ait-node license and NOTICE must be non-empty");
  }
  return {
    addonBytes,
    addonPath,
    bindingInfo,
    licenseBytes,
    noticeBytes,
    payload,
    repository: packageJson.repository,
  };
}

function runNpm(args) {
  const result = spawnNpmSync(args, {
    cwd: ROOT,
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
      `npm ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

async function buildPackage(inputs, outDir) {
  const {
    addonBytes,
    addonPath,
    licenseBytes,
    noticeBytes,
    payload,
    repository,
  } = inputs;
  const outputRoot = path.resolve(outDir);
  await mkdir(outputRoot, { recursive: true });
  const tarballName = npmTarballName(payload.package, payload.version);
  const tarballPath = path.join(outputRoot, tarballName);
  try {
    await lstat(tarballPath);
    fail(`refusing to overwrite existing npm addon tarball: ${tarballPath}`);
  } catch (error) {
    if (error?.code !== "ENOENT") {
      throw error;
    }
  }

  const temporaryRoot = await mkdtemp(path.join(outputRoot, ".ait-npm-addon-"));
  try {
    const packageRoot = path.join(temporaryRoot, "package");
    const installedAddon = path.join(packageRoot, ...payload.addon.split("/"));
    await mkdir(path.dirname(installedAddon), { recursive: true });
    await copyFile(addonPath, installedAddon);
    const copiedBytes = await readFile(installedAddon);
    if (sha256(copiedBytes) !== sha256(addonBytes)) {
      fail("copied npm Node addon drifted from the ait-node build output");
    }

    await writeFile(path.join(packageRoot, "LICENSE"), licenseBytes);
    await writeFile(path.join(packageRoot, "NOTICE"), noticeBytes);
    const packageJson = {
      name: payload.package,
      version: payload.version,
      description: `Implementation-only AIT Node-API addon for ${payload.target}`,
      license: payload.license,
      repository,
      os: [payload.os],
      cpu: [payload.cpu],
      ...(payload.libc === null ? {} : { libc: [payload.libc] }),
      main: payload.addon,
      files: ["native", "provenance.json", "LICENSE", "NOTICE"],
      aitNativeAddon: {
        schema: "ait.node.napi-platform-addon/v2",
        component: payload.component,
        target: payload.target,
        libc: payload.libc,
        addon: payload.addon,
        binding_repository: payload.binding_repository,
        binding_snapshot: payload.binding_snapshot,
      },
    };
    await writeFile(
      path.join(packageRoot, "package.json"),
      `${JSON.stringify(packageJson, null, 2)}\n`,
    );
    const provenance = {
      schema: "ait.node.napi-platform-addon-provenance/v2",
      family_version: payload.version,
      package: payload.package,
      target: payload.target,
      os: payload.os,
      cpu: payload.cpu,
      libc: payload.libc,
      component: payload.component,
      package_source_repository: "ait-node",
      binding_repository: payload.binding_repository,
      binding_snapshot: payload.binding_snapshot,
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
      source_artifact: {
        sha256: sha256(addonBytes),
        size_bytes: addonBytes.length,
      },
      installed_path: payload.addon,
    };
    await writeFile(
      path.join(packageRoot, "provenance.json"),
      `${JSON.stringify(provenance, null, 2)}\n`,
    );

    const expectedFiles = new Set([
      "LICENSE",
      "NOTICE",
      payload.addon,
      "package.json",
      "provenance.json",
    ]);
    const dryRun = runNpm([
      "pack",
      "--ignore-scripts",
      "--dry-run",
      "--json",
      packageRoot,
    ]);
    const inventory = JSON.parse(dryRun.stdout);
    const packedFiles = new Set(
      inventory[0]?.files?.map((entry) => entry.path) ?? [],
    );
    if (
      inventory.length !== 1 ||
      packedFiles.size !== expectedFiles.size ||
      [...expectedFiles].some((entry) => !packedFiles.has(entry))
    ) {
      fail("npm addon tarball inventory does not match the exact contract");
    }

    const packed = runNpm([
      "pack",
      "--ignore-scripts",
      "--json",
      "--pack-destination",
      outputRoot,
      packageRoot,
    ]);
    const packedResult = JSON.parse(packed.stdout);
    if (packedResult.length !== 1 || packedResult[0].filename !== tarballName) {
      fail("npm addon tarball filename drift");
    }
    const tarballEntry = await regularFile(tarballPath, "npm addon tarball");
    const tarballBytes = await readFile(tarballPath);
    return {
      action: "build",
      package: `${payload.package}@${payload.version}`,
      component: payload.component,
      target: payload.target,
      artifact: tarballPath,
      sha256: sha256(tarballBytes),
      size_bytes: tarballEntry.size,
      binding_repository: payload.binding_repository,
      binding_snapshot: payload.binding_snapshot,
      source_sha256: sha256(addonBytes),
      status: "pass",
    };
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function main() {
  const { action, options } = parseOptions(process.argv.slice(2));
  const contract = await readJson(CONTRACT_PATH, "npm addon contract");
  validateContract(contract);
  const packageJson = await readJson(PACKAGE_PATH, "npm source package");
  validateSourcePackage(packageJson, contract);
  const inputs = await validateInputs(contract, packageJson, options);
  if (action === "check") {
    process.stdout.write(
      `${JSON.stringify({
        action,
        package: `${inputs.payload.package}@${inputs.payload.version}`,
        component: inputs.payload.component,
        target: inputs.payload.target,
        binding_repository: inputs.payload.binding_repository,
        binding_snapshot: inputs.payload.binding_snapshot,
        source_sha256: sha256(inputs.addonBytes),
        status: "pass",
      })}\n`,
    );
    return;
  }
  const result = await buildPackage(
    inputs,
    options["--out-dir"] ?? path.join(ROOT, "dist", "npm-addons"),
  );
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error.message}\n`);
  process.exitCode = 1;
});
