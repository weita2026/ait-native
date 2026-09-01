#!/usr/bin/env node

import { execFileSync } from "node:child_process";
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
const newRcMatch = /^([0-9]+\.[0-9]+\.[0-9]+)-rc\.([1-9][0-9]*)$/.exec(newVersion ?? "");
const newStableMatch = /^[0-9]+\.[0-9]+\.[0-9]+$/.exec(newVersion ?? "");
const qualifiedPythonCanonical =
  oldMatch !== null && oldPython === `${oldMatch[1]}rc${oldMatch[2]}`;
const rcAdvance =
  oldMatch !== null &&
  newRcMatch !== null &&
  qualifiedPythonCanonical &&
  oldMatch[1] === newRcMatch[1] &&
  Number(newRcMatch[2]) === Number(oldMatch[2]) + 1 &&
  newPython === `${newRcMatch[1]}rc${newRcMatch[2]}`;
const stablePromotion =
  oldMatch !== null &&
  newStableMatch !== null &&
  qualifiedPythonCanonical &&
  oldMatch[1] === newVersion &&
  newPython === newVersion;
const oldStableMatch = /^([0-9]+)\.([0-9]+)\.([0-9]+)$/.exec(oldVersion ?? "");
const newStableParts = /^([0-9]+)\.([0-9]+)\.([0-9]+)$/.exec(newVersion ?? "");
const stablePatchAdvance =
  oldStableMatch !== null &&
  newStableParts !== null &&
  oldPython === oldVersion &&
  oldStableMatch[1] === newStableParts[1] &&
  oldStableMatch[2] === newStableParts[2] &&
  Number(newStableParts[3]) === Number(oldStableMatch[3]) + 1 &&
  newPython === newVersion;
const stableMinorAdvance =
  oldStableMatch !== null &&
  newStableParts !== null &&
  oldPython === oldVersion &&
  oldStableMatch[1] === newStableParts[1] &&
  Number(newStableParts[2]) === Number(oldStableMatch[2]) + 1 &&
  Number(newStableParts[3]) === 0 &&
  newPython === newVersion;
if (
  !(rcAdvance || stablePromotion || stablePatchAdvance || stableMinorAdvance) ||
  releaseFamily.family.tag !== `v${newVersion}`
) {
  fail(
    "release delta must advance exactly one canonical RC ordinal, promote the qualified RC base to its stable version, or advance a qualified stable base by exactly one patch version or one minor version with a reset patch",
  );
}

const expectedSourceRepositories = [
  "ait-core",
  "ait-node",
  "ait-python",
  "ait-runner",
  "ait-server",
];

function sourceSnapshots(mapping, version, label) {
  if (
    mapping?.schema !== "ait.release.monorepo-source/v1" ||
    mapping.family_version !== version ||
    mapping.family_tag !== `v${version}` ||
    !Array.isArray(mapping.subtrees) ||
    mapping.subtrees.length !== expectedSourceRepositories.length
  ) {
    fail(`${label} monorepo mapping authority is invalid`);
  }
  const snapshots = new Map();
  for (const subtree of mapping.subtrees) {
    const repositoryName = subtree?.source_repository;
    const snapshot = subtree?.source_snapshot;
    if (
      !expectedSourceRepositories.includes(repositoryName) ||
      !/^SNP-[0-9A-F]{12}$/.test(snapshot ?? "") ||
      snapshots.has(repositoryName)
    ) {
      fail(`${label} monorepo mapping Snapshot authority is invalid`);
    }
    snapshots.set(repositoryName, snapshot);
  }
  if (expectedSourceRepositories.some((name) => !snapshots.has(name))) {
    fail(`${label} monorepo mapping repository inventory is invalid`);
  }
  if (new Set(snapshots.values()).size !== snapshots.size) {
    fail(`${label} monorepo mapping Snapshot authority is ambiguous`);
  }
  return snapshots;
}

const qualifiedMapping = jsonAt(qualifiedCommit, "ait-monorepo-source.json");
const releaseMapping = jsonAt(releaseCommit, "ait-monorepo-source.json");
const qualifiedSnapshots = sourceSnapshots(qualifiedMapping, oldVersion, "qualified");
const releaseSnapshots = sourceSnapshots(releaseMapping, newVersion, "release");
const authoritySnapshotTransitions = expectedSourceRepositories
  .map((sourceRepository) => ({
    source_repository: sourceRepository,
    qualified_snapshot: qualifiedSnapshots.get(sourceRepository),
    release_snapshot: releaseSnapshots.get(sourceRepository),
  }))
  .filter((row) => row.qualified_snapshot !== row.release_snapshot);

function dotEscaped(value, slashCount) {
  return value.replaceAll(".", `${"\\".repeat(slashCount)}.`);
}

const tokenTransitions = [
  { release: newPython, qualified: oldPython },
  { release: newVersion, qualified: oldVersion },
  ...[1, 2].flatMap((slashCount) => [
    {
      release: dotEscaped(newPython, slashCount),
      qualified: dotEscaped(oldPython, slashCount),
    },
    {
      release: dotEscaped(newVersion, slashCount),
      qualified: dotEscaped(oldVersion, slashCount),
    },
  ]),
  ...authoritySnapshotTransitions.map((row) => ({
    release: row.release_snapshot,
    qualified: row.qualified_snapshot,
  })),
];
// Normalization replaces qualified tokens with release tokens, so the
// qualified side must stay unambiguous. The release side is allowed to
// collide: a stable promotion maps both the family and Python qualified
// forms onto the same stable version string. A stable patch advance makes
// the family and Python transitions byte-identical, so exact duplicate
// pairs collapse before ambiguity is judged; distinct release targets for
// one qualified token remain an error.
const dedupedTransitions = [
  ...new Map(
    tokenTransitions.map((transition) => [
      `${transition.qualified}\u0000${transition.release}`,
      transition,
    ]),
  ).values(),
];
tokenTransitions.length = 0;
tokenTransitions.push(...dedupedTransitions);
if (
  tokenTransitions.some(
    (transition) =>
      !transition.release ||
      !transition.qualified ||
      transition.release === transition.qualified,
  ) ||
  new Set(tokenTransitions.map((transition) => transition.qualified)).size !==
    tokenTransitions.length
) {
  fail("release delta token authority is invalid or ambiguous");
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
  // Each qualified token occurrence may independently advance to its
  // release form or remain unchanged. Third-party version strings that
  // coincide with a bare stable family version (for example a dependency
  // whose own version equals the qualified family version) therefore stay
  // valid without weakening the rule that every byte outside the token
  // occurrences must match exactly. The walk is a bounded depth-first
  // match over the token occurrence points; regular expressions are
  // deliberately avoided because engine code-size limits reject patterns
  // built from large lockfiles.
  const qualifiedText = before.toString("utf8");
  const releaseText = after.toString("utf8");
  const orderedTransitions = [...tokenTransitions].sort(
    (a, b) => b.qualified.length - a.qualified.length,
  );
  const admissible = (() => {
    const stack = [[0, 0]];
    const seen = new Set();
    while (stack.length > 0) {
      let [qi, ri] = stack.pop();
      const key = `${qi}:${ri}`;
      if (seen.has(key)) continue;
      seen.add(key);
      let diverged = false;
      while (qi < qualifiedText.length) {
        const transition = orderedTransitions.find((row) =>
          qualifiedText.startsWith(row.qualified, qi),
        );
        if (transition === undefined) {
          if (releaseText[ri] !== qualifiedText[qi]) {
            diverged = true;
            break;
          }
          qi += 1;
          ri += 1;
        } else {
          const keepMatches = releaseText.startsWith(transition.qualified, ri);
          const advanceMatches = releaseText.startsWith(transition.release, ri);
          if (advanceMatches && keepMatches) {
            stack.push([qi + transition.qualified.length, ri + transition.qualified.length]);
            qi += transition.qualified.length;
            ri += transition.release.length;
          } else if (advanceMatches) {
            qi += transition.qualified.length;
            ri += transition.release.length;
          } else if (keepMatches) {
            qi += transition.qualified.length;
            ri += transition.qualified.length;
          } else {
            diverged = true;
            break;
          }
        }
      }
      if (!diverged && qi === qualifiedText.length && ri === releaseText.length) {
        return true;
      }
    }
    return false;
  })();
  if (!admissible) {
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
      authority_snapshot_transitions: authoritySnapshotTransitions,
      structural_authority_paths: [...structuralAuthorityPaths],
      normalized_version_paths: normalizedPaths,
      changed_paths: changed,
    },
    null,
    2,
  )}\n`,
);
