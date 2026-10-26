import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

export const RUNNER_CORE_PATH = ".ait-external/ait-core";
export const SOURCE_CACHE_EVIDENCE = "source-cache.evidence.json";
const SNAPSHOT = /^SNP-[0-9A-F]{12}$/;
const HASH = /^[0-9a-f]{64}$/;
const OPERATIONAL = new Set([".ait", ".ait-external", ".ait-runtime", ".ait-worktree-links", ".git"]);

function regular(file) {
  assert(lstatSync(file).isFile() && !lstatSync(file).isSymbolicLink(), `dependency input is not a regular file: ${file}`);
  return readFileSync(file, "utf8");
}

function directCoreRow(file, table) {
  const rows = regular(file).split(new RegExp(`^\\[\\[${table}\\]\\]\\s*$`, "m")).slice(1);
  const matches = rows.map(text => {
    const result = {};
    for (const line of text.split(/^\s*\[/m)[0].split("\n")) {
      const item = /^([a-z_]+)\s*=\s*("(?:[^"\\]|\\.)*"|\d+)\s*$/.exec(line);
      if (!item) continue;
      assert(!Object.hasOwn(result, item[1]), `duplicate dependency key in ${file}`);
      result[item[1]] = JSON.parse(item[2]);
    }
    return result;
  }).filter(row => row.name === "ait-core" && (row.parent_path ?? "") === "");
  assert.equal(matches.length, 1, `Runner must declare exactly one direct Core dependency: ${file}`);
  return matches[0];
}

export function runnerCoreDeclaration(runner) {
  const manifest = directCoreRow(path.join(runner, "ait-external.toml"), "external");
  const locked = directCoreRow(path.join(runner, "ait-external.lock"), "node");
  for (const key of ["name", "repo_name", "repository_index", "snapshot", "materialize_to", "license", "version"]) {
    assert.equal(manifest[key], locked[key], `Runner Core manifest/lock mismatch: ${key}`);
  }
  assert.equal(locked.repo_name, "ait-core");
  assert.equal(locked.repository_index, 0);
  assert.equal(locked.materialize_to, RUNNER_CORE_PATH);
  assert.equal(locked.license, "Apache-2.0");
  assert(SNAPSHOT.test(locked.snapshot));
  assert(/^\d+\.\d+\.\d+(?:-rc\.\d+)?$/.test(locked.version));
  return locked;
}

export function validateReleaseDependencyGraph(root, status) {
  const direct = status.externals.filter(row => row.parent_path === "");
  const core = direct.filter(row => row.name === "ait-core");
  assert.equal(core.length, 1, "release needs one direct Core producer");
  const nested = status.externals.filter(row => row.parent_path !== "");
  assert.equal(nested.length, 3, "release must retain the three declared Core dependencies");
  for (const owner of ["ait-node", "ait-python", "ait-runner"]) {
    const parent = `.ait-external/${owner}`;
    const rows = nested.filter(row => row.parent_path === parent);
    assert.equal(rows.length, 1, `missing or ambiguous declared Core dependency: ${owner}`);
    const row = rows[0];
    assert(row.name === "ait-core" && row.repo_name === "ait-core" && row.repository_index === 0 &&
      row.materialize_to === `${parent}/${RUNNER_CORE_PATH}` && row.state === "materialized" &&
      row.dirty === false && row.linked === false && row.lock_drift === false,
    `declared Core dependency is not clean/materialized: ${owner}`);
    const expected = owner === "ait-runner" ? runnerCoreDeclaration(path.join(root, parent)).snapshot : core[0].snapshot;
    assert.equal(row.snapshot, expected, `${owner} Core dependency differs from its required producer`);
    const marker = JSON.parse(regular(path.join(root, row.materialize_to, ".ait-external-marker.json")));
    assert(marker.format === "ait.external.materialized" && marker.repo_name === "ait-core" &&
      marker.snapshot === expected && marker.repository_index === 0 && marker.parent_path === parent &&
      marker.materialize_to === row.materialize_to, `${owner} Core materialization differs from the declared Snapshot`);
  }
}

export function dependencyTreeInventory(root) {
  const files = [];
  function walk(relative) {
    for (const name of readdirSync(path.join(root, relative))) {
      const local = path.join(relative, name), file = path.join(root, local), stat = lstatSync(file);
      assert(!stat.isSymbolicLink(), `dependency source contains a link: ${local}`);
      if (relative === "" && name === SOURCE_CACHE_EVIDENCE) continue;
      assert(!OPERATIONAL.has(name) && name !== ".ait-external-marker.json" && name !== ".gitmodules",
        `dependency source contains operational state: ${local}`);
      if (stat.isDirectory()) walk(local);
      else {
        assert(stat.isFile(), `dependency source contains a special file: ${local}`);
        const bytes = readFileSync(file);
        files.push({ relative: local.split(path.sep).join("/"), size: bytes.length,
          hash: createHash("sha256").update(bytes).digest("hex"), executable: Boolean(stat.mode & 0o111) });
      }
    }
  }
  walk("");
  files.sort((a, b) => Buffer.compare(Buffer.from(a.relative), Buffer.from(b.relative)));
  return files;
}

export function dependencyTreeDigest(root) {
  const files = dependencyTreeInventory(root);
  return createHash("sha256").update(files.map(row => `${row.size}\t${row.hash}\t${row.executable ? 1 : 0}\t${row.relative}\n`).join("")).digest("hex");
}

export function validateRunnerDependencySource(runner, mainCore) {
  const declared = runnerCoreDeclaration(runner);
  const source = path.join(runner, RUNNER_CORE_PATH);
  const container = lstatSync(path.join(runner, ".ait-external"));
  assert(container.isDirectory() && !container.isSymbolicLink(), "Runner dependency container is not a regular directory");
  assert.deepEqual(readdirSync(path.join(runner, ".ait-external")), ["ait-core"],
    "Runner's immutable dependency container contains undeclared content");
  assert(lstatSync(source).isDirectory() && !lstatSync(source).isSymbolicLink(), "Runner's immutable Core source is missing");
  const evidence = JSON.parse(regular(path.join(source, SOURCE_CACHE_EVIDENCE)));
  assert(evidence.contract === "ait.release.source-cache/v1" && evidence.status === "ready" &&
    evidence.repo_name === "ait-core" && evidence.repository_index === 0 &&
    evidence.source_snapshot === declared.snapshot && HASH.test(evidence.source_manifest_hash ?? "") &&
    /^\d+$/.test(evidence.source_snapshot_created_at ?? "") && evidence.version === declared.version &&
    [RUNNER_CORE_PATH, `.ait-external/ait-runner/${RUNNER_CORE_PATH}`].includes(evidence.materialize_to) &&
    evidence.license === "Apache-2.0" && evidence.source_authority === "ait_remote_snapshot_boundary" &&
    evidence.workspace_clean === true && evidence.remote_coordinates_embedded === false && evidence.public_publish === false,
  "Runner's immutable Core source provenance differs from its declaration");
  const equivalent = dependencyTreeDigest(source) === dependencyTreeDigest(mainCore);
  const cargo = regular(path.join(runner, "Cargo.toml"));
  const matches = [...cargo.matchAll(/^ait-core\s*=\s*\{[^\n]*\bpath\s*=\s*"([^"]+)"[^\n]*\}\s*$/gm)];
  assert.equal(matches.length, 1, "Runner must use one explicit Core Cargo path");
  const expected = equivalent ? "../ait-core/rust/crates/ait-core" : `${RUNNER_CORE_PATH}/rust/crates/ait-core`;
  assert.equal(matches[0][1], expected, "Runner Core path does not preserve the declared source or proven equivalent subtree");
  return { declared, equivalent, source };
}

const PUBLIC_DEPENDENCY = `ait-runner/${RUNNER_CORE_PATH}/`;

function publicDependencyInventory(root) {
  // Older non-monorepo release inputs do not contain a Runner subtree.
  if (!existsSync(path.join(root, "ait-runner"))) return [];
  const { source } = validateRunnerDependencySource(path.join(root, "ait-runner"), path.join(root, "ait-core"));
  const rows = dependencyTreeInventory(source);
  const evidence = path.join(source, SOURCE_CACHE_EVIDENCE);
  regular(evidence);
  const bytes = readFileSync(evidence);
  rows.push({ relative: SOURCE_CACHE_EVIDENCE, size: bytes.length,
    hash: createHash("sha256").update(bytes).digest("hex"), executable: Boolean(lstatSync(evidence).mode & 0o111) });
  return rows.map(row => ({ ...row, relative: PUBLIC_DEPENDENCY + row.relative }))
    .sort((a, b) => Buffer.compare(Buffer.from(a.relative), Buffer.from(b.relative)));
}

function sourceGit(root, args, input, maxBuffer = 4 * 1024 * 1024) {
  const result = spawnSync("git", ["--literal-pathspecs", "-C", root, ...args], { input, maxBuffer });
  assert(!result.error && result.status === 0,
    `public dependency Git check failed: ${result.error?.message ?? result.stderr?.toString()}`);
  return result.stdout;
}

function verifyGitDependencyInventory(root, expected, commit) {
  if (expected.length === 0) return;
  const args = commit ? ["ls-tree", "-r", "-z", "--full-tree", commit, "--", PUBLIC_DEPENDENCY.slice(0, -1)] :
    ["ls-files", "--stage", "-z", "--", PUBLIC_DEPENDENCY.slice(0, -1)];
  const entries = sourceGit(root, args).toString("utf8").split("\0").filter(Boolean).map(line => {
    const match = (commit ? /^([0-7]{6}) blob ([0-9a-f]{40,64})\t([\s\S]+)$/ :
      /^([0-7]{6}) ([0-9a-f]{40,64}) 0\t([\s\S]+)$/).exec(line);
    assert(match && ["100644", "100755"].includes(match[1]), "public dependency Git inventory contains a non-regular entry");
    return { mode: match[1], oid: match[2], relative: match[3] };
  });
  assert.deepEqual(entries.map(row => row.relative), expected.map(row => row.relative),
    "public dependency Git file membership differs from the validated export inventory");
  const budget = expected.reduce((total, row) => total + row.size + 256, 4096);
  const blobs = sourceGit(root, ["cat-file", "--batch"], entries.map(row => row.oid + "\n").join(""), budget);
  let offset = 0;
  const actual = entries.map(row => {
    const end = blobs.indexOf(10, offset);
    assert(end >= offset, "public dependency Git blob header is truncated");
    const header = blobs.subarray(offset, end).toString("ascii").split(" ");
    assert(header[0] === row.oid && header[1] === "blob" && /^\d+$/.test(header[2]), "public dependency Git blob header differs");
    const size = Number(header[2]);
    offset = end + 1;
    assert(Number.isSafeInteger(size) && blobs[offset + size] === 10, "public dependency Git blob is truncated");
    const bytes = blobs.subarray(offset, offset + size);
    offset += size + 1;
    return { relative: row.relative, size, hash: createHash("sha256").update(bytes).digest("hex"), executable: row.mode === "100755" };
  });
  assert.equal(offset, blobs.length, "public dependency Git blob output contains unexpected data");
  assert.deepEqual(actual, expected, "public dependency Git bytes or modes differ from the validated export inventory");
}

export function stagePublicRunnerDependency(exportRoot, publicRoot) {
  const expected = publicDependencyInventory(exportRoot);
  assert.deepEqual(publicDependencyInventory(publicRoot), expected,
    "public Runner dependency differs from the validated export inventory");
  if (expected.length === 0) return;
  // Force only the already validated literal file membership, never a directory.
  const paths = Buffer.from(expected.map(row => row.relative + "\0").join(""));
  sourceGit(publicRoot, ["add", "--force", "--pathspec-from-file=-", "--pathspec-file-nul"], paths);
  verifyGitDependencyInventory(publicRoot, expected);
}

export function verifyPublicRunnerDependencyCommit(exportRoot, publicRoot, commit) {
  assert(typeof commit === "string" && /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(commit),
    "public source commit identity is invalid");
  verifyGitDependencyInventory(publicRoot, publicDependencyInventory(exportRoot), commit);
}

