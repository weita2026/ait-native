#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  copyFile,
  lstat,
  mkdir,
  readFile,
  realpath,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CORE_SNAPSHOT = "SNP-F136DB9A342B";
const INTERNAL_CORE_ROOT = path.join(ROOT, ".ait-external", "ait-core");
const PUBLIC_CORE_ROOT = path.resolve(ROOT, "..", "ait-core");
const MANIFEST_RELATIVE = path.join("rust", "crates", "ait-napi", "Cargo.toml");
const TARGET_ROOT = path.join(ROOT, ".ait-native-target");
const INSTALLED_ADDON = path.join(ROOT, "native", "ait_napi.node");

export const TARGETS = new Map([
  ["aarch64-apple-darwin", { platform: "darwin", arch: "arm64", library: "libait_napi.dylib", deployment: "11.0" }],
  ["x86_64-apple-darwin", { platform: "darwin", arch: "x64", library: "libait_napi.dylib", deployment: "10.12" }],
  ["aarch64-unknown-linux-gnu", { platform: "linux", arch: "arm64", library: "libait_napi.so" }],
  ["x86_64-unknown-linux-gnu", { platform: "linux", arch: "x64", library: "libait_napi.so" }],
  ["aarch64-pc-windows-msvc", { platform: "win32", arch: "arm64", library: "ait_napi.dll" }],
  ["x86_64-pc-windows-msvc", { platform: "win32", arch: "x64", library: "ait_napi.dll" }],
]);

function fail(message) {
  throw new Error(message);
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

async function readJson(filePath, label) {
  await regularFile(filePath, label);
  try {
    const value = JSON.parse(await readFile(filePath, "utf8"));
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      fail(`${label} must contain a JSON object`);
    }
    return value;
  } catch (error) {
    fail(`${label} must contain valid UTF-8 JSON: ${error.message}`);
  }
}

function parseLockedNode(source) {
  const sections = source.split(/^\[\[node\]\]\s*$/m);
  if (sections.length !== 2) {
    fail("ait-external.lock must contain exactly one node");
  }
  const nodeSource = sections[1].split(/^\[\[node\.binding\]\]\s*$/m);
  if (nodeSource.length !== 2) {
    fail("ait-external.lock must contain exactly one node binding");
  }
  const parseFields = (text) => {
    const fields = {};
    for (const line of text.split(/\r?\n/)) {
      const match = line.match(/^([a-z_]+)\s*=\s*(?:"([^"]*)"|(\d+))\s*$/);
      if (match !== null) {
        fields[match[1]] = match[2] ?? Number(match[3]);
      }
    }
    return fields;
  };
  return { node: parseFields(nodeSource[0]), binding: parseFields(nodeSource[1]) };
}

function assertLockedNode(lock) {
  const expectedNode = {
    name: "ait-core",
    repo_name: "ait-core",
    repository_index: 0,
    snapshot: CORE_SNAPSHOT,
    parent_path: "",
    materialize_to: ".ait-external/ait-core",
    license: "Apache-2.0",
    version: "1.0.0-rc.8",
  };
  const expectedBinding = {
    language: "rust",
    kind: "cargo-path",
    path: "rust/crates/ait-napi",
    package: "ait-napi",
  };
  for (const [field, expected] of Object.entries(expectedNode)) {
    if (lock.node[field] !== expected) {
      fail(`ait-external.lock node.${field} must be ${JSON.stringify(expected)}`);
    }
  }
  for (const [field, expected] of Object.entries(expectedBinding)) {
    if (lock.binding[field] !== expected) {
      fail(`ait-external.lock binding.${field} must be ${JSON.stringify(expected)}`);
    }
  }
}

async function validateInternalCore(manifestPath) {
  const lock = parseLockedNode(
    await readFile(path.join(ROOT, "ait-external.lock"), "utf8"),
  );
  assertLockedNode(lock);
  const marker = await readJson(
    path.join(INTERNAL_CORE_ROOT, ".ait-external-marker.json"),
    "external materialization marker",
  );
  for (const [field, expected] of Object.entries({
    name: "ait-core",
    repo_name: "ait-core",
    repository_index: 0,
    snapshot: CORE_SNAPSHOT,
    materialize_to: ".ait-external/ait-core",
  })) {
    if (marker[field] !== expected) {
      fail(`external marker field ${field} does not match ait-external.lock`);
    }
  }
  const canonicalRoot = await realpath(INTERNAL_CORE_ROOT);
  const canonicalManifest = await realpath(manifestPath);
  if (!canonicalManifest.startsWith(`${canonicalRoot}${path.sep}`)) {
    fail("materialized ait-napi manifest escapes its ait-core root");
  }
}

function mappedRows(mapping) {
  if (!Array.isArray(mapping.subtrees)) {
    fail("public source mapping subtrees must be an array");
  }
  const rows = new Map();
  for (const row of mapping.subtrees) {
    if (row === null || typeof row !== "object" || typeof row.source_repository !== "string") {
      fail("public source mapping contains an invalid subtree");
    }
    if (rows.has(row.source_repository)) {
      fail(`public source mapping repeats ${row.source_repository}`);
    }
    rows.set(row.source_repository, row);
  }
  return rows;
}

async function validatePublicCore(manifestPath) {
  if (path.basename(ROOT) !== "ait-node") {
    fail("public Node.js source subtree name is invalid");
  }
  const publicRoot = path.dirname(ROOT);
  const mapping = await readJson(
    path.join(publicRoot, "ait-monorepo-source.json"),
    "public source mapping",
  );
  const family = await readJson(
    path.join(publicRoot, "ait-release-family.json"),
    "public family manifest",
  );
  if (
    mapping.schema !== "ait.release.monorepo-source/v1" ||
    mapping.public_source_identity !== "weita2026/ait-native" ||
    mapping.public_publish !== false ||
    family.schema !== "ait.release.family/v3" ||
    family.public_source?.model !== "release-monorepo" ||
    family.public_source?.identity !== "weita2026/ait-native"
  ) {
    fail("public monorepo source identity is invalid");
  }
  const rows = mappedRows(mapping);
  const core = rows.get("ait-core");
  const node = rows.get("ait-node");
  if (
    rows.size !== 5 ||
    core?.path !== "ait-core" ||
    core?.source_snapshot !== CORE_SNAPSHOT ||
    core?.license !== "Apache-2.0" ||
    node?.path !== "ait-node" ||
    node?.license !== "Apache-2.0" ||
    JSON.stringify(node?.components) !== JSON.stringify(["ait-node"])
  ) {
    fail("public Node.js/core source mapping is invalid");
  }
  const nodeComponents = family.components?.filter(
    (component) => component?.id === "ait-node",
  );
  const coreComponents = family.components?.filter(
    (component) => component?.source_repository === "ait-core",
  );
  if (
    nodeComponents?.length !== 1 ||
    nodeComponents[0].source_repository !== "ait-node" ||
    nodeComponents[0].source_snapshot !== node.source_snapshot ||
    !Array.isArray(coreComponents) ||
    coreComponents.length === 0 ||
    coreComponents.some((component) => component.source_snapshot !== CORE_SNAPSHOT)
  ) {
    fail("public family Snapshots differ from the source mapping");
  }
  const canonicalExpected = await realpath(
    path.join(publicRoot, "ait-core", MANIFEST_RELATIVE),
  );
  const canonicalManifest = await realpath(manifestPath);
  if (canonicalManifest !== canonicalExpected) {
    fail("public monorepo manifest does not select the mapped ait-core");
  }
}

export async function resolveNativeManifest() {
  const internal = path.join(INTERNAL_CORE_ROOT, MANIFEST_RELATIVE);
  try {
    await regularFile(internal, "materialized ait-napi manifest");
    await validateInternalCore(internal);
    return { layout: "materialized-external", manifestPath: internal };
  } catch (error) {
    if (error?.code !== "ENOENT" && !String(error?.message).includes("is missing:")) {
      throw error;
    }
  }
  const publicManifest = path.join(PUBLIC_CORE_ROOT, MANIFEST_RELATIVE);
  await regularFile(publicManifest, "public monorepo ait-napi manifest");
  await validatePublicCore(publicManifest);
  return { layout: "public-monorepo", manifestPath: publicManifest };
}

export function hostTarget() {
  const matches = [...TARGETS.entries()].filter(
    ([, spec]) => spec.platform === process.platform && spec.arch === process.arch,
  );
  if (matches.length !== 1) {
    fail(`unsupported native Node.js host ${process.platform}/${process.arch}`);
  }
  return matches[0][0];
}

function requireNativeTarget(target) {
  const spec = TARGETS.get(target);
  if (spec === undefined) {
    fail(`unsupported native Node.js target ${target}`);
  }
  if (spec.platform !== process.platform || spec.arch !== process.arch) {
    fail(
      `target ${target} requires native host ${spec.platform}/${spec.arch}, got ${process.platform}/${process.arch}`,
    );
  }
  return spec;
}

function runCargo(args, environment) {
  const result = spawnSync("cargo", args, {
    cwd: ROOT,
    encoding: "utf8",
    env: environment,
    windowsHide: true,
  });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    fail(`cargo ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`);
  }
}

export async function nativeBuild(action = "build", selectedTarget = hostTarget()) {
  if (action !== "check" && action !== "build") {
    fail("native build action must be check or build");
  }
  const spec = requireNativeTarget(selectedTarget);
  const source = await resolveNativeManifest();
  const args = [
    action,
    "--release",
    "--locked",
    "--manifest-path",
    source.manifestPath,
    "--target",
    selectedTarget,
    "--target-dir",
    TARGET_ROOT,
  ];
  const environment = { ...process.env, CARGO_INCREMENTAL: "0" };
  if (spec.deployment !== undefined) {
    environment.MACOSX_DEPLOYMENT_TARGET = spec.deployment;
  }
  runCargo(args, environment);
  if (action === "check") {
    return {
      action,
      layout: source.layout,
      manifest: source.manifestPath,
      target: selectedTarget,
    };
  }
  const builtAddon = path.join(
    TARGET_ROOT,
    selectedTarget,
    "release",
    spec.library,
  );
  const entry = await regularFile(builtAddon, "built ait-napi addon");
  if (entry.size === 0) {
    fail("built ait-napi addon is empty");
  }
  await mkdir(path.dirname(INSTALLED_ADDON), { recursive: true });
  await copyFile(builtAddon, INSTALLED_ADDON);
  await regularFile(INSTALLED_ADDON, "installed ait-napi addon");
  return {
    action,
    addonPath: INSTALLED_ADDON,
    builtAddon,
    layout: source.layout,
    manifest: source.manifestPath,
    target: selectedTarget,
  };
}

async function main() {
  const [action = "build", target = hostTarget()] = process.argv.slice(2);
  if (process.argv.length > 4) {
    fail("usage: node scripts/native-build.mjs {check|build} [target]");
  }
  const result = await nativeBuild(action, target);
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (
  process.argv[1] !== undefined &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  main().catch((error) => {
    process.stderr.write(`${error.stack ?? error.message}\n`);
    process.exitCode = 1;
  });
}
