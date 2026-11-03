#!/usr/bin/env node

import { existsSync, lstatSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SNAPSHOT_RE = /^SNP-[0-9A-F]{12}$/;
const STABLE_RE = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const RC_RE = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-rc\.([1-9][0-9]*)$/;
const UNBOUND = "unbound";
const BINDING_CONTRACT = "ait.release.family-binding/v1";
const FAMILY_SCHEMA = "ait.release.family/v3";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail(
    "usage: release_family_bind.mjs --template <absolute-family.json> --binding <absolute-binding.json> --output <absolute-output.json> [--version <family-version>]",
    64,
  );
}

function parseCli(argv) {
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) usage();
    options[key] = value;
  }
  for (const key of ["--template", "--binding", "--output"]) {
    if (typeof options[key] !== "string" || !path.isAbsolute(options[key])) usage();
  }
  if (options["--version"] !== undefined && !familyVersion(options["--version"])) {
    fail("family overlay version must be an exact stable or rc version");
  }
  return options;
}

function familyVersion(value) {
  return STABLE_RE.test(value ?? "") || RC_RE.test(value ?? "");
}

function pep440(version) {
  return version.replace(/-rc\.([1-9][0-9]*)$/, "rc$1");
}

function readJson(file, label) {
  if (!existsSync(file) || !lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) {
    fail(`${label} must be a regular file: ${file}`, 66);
  }
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    fail(`${label} is invalid JSON: ${file}`);
  }
}

function validateTemplate(family) {
  if (family?.schema !== FAMILY_SCHEMA || typeof family.family?.version !== "string" ||
      family.family?.tag !== `v${family.family.version}` || !familyVersion(family.family.version) ||
      !Array.isArray(family.components) || family.components.length === 0) {
    fail("family template is not a canonical v3 family manifest");
  }
  const repositories = new Set();
  for (const component of family.components) {
    if (typeof component?.id !== "string" || typeof component.source_repository !== "string") {
      fail("family template component identity is invalid");
    }
    if (component.source_snapshot !== UNBOUND) {
      fail(`family template component ${component.id} must be ${UNBOUND}; Snapshot binding belongs to the export-time binding record`);
    }
    if (!["family", "pep440"].includes(component.version_scheme)) {
      fail(`family template component ${component.id} has an unsupported version scheme`);
    }
    const expected = component.version_scheme === "pep440" ? pep440(family.family.version) : family.family.version;
    if (component.version !== expected) {
      fail(`family template component ${component.id} version differs from the family version`);
    }
    repositories.add(component.source_repository);
  }
  return [...repositories].sort();
}

function validateBinding(binding, repositories, templateVersion) {
  if (binding?.contract !== BINDING_CONTRACT || typeof binding.version !== "string" ||
      binding.tag !== `v${binding.version}` || binding.components === null ||
      typeof binding.components !== "object" || Array.isArray(binding.components)) {
    fail("family binding record is invalid");
  }
  if (binding.version !== templateVersion) {
    fail(`family binding version ${binding.version} differs from the template version ${templateVersion}`);
  }
  const bound = Object.keys(binding.components).sort();
  if (JSON.stringify(bound) !== JSON.stringify(repositories)) {
    fail(`family binding must name exactly the template repositories: ${repositories.join(", ")}`);
  }
  const snapshots = new Set();
  for (const repository of bound) {
    const snapshot = binding.components[repository];
    if (!SNAPSHOT_RE.test(snapshot ?? "")) fail(`family binding Snapshot for ${repository} is invalid`);
    if (snapshots.has(snapshot)) fail("family binding Snapshot identities must be distinct");
    snapshots.add(snapshot);
  }
}

function bindFamily(template, binding, overlayVersion) {
  const repositories = validateTemplate(template);
  validateBinding(binding, repositories, template.family.version);
  const bound = structuredClone(template);
  const version = overlayVersion ?? template.family.version;
  bound.family.version = version;
  bound.family.tag = `v${version}`;
  bound.components = bound.components.map((component) => ({
    ...component,
    source_snapshot: binding.components[component.source_repository],
    version: component.version_scheme === "pep440" ? pep440(version) : version,
  }));
  return { bound, version, overlaid: version !== template.family.version };
}

function main() {
  const options = parseCli(process.argv.slice(2));
  const output = options["--output"];
  if (existsSync(output)) fail(`family binding output already exists: ${output}`, 73);
  const parent = path.dirname(output);
  if (!existsSync(parent) || !lstatSync(parent).isDirectory()) fail("family binding output parent must exist", 66);
  const template = readJson(options["--template"], "family template");
  const binding = readJson(options["--binding"], "family binding");
  const { bound } = bindFamily(template, binding, options["--version"]);
  writeFileSync(output, `${JSON.stringify(bound, null, 2)}\n`, { mode: 0o644, flag: "wx" });
  process.stdout.write(`${output}\n`);
}

if (process.argv[1] && realpathSync(path.resolve(process.argv[1])) === realpathSync(fileURLToPath(import.meta.url))) {
  main();
}

export { BINDING_CONTRACT, UNBOUND, bindFamily, pep440, validateBinding, validateTemplate };
