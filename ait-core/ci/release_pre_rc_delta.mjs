#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(65);
}

function usage(message) {
  if (message) process.stderr.write(`${message}\n`);
  process.stderr.write(
    "usage: release_pre_rc_delta.mjs --repository <git-root> --qualified-commit <sha> --release-commit <sha>\n",
  );
  process.exit(64);
}

const options = {};
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!key?.startsWith("--") || value === undefined || options[key]) {
    usage("invalid or repeated pre-RC delta option");
  }
  options[key] = value;
}

const repository = options["--repository"];
const qualifiedCommit = options["--qualified-commit"];
const releaseCommit = options["--release-commit"];
if (!repository || !qualifiedCommit || !releaseCommit) usage();
if (!/^[0-9a-f]{40}$/.test(qualifiedCommit) || !/^[0-9a-f]{40}$/.test(releaseCommit)) {
  usage("pre-RC commits must be full lowercase Git identities");
}

function git(args, encoding = "utf8") {
  try {
    return execFileSync("git", ["-C", repository, ...args], {
      encoding,
      maxBuffer: 32 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const detail = error.stderr?.toString().trim();
    fail(detail || `Git operation failed: ${args.join(" ")}`);
  }
}

const root = git(["rev-parse", "--show-toplevel"]).trim();
if (path.resolve(root) !== path.resolve(repository)) {
  fail("pre-RC delta repository must be the canonical Git root");
}

const lineage = git(["rev-list", "--parents", "-n", "1", releaseCommit]).trim().split(/\s+/);
if (lineage.length !== 2 || lineage[0] !== releaseCommit || lineage[1] !== qualifiedCommit) {
  fail("release commit must be the single direct child of the qualified repair commit");
}

function jsonAt(commit, relativePath) {
  const text = git(["show", `${commit}:${relativePath}`]);
  try {
    return JSON.parse(text);
  } catch {
    fail(`invalid JSON at ${commit}:${relativePath}`);
  }
}

const qualifiedFamily = jsonAt(qualifiedCommit, "ait-release-family.json");
const releaseFamily = jsonAt(releaseCommit, "ait-release-family.json");
const oldVersion = qualifiedFamily?.family?.version;
const newVersion = releaseFamily?.family?.version;
const oldPython = qualifiedFamily?.components?.find((row) => row.id === "ait-python")?.version;
const newPython = releaseFamily?.components?.find((row) => row.id === "ait-python")?.version;
const oldMatch = /^([0-9]+\.[0-9]+\.[0-9]+)-rc\.([1-9][0-9]*)$/.exec(oldVersion ?? "");
const newMatch = /^([0-9]+\.[0-9]+\.[0-9]+)-rc\.([1-9][0-9]*)$/.exec(newVersion ?? "");
if (
  !oldMatch ||
  !newMatch ||
  oldMatch[1] !== newMatch[1] ||
  Number(newMatch[2]) !== Number(oldMatch[2]) + 1 ||
  oldPython !== `${oldMatch[1]}rc${oldMatch[2]}` ||
  newPython !== `${newMatch[1]}rc${newMatch[2]}` ||
  releaseFamily.family.tag !== `v${newVersion}`
) {
  fail("release delta must advance exactly one canonical RC ordinal and Python mapping");
}

const requiredAuthorityPaths = new Set([
  "ait-release-family.json",
  "ait-monorepo-source.json",
  "ci/native_bootstrap_matrix.json",
  "ci/release_repository_authorities.json",
]);
const structuralAuthorityPaths = new Set([
  ...requiredAuthorityPaths,
  "ait-core/ait-release-family.json",
  "ait-core/ci/release_repository_authorities.json",
]);
const changed = git(["diff", "--name-only", "-z", qualifiedCommit, releaseCommit], null)
  .toString("utf8")
  .split("\0")
  .filter(Boolean);
if (changed.length === 0) fail("release delta is empty");

const normalizedPaths = [];
for (const relativePath of changed) {
  if (structuralAuthorityPaths.has(relativePath)) continue;
  let before;
  let after;
  try {
    before = git(["show", `${qualifiedCommit}:${relativePath}`], null);
    after = git(["show", `${releaseCommit}:${relativePath}`], null);
  } catch {
    fail(`release delta adds or removes a non-authority path: ${relativePath}`);
  }
  if (before.includes(0) || after.includes(0)) {
    fail(`release delta changes a binary non-authority path: ${relativePath}`);
  }
  const normalized = after
    .toString("utf8")
    .split(newPython)
    .join(oldPython)
    .split(newVersion)
    .join(oldVersion);
  if (normalized !== before.toString("utf8")) {
    fail(`release delta contains non-version changes: ${relativePath}`);
  }
  normalizedPaths.push(relativePath);
}

const requiredAuthorities = [...requiredAuthorityPaths];
for (const required of requiredAuthorities) {
  if (!changed.includes(required)) fail(`release delta is missing authority path: ${required}`);
}
if (normalizedPaths.length === 0) {
  fail("release delta does not update any component version authority");
}

process.stdout.write(
  `${JSON.stringify(
    {
      contract: "ait.release.pre-rc-delta/v1",
      decision: "pass",
      qualified_commit: qualifiedCommit,
      release_commit: releaseCommit,
      qualified_version: oldVersion,
      release_version: newVersion,
      structural_authority_paths: [...structuralAuthorityPaths],
      normalized_version_paths: normalizedPaths,
      changed_paths: changed,
    },
    null,
    2,
  )}\n`,
);
