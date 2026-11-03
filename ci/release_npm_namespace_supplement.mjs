#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";

const VERSION = "1.0.0-rc.3";
const RELEASE_ID = "REL-FAM-600EFDC327FE7860";
const RELEASE_TAG = "v1.0.0-rc.3";
const TAG_OBJECT = "810265c705ffececba3d74924f60ed2d0453ef7d";
const SOURCE_COMMIT = "ba368cf4d0750035345f14a8a91c22fb9e450260";
const NODE_SNAPSHOT = "SNP-22993C1FEF52";
const CORE_SNAPSHOT = "SNP-158C9C5BB3D7";
const TOP_PACKAGE = "@wa120/ait-native";
const REPOSITORY = Object.freeze({
  type: "git",
  url: "git+https://github.com/weita2026/ait-native.git",
  directory: "ait-node",
});
const PLATFORM_ROWS = Object.freeze([
  ["aarch64-apple-darwin", "darwin", "arm64"],
  ["x86_64-apple-darwin", "darwin", "x64"],
  ["aarch64-unknown-linux-gnu", "linux", "arm64"],
  ["x86_64-unknown-linux-gnu", "linux", "x64"],
  ["aarch64-pc-windows-msvc", "win32", "arm64"],
  ["x86_64-pc-windows-msvc", "win32", "x64"],
]);
const TOP_SOURCE_PATHS = Object.freeze([
  "LICENSE",
  "NOTICE",
  "bin/ait.mjs",
  "lib/npm-payload-contract.json",
  "package.json",
  "release/npm-readme.txt",
  "release/release-adapter.mjs",
  "scripts/fixture-payloads.mjs",
  "scripts/installed-smoke.mjs",
  "scripts/native-build.mjs",
  "scripts/npm-command.mjs",
  "src/agent.js",
  "src/contract.js",
  "src/errors.js",
  "src/index.d.ts",
  "src/index.js",
  "src/runtime.js",
]);
const ADDON_ARCHIVE_PATHS = Object.freeze([
  "package/LICENSE",
  "package/NOTICE",
  "package/native/ait_napi.node",
  "package/package.json",
  "package/provenance.json",
]);
const TOP_ARCHIVE_PATHS = Object.freeze([
  "package/LICENSE",
  "package/NOTICE",
  "package/README.md",
  "package/bin/ait.mjs",
  "package/lib/npm-payload-contract.json",
  "package/package.json",
  "package/src/agent.js",
  "package/src/contract.js",
  "package/src/errors.js",
  "package/src/index.d.ts",
  "package/src/index.js",
  "package/src/runtime.js",
]);

function fail(message) {
  throw new Error(message);
}

function sha(bytes, algorithm = "sha256") {
  return createHash(algorithm).update(bytes).digest("hex");
}

function integrity(bytes) {
  return `sha512-${createHash("sha512").update(bytes).digest("base64")}`;
}

function exactKeys(value, keys, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  assert.deepEqual(
    Object.keys(value).sort(),
    [...keys].sort(),
    `${label} fields drifted`,
  );
}

function exactArray(actual, expected, label) {
  assert.deepEqual(actual, expected, `${label} drifted`);
}

function isSha256(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function isPositiveInteger(value) {
  return Number.isSafeInteger(value) && value > 0;
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

async function realDirectory(directory, label) {
  if (!path.isAbsolute(directory)) {
    fail(`${label} must be an absolute path: ${directory}`);
  }
  let entry;
  try {
    entry = await lstat(directory);
  } catch (error) {
    if (error?.code === "ENOENT") {
      fail(`${label} is missing: ${directory}`);
    }
    throw error;
  }
  if (!entry.isDirectory() || entry.isSymbolicLink()) {
    fail(`${label} must be a real directory: ${directory}`);
  }
  return realpath(directory);
}

async function readJson(filePath, label) {
  await regularFile(filePath, label);
  try {
    return JSON.parse(await readFile(filePath, "utf8"));
  } catch (error) {
    fail(`${label} must contain valid JSON: ${error.message}`);
  }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    windowsHide: true,
    ...options,
    env: {
      ...process.env,
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_update_notifier: "false",
      ...options.env,
    },
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

function npmTarballName(packageName, version) {
  const match = /^@([^/]+)\/([^/]+)$/.exec(packageName);
  if (match === null) {
    fail(`npm package name must be scoped: ${packageName}`);
  }
  return `${match[1]}-${match[2]}-${version}.tgz`;
}

function validateConfig(config) {
  exactKeys(
    config,
    [
      "addons",
      "build_toolchain",
      "contract",
      "legal_material",
      "mutation",
      "node_source",
      "original_envelope",
      "publisher",
      "registry",
      "release",
    ],
    "namespace supplement configuration",
  );
  assert.equal(config.contract, "ait.release.npm-namespace-supplement/v1");
  assert.deepEqual(config.release, {
    id: RELEASE_ID,
    version: VERSION,
    tag: RELEASE_TAG,
    tag_object: TAG_OBJECT,
    source_commit: SOURCE_COMMIT,
    github_release_id: 369674917,
    source_dossier_run_id: 31664713921,
    source_dossier_artifact_id: 9167933771,
    source_dossier_artifact_digest:
      "sha256:08afc391688c902f3c2259392286b51612e6b6eb0aa51c388e8e513329705823",
    protected_authorization_run_id: 31666479359,
    protected_authorization_artifact_id: 9168120753,
    protected_authorization_artifact_digest:
      "sha256:54079d53bc3e115f314d99591228cc80dbab4c56c5ae361530f9c490c0764be9",
    protected_authorization_evidence_sha256:
      "cc18cf39db59147d5ee94359f0c00813be6841bf317686451eafd1152f870b32",
    failed_endpoint_run_id: 31668411148,
    failed_endpoint_workflow_commit:
      "30672445b7321226f81db280f3e2531ad6fc2a5d",
  });
  assert.equal(config.node_source.repository, "ait-node");
  assert.equal(config.node_source.snapshot, NODE_SNAPSHOT);
  assert.equal(
    config.node_source.snapshot_manifest_hash,
    "22993c1fef528a53c1e3815956e3fc082197ca4700f64ac2051da32598c7c21c",
  );
  assert.equal(config.node_source.snapshot_created_at_s, 1786598582);
  assert.equal(config.node_source.binding_repository, "ait-core");
  assert.equal(config.node_source.binding_snapshot, CORE_SNAPSHOT);
  exactKeys(
    config.node_source.files,
    TOP_SOURCE_PATHS,
    "ait-node source hash inventory",
  );
  for (const [file, digest] of Object.entries(config.node_source.files)) {
    if (!isSha256(digest)) {
      fail(`ait-node source digest is invalid: ${file}`);
    }
  }
  assert.deepEqual(config.publisher, {
    repository: "weita2026/ait-native",
    workflow: "npm-namespace-supplement.yml",
    environment: "pypi",
    npm_username: "wa120",
    credential_secret: "AIT_NPM_TOKEN",
  });
  assert.deepEqual(config.build_toolchain, {
    node: "22.17.1",
    npm: "10.9.2",
  });
  const expectedPackages = [
    TOP_PACKAGE,
    ...PLATFORM_ROWS.map(
      ([, os, cpu]) => `@wa120/ait-native-${os}-${cpu}`,
    ),
  ];
  assert.deepEqual(config.registry, {
    url: "https://registry.npmjs.org",
    dist_tag: "rc",
    top_level_package: TOP_PACKAGE,
    packages: expectedPackages,
  });
  assert.deepEqual(config.original_envelope, {
    package: "ait-native",
    filename: "ait-native-1.0.0-rc.3.tgz",
    github_release_asset_id: 512511134,
    size_bytes: 64813,
    sha256: "8862dc3621320fda30e6923c85eee872751bfc92d95f319382b5b690540392f8",
  });
  if (!Array.isArray(config.addons) || config.addons.length !== 6) {
    fail("namespace supplement must contain exactly six addon rows");
  }
  const seenSources = new Set();
  const seenPackages = new Set();
  for (const [index, row] of config.addons.entries()) {
    exactKeys(
      row,
      [
        "cpu",
        "filename",
        "native_sha256",
        "native_size_bytes",
        "os",
        "package",
        "source_filename",
        "source_github_release_asset_id",
        "source_package",
        "source_sha256",
        "source_size_bytes",
        "target",
      ],
      `addon row ${index}`,
    );
    const [target, os, cpu] = PLATFORM_ROWS[index];
    const sourcePackage = `ait-native-ait-${os}-${cpu}`;
    const packageName = `@wa120/ait-native-${os}-${cpu}`;
    assert.equal(row.target, target);
    assert.equal(row.os, os);
    assert.equal(row.cpu, cpu);
    assert.equal(row.source_package, sourcePackage);
    assert.equal(row.source_filename, `${sourcePackage}-${VERSION}.tgz`);
    assert.equal(row.package, packageName);
    assert.equal(row.filename, npmTarballName(packageName, VERSION));
    if (
      !isPositiveInteger(row.source_github_release_asset_id) ||
      !isPositiveInteger(row.source_size_bytes) ||
      !isPositiveInteger(row.native_size_bytes) ||
      !isSha256(row.source_sha256) ||
      !isSha256(row.native_sha256)
    ) {
      fail(`addon row ${index} has an invalid digest, size, or asset identity`);
    }
    if (seenSources.has(row.source_filename) || seenPackages.has(row.package)) {
      fail(`addon row ${index} repeats a source or scoped package`);
    }
    seenSources.add(row.source_filename);
    seenPackages.add(row.package);
  }
  assert.deepEqual(config.legal_material, {
    license_sha256:
      "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
    license_size_bytes: 11357,
    notice_sha256:
      "ba14cd6cfd2e17c4a9051c8a699bef0bac0060999de2d3b4a36a457dc41e780d",
    notice_size_bytes: 617320,
  });
  assert.deepEqual(config.mutation, {
    npm_tarball_rebuild: true,
    javascript_envelope_rebuild: true,
    package_metadata_rebuild: true,
    native_addon_rebuild: false,
    release_family_rebuild: false,
    tag_write: false,
    github_release_write: false,
    existing_unscoped_package_write: false,
  });
  return config;
}

async function verifyToolchain(config, fixture) {
  const npmVersion = run("npm", ["--version"]).stdout.trim();
  if (fixture) {
    return {
      node: process.version.replace(/^v/, ""),
      npm: npmVersion,
    };
  }
  assert.equal(process.version, `v${config.build_toolchain.node}`);
  assert.equal(npmVersion, config.build_toolchain.npm, "npm version drifted");
  return config.build_toolchain;
}

async function verifyNodeSource(config, nodeRoot) {
  const actualEntries = [];
  for (const relative of TOP_SOURCE_PATHS) {
    const source = path.join(nodeRoot, ...relative.split("/"));
    await regularFile(source, `ait-node source ${relative}`);
    const bytes = await readFile(source);
    assert.equal(
      sha(bytes),
      config.node_source.files[relative],
      `ait-node source digest drifted: ${relative}`,
    );
    actualEntries.push(relative);
  }
  exactArray(actualEntries, TOP_SOURCE_PATHS, "ait-node source inventory");

  const packageJson = await readJson(
    path.join(nodeRoot, "package.json"),
    "ait-node package",
  );
  assert.equal(packageJson.name, TOP_PACKAGE);
  assert.equal(packageJson.version, VERSION);
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
  const expectedDependencies = Object.fromEntries(
    config.addons.map((row) => [row.package, VERSION]),
  );
  assert.deepEqual(packageJson.optionalDependencies, expectedDependencies);
  exactArray(
    packageJson.files,
    ["bin/ait.mjs", "lib", "src", "LICENSE", "NOTICE"],
    "top-level npm files",
  );
  for (const hook of ["preinstall", "install", "postinstall", "prepack"]) {
    assert.equal(packageJson.scripts?.[hook], undefined, `${hook} is forbidden`);
  }

  const payload = await readJson(
    path.join(nodeRoot, "lib", "npm-payload-contract.json"),
    "ait-node payload contract",
  );
  assert.equal(payload.schema, "ait.node.napi-platform-packages/v1");
  assert.equal(payload.family_version, VERSION);
  assert.equal(payload.top_level_package, TOP_PACKAGE);
  assert.equal(payload.payloads?.length, 6);
  for (const [index, row] of config.addons.entries()) {
    assert.deepEqual(payload.payloads[index], {
      target: row.target,
      os: row.os,
      cpu: row.cpu,
      component: "ait-node",
      package: row.package,
      version: VERSION,
      binding_repository: "ait-core",
      binding_snapshot: CORE_SNAPSHOT,
      license: "Apache-2.0",
      addon: "native/ait_napi.node",
    });
  }
  return packageJson;
}

function listArchive(archive) {
  const listing = run("tar", ["-tzf", archive]).stdout
    .split(/\r?\n/)
    .filter((entry) => entry.length > 0);
  for (const entry of listing) {
    if (
      entry.startsWith("/") ||
      entry.includes("\\") ||
      entry.split("/").some((part) => part === "" || part === "." || part === "..")
    ) {
      fail(`npm archive contains an unsafe path: ${entry}`);
    }
  }
  return listing;
}

async function inspectArchive(archive, expectedPaths, temporaryRoot, label) {
  exactArray(listArchive(archive).sort(), [...expectedPaths].sort(), `${label} inventory`);
  const extractRoot = await mkdtemp(path.join(temporaryRoot, "extract-"));
  run("tar", ["-xzf", archive, "-C", extractRoot]);
  const visit = async (directory) => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const candidate = path.join(directory, entry.name);
      const metadata = await lstat(candidate);
      if (metadata.isSymbolicLink() || (!metadata.isDirectory() && !metadata.isFile())) {
        fail(`${label} contains a symlink or special member: ${candidate}`);
      }
      if (metadata.isDirectory()) {
        await visit(candidate);
      }
    }
  };
  await visit(extractRoot);
  return path.join(extractRoot, "package");
}

function expectedAddonPackage(row, packageName, includeRepository) {
  const value = {
    name: packageName,
    version: VERSION,
    description: `Implementation-only AIT Node-API addon for ${row.target}`,
    license: "Apache-2.0",
  };
  if (includeRepository) {
    value.repository = REPOSITORY;
  }
  return {
    ...value,
    os: [row.os],
    cpu: [row.cpu],
    main: "native/ait_napi.node",
    files: ["native", "provenance.json", "LICENSE", "NOTICE"],
    aitNativeAddon: {
      schema: "ait.node.napi-platform-addon/v1",
      component: "ait-node",
      target: row.target,
      addon: "native/ait_napi.node",
      binding_repository: "ait-core",
      binding_snapshot: CORE_SNAPSHOT,
    },
  };
}

function expectedAddonProvenance(config, row, packageName) {
  return {
    schema: "ait.node.napi-platform-addon-provenance/v1",
    family_version: VERSION,
    package: packageName,
    target: row.target,
    os: row.os,
    cpu: row.cpu,
    component: "ait-node",
    package_source_repository: "ait-node",
    binding_repository: "ait-core",
    binding_snapshot: CORE_SNAPSHOT,
    license: "Apache-2.0",
    license_file: {
      path: "LICENSE",
      sha256: config.legal_material.license_sha256,
      size_bytes: config.legal_material.license_size_bytes,
    },
    notice_file: {
      path: "NOTICE",
      sha256: config.legal_material.notice_sha256,
      size_bytes: config.legal_material.notice_size_bytes,
    },
    source_artifact: {
      sha256: row.native_sha256,
      size_bytes: row.native_size_bytes,
    },
    installed_path: "native/ait_napi.node",
  };
}

async function verifyBytes(filePath, expectedDigest, expectedSize, label) {
  const entry = await regularFile(filePath, label);
  const bytes = await readFile(filePath);
  assert.equal(entry.size, expectedSize, `${label} size drifted`);
  assert.equal(sha(bytes), expectedDigest, `${label} digest drifted`);
  return bytes;
}

async function packDirectory(packageRoot, outputRoot) {
  const result = run(
    "npm",
    [
      "pack",
      "--ignore-scripts",
      "--json",
      "--pack-destination",
      outputRoot,
      packageRoot,
    ],
    { cwd: packageRoot },
  );
  let packed;
  try {
    packed = JSON.parse(result.stdout);
  } catch (error) {
    fail(`npm pack did not return JSON: ${error.message}`);
  }
  if (packed.length !== 1 || typeof packed[0]?.filename !== "string") {
    fail("npm pack did not return exactly one archive");
  }
  return path.join(outputRoot, packed[0].filename);
}

async function prepareAddon(config, row, sourceRoot, outputRoot, temporaryRoot) {
  const sourceArchive = path.join(sourceRoot, row.source_filename);
  await verifyBytes(
    sourceArchive,
    row.source_sha256,
    row.source_size_bytes,
    `original addon archive ${row.source_filename}`,
  );
  const originalRoot = await inspectArchive(
    sourceArchive,
    ADDON_ARCHIVE_PATHS,
    temporaryRoot,
    `original addon ${row.source_package}`,
  );
  assert.deepEqual(
    await readJson(path.join(originalRoot, "package.json"), "original addon package"),
    expectedAddonPackage(row, row.source_package, false),
    `original addon metadata drifted: ${row.source_package}`,
  );
  assert.deepEqual(
    await readJson(path.join(originalRoot, "provenance.json"), "original addon provenance"),
    expectedAddonProvenance(config, row, row.source_package),
    `original addon provenance drifted: ${row.source_package}`,
  );
  const licenseBytes = await verifyBytes(
    path.join(originalRoot, "LICENSE"),
    config.legal_material.license_sha256,
    config.legal_material.license_size_bytes,
    "original addon LICENSE",
  );
  const noticeBytes = await verifyBytes(
    path.join(originalRoot, "NOTICE"),
    config.legal_material.notice_sha256,
    config.legal_material.notice_size_bytes,
    "original addon NOTICE",
  );
  const nativeBytes = await verifyBytes(
    path.join(originalRoot, "native", "ait_napi.node"),
    row.native_sha256,
    row.native_size_bytes,
    "original native Node-API addon",
  );

  const stageRoot = await mkdtemp(path.join(temporaryRoot, "repack-"));
  const packageRoot = path.join(stageRoot, "package");
  await mkdir(path.join(packageRoot, "native"), { recursive: true });
  await writeFile(path.join(packageRoot, "LICENSE"), licenseBytes);
  await writeFile(path.join(packageRoot, "NOTICE"), noticeBytes);
  await writeFile(path.join(packageRoot, "native", "ait_napi.node"), nativeBytes);
  await writeFile(
    path.join(packageRoot, "package.json"),
    `${JSON.stringify(expectedAddonPackage(row, row.package, true), null, 2)}\n`,
  );
  await writeFile(
    path.join(packageRoot, "provenance.json"),
    `${JSON.stringify(expectedAddonProvenance(config, row, row.package), null, 2)}\n`,
  );
  const archive = await packDirectory(packageRoot, outputRoot);
  assert.equal(path.basename(archive), row.filename, "scoped addon filename drifted");
  const repackedRoot = await inspectArchive(
    archive,
    ADDON_ARCHIVE_PATHS,
    temporaryRoot,
    `scoped addon ${row.package}`,
  );
  assert.deepEqual(
    await readJson(path.join(repackedRoot, "package.json"), "scoped addon package"),
    expectedAddonPackage(row, row.package, true),
  );
  assert.deepEqual(
    await readJson(path.join(repackedRoot, "provenance.json"), "scoped addon provenance"),
    expectedAddonProvenance(config, row, row.package),
  );
  const repackedNative = await verifyBytes(
    path.join(repackedRoot, "native", "ait_napi.node"),
    row.native_sha256,
    row.native_size_bytes,
    "repacked native Node-API addon",
  );
  assert.deepEqual(repackedNative, nativeBytes, "native Node-API bytes changed during rename");
  await verifyBytes(
    path.join(repackedRoot, "LICENSE"),
    config.legal_material.license_sha256,
    config.legal_material.license_size_bytes,
    "repacked addon LICENSE",
  );
  await verifyBytes(
    path.join(repackedRoot, "NOTICE"),
    config.legal_material.notice_sha256,
    config.legal_material.notice_size_bytes,
    "repacked addon NOTICE",
  );
  return archive;
}

async function prepareEnvelope(config, nodeRoot, outputRoot, temporaryRoot) {
  const packageRoot = path.join(await mkdtemp(path.join(temporaryRoot, "envelope-")), "package");
  await mkdir(packageRoot, { recursive: true });
  for (const entry of ["package.json", "LICENSE", "NOTICE", "bin", "lib", "src"]) {
    await cp(path.join(nodeRoot, entry), path.join(packageRoot, entry), {
      recursive: true,
    });
  }
  await copyFile(
    path.join(nodeRoot, "release", "npm-readme.txt"),
    path.join(packageRoot, "README.md"),
  );
  const archive = await packDirectory(packageRoot, outputRoot);
  const expectedName = npmTarballName(TOP_PACKAGE, VERSION);
  assert.equal(path.basename(archive), expectedName, "scoped envelope filename drifted");
  const unpacked = await inspectArchive(
    archive,
    TOP_ARCHIVE_PATHS,
    temporaryRoot,
    "scoped top-level envelope",
  );
  const metadata = await readJson(path.join(unpacked, "package.json"), "scoped envelope package");
  assert.equal(metadata.name, TOP_PACKAGE);
  assert.equal(metadata.version, VERSION);
  assert.deepEqual(metadata.repository, REPOSITORY);
  assert.equal(metadata.dependencies, undefined);
  assert.equal(metadata.main, undefined);
  for (const hook of ["preinstall", "install", "postinstall", "prepack"]) {
    assert.equal(metadata.scripts?.[hook], undefined, `${hook} is forbidden`);
  }
  const nativeEntries = listArchive(archive).filter(
    (entry) => entry.endsWith(".node") || entry.includes("/native/"),
  );
  assert.deepEqual(nativeEntries, [], "top-level envelope contains native bytes");
  return archive;
}

async function packageReceipt(packageName, archive, order, target = null) {
  const bytes = await readFile(archive);
  const entry = await regularFile(archive, `staged npm package ${packageName}`);
  return {
    order,
    package: packageName,
    version: VERSION,
    target,
    filename: path.basename(archive),
    sha256: sha(bytes),
    sha1: sha(bytes, "sha1"),
    integrity: integrity(bytes),
    size_bytes: entry.size,
  };
}

async function prepare(
  configPath,
  nodeRootInput,
  sourceRootInput,
  outputRootInput,
  fixture,
) {
  const config = validateConfig(await readJson(configPath, "namespace supplement configuration"));
  const actualToolchain = await verifyToolchain(config, fixture);
  const nodeRoot = await realDirectory(nodeRootInput, "ait-node source root");
  const sourceRoot = await realDirectory(sourceRootInput, "original npm asset root");
  const outputRoot = path.resolve(outputRootInput);
  if (!path.isAbsolute(outputRootInput)) {
    fail(`namespace supplement output must be absolute: ${outputRootInput}`);
  }
  try {
    await lstat(outputRoot);
    fail(`namespace supplement output must not exist: ${outputRoot}`);
  } catch (error) {
    if (error?.code !== "ENOENT") {
      throw error;
    }
  }
  const outputParent = await realDirectory(path.dirname(outputRoot), "namespace supplement output parent");
  const configBytes = await readFile(configPath);
  await verifyNodeSource(config, nodeRoot);

  const expectedSources = config.addons.map((row) => row.source_filename).sort();
  const actualSources = (await readdir(sourceRoot, { withFileTypes: true }))
    .map((entry) => {
      if (!entry.isFile() || entry.isSymbolicLink()) {
        fail(`original npm asset root contains a non-file: ${entry.name}`);
      }
      return entry.name;
    })
    .sort();
  exactArray(actualSources, expectedSources, "original npm asset inventory");

  const temporaryRoot = await mkdtemp(path.join(outputParent, ".ait-npm-supplement-"));
  const staging = path.join(temporaryRoot, "stage");
  const packagesRoot = path.join(staging, "packages");
  await mkdir(packagesRoot, { recursive: true });
  let completed = false;
  try {
    const packageRows = [];
    const mappings = [];
    for (const [index, row] of config.addons.entries()) {
      const archive = await prepareAddon(
        config,
        row,
        sourceRoot,
        packagesRoot,
        temporaryRoot,
      );
      const receipt = await packageReceipt(row.package, archive, index + 1, row.target);
      packageRows.push(receipt);
      mappings.push({
        order: index + 1,
        target: row.target,
        source_package: `${row.source_package}@${VERSION}`,
        source_filename: row.source_filename,
        source_github_release_asset_id: row.source_github_release_asset_id,
        source_sha256: row.source_sha256,
        scoped_package: `${row.package}@${VERSION}`,
        scoped_filename: receipt.filename,
        scoped_sha256: receipt.sha256,
        native_sha256: row.native_sha256,
        native_size_bytes: row.native_size_bytes,
        native_bytes_identical: true,
      });
    }
    const envelope = await prepareEnvelope(config, nodeRoot, packagesRoot, temporaryRoot);
    packageRows.push(await packageReceipt(TOP_PACKAGE, envelope, 7));

    const checksumRows = [...packageRows]
      .sort((left, right) => left.filename.localeCompare(right.filename))
      .map((row) => `${row.sha256}  ${row.filename}`);
    await writeFile(
      path.join(staging, "SHA256SUMS"),
      `${checksumRows.join("\n")}\n`,
    );
    const receipt = {
      contract: fixture
        ? "ait.release.npm-namespace-supplement.fixture-stage/v1"
        : "ait.release.npm-namespace-supplement.stage/v1",
      status: fixture
        ? "test_fixture_only"
        : "ready_for_authenticated_npm_preflight",
      release: config.release,
      node_source: {
        repository: config.node_source.repository,
        snapshot: config.node_source.snapshot,
        snapshot_manifest_hash: config.node_source.snapshot_manifest_hash,
        snapshot_created_at_s: config.node_source.snapshot_created_at_s,
        binding_repository: config.node_source.binding_repository,
        binding_snapshot: config.node_source.binding_snapshot,
      },
      publisher: config.publisher,
      toolchain: actualToolchain,
      config_sha256: sha(configBytes),
      packages: packageRows,
      addon_mappings: mappings,
      mutation: config.mutation,
    };
    await writeFile(
      path.join(staging, "ait-release.npm-namespace-supplement.json"),
      `${JSON.stringify(receipt, null, 2)}\n`,
    );
    await rename(staging, outputRoot);
    completed = true;
    process.stdout.write(
      `${JSON.stringify({
        contract: receipt.contract,
        release_id: RELEASE_ID,
        version: VERSION,
        package_count: packageRows.length,
        native_addon_rebuild: false,
        output: outputRoot,
        status: "pass",
      })}\n`,
    );
  } finally {
    const expectedPrefix = `${outputParent}${path.sep}.ait-npm-supplement-`;
    if (!temporaryRoot.startsWith(expectedPrefix)) {
      fail(`refusing to clean unexpected temporary path: ${temporaryRoot}`);
    }
    await rm(temporaryRoot, { recursive: true, force: true });
    if (!completed) {
      await rm(outputRoot, { recursive: true, force: true });
    }
  }
}

async function main() {
  const [action, configPath, nodeRoot, sourceRoot, outputRoot] = process.argv.slice(2);
  if (
    (action !== "prepare" && action !== "prepare-fixture") ||
    [configPath, nodeRoot, sourceRoot, outputRoot].some((value) => value === undefined) ||
    process.argv.length !== 7
  ) {
    fail(
      "usage: release_npm_namespace_supplement.mjs {prepare|prepare-fixture} <config> <ait-node-root> <original-assets-root> <new-output-root>",
    );
  }
  await prepare(
    configPath,
    nodeRoot,
    sourceRoot,
    outputRoot,
    action === "prepare-fixture",
  );
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error.message}\n`);
  process.exitCode = 1;
});
