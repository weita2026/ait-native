#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail("usage: release_source_equivalence.mjs --prior <absolute-dir> --final <absolute-dir> --policy <absolute-json> --output <absolute-json>", 64);
}

function parseCli(argv) {
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) usage();
    options[key] = value;
  }
  for (const key of ["--prior", "--final", "--policy", "--output"]) if (!options[key]) usage();
  return options;
}

function absolute(value, label) {
  if (!path.isAbsolute(value)) fail(`${label} must be an absolute path`);
  return path.resolve(value);
}

function regularJson(file, label) {
  if (!existsSync(file) || !lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) fail(`${label} must be a regular file`);
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    fail(`${label} is invalid JSON`);
  }
}

function digest(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function inventory(root, ignored) {
  if (!existsSync(root) || !lstatSync(root).isDirectory() || lstatSync(root).isSymbolicLink()) fail(`source root must be a real directory: ${root}`);
  const rows = new Map();
  const walk = (directory, relative) => {
    for (const name of readdirSync(directory).sort()) {
      const childRelative = relative ? `${relative}/${name}` : name;
      if (!relative && ignored.has(name)) continue;
      const child = path.join(directory, name);
      const stat = lstatSync(child);
      if (stat.isSymbolicLink()) fail(`source inventory contains a symbolic link: ${childRelative}`);
      if (stat.isDirectory()) walk(child, childRelative);
      else if (stat.isFile()) rows.set(childRelative, { sha256: digest(child), mode: stat.mode & 0o777 });
      else fail(`source inventory contains an unsupported entry: ${childRelative}`);
    }
  };
  walk(root, "");
  return rows;
}

function same(left, right) {
  return left?.sha256 === right?.sha256 && left?.mode === right?.mode;
}

const cli = parseCli(process.argv.slice(2));
const priorRoot = absolute(cli["--prior"], "prior");
const finalRoot = absolute(cli["--final"], "final");
const policyPath = absolute(cli["--policy"], "policy");
const output = absolute(cli["--output"], "output");
if (existsSync(output)) fail("source-equivalence output already exists", 73);
const policy = regularJson(policyPath, "source-equivalence policy");
if (
  policy.contract !== "ait.release.source-equivalence-policy/v1" ||
  !Array.isArray(policy.expected_changed_paths) || policy.expected_changed_paths.length === 0 ||
  !Array.isArray(policy.required_equal_paths) || policy.required_equal_paths.length === 0 ||
  typeof policy.family_manifest !== "string"
) fail("source-equivalence policy contract is invalid");
for (const field of ["expected_changed_paths", "required_equal_paths"]) {
  if (policy[field].some((row) => typeof row !== "string" || row.startsWith("/") || row.includes(".."))) {
    fail(`source-equivalence ${field} contains an unsafe path`);
  }
  if (new Set(policy[field]).size !== policy[field].length) fail(`source-equivalence ${field} contains duplicates`);
}
const ignored = new Set(policy.ignored_root_entries ?? [".ait", ".ait-runtime", ".git"]);
const prior = inventory(priorRoot, ignored);
const final = inventory(finalRoot, ignored);
const allPaths = [...new Set([...prior.keys(), ...final.keys()])].sort();
const changedPaths = allPaths.filter((entry) => !same(prior.get(entry), final.get(entry)));
const expected = [...policy.expected_changed_paths].sort();
if (JSON.stringify(changedPaths) !== JSON.stringify(expected)) {
  fail(`source-equivalence changed paths differ: ${JSON.stringify(changedPaths)}`);
}
for (const entry of policy.required_equal_paths) {
  if (!prior.has(entry) || !final.has(entry) || !same(prior.get(entry), final.get(entry))) {
    fail(`source-equivalence required path differs: ${entry}`);
  }
}
const priorFamily = regularJson(path.join(priorRoot, policy.family_manifest), "prior family manifest");
const finalFamily = regularJson(path.join(finalRoot, policy.family_manifest), "final family manifest");
const selectors = (family) => (family.components ?? []).map((row) => ({ id: row.id, source_snapshot: row.source_snapshot }));
if (JSON.stringify(selectors(priorFamily)) !== JSON.stringify(selectors(finalFamily))) {
  fail("source-equivalence nested family selectors differ");
}
mkdirSync(path.dirname(output), { recursive: true, mode: 0o700 });
writeFileSync(output, `${JSON.stringify({
  contract: "ait.release.source-equivalence/v1",
  status: "pass",
  prior_root: priorRoot,
  final_root: finalRoot,
  policy_sha256: digest(policyPath),
  changed_paths: changedPaths,
  nested_family_selectors_equal: true,
}, null, 2)}\n`, { mode: 0o600, flag: "wx" });
process.stdout.write(`${output}\n`);
