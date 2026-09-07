#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function parseCli(argv) {
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) fail("invalid Web admission init arguments", 64);
    options[key] = value;
  }
  for (const key of [
    "--candidate", "--components", "--repository", "--ait", "--server-bin", "--personal-root", "--personal-bin",
    "--community-bin", "--community-cli-bin", "--server-url", "--seed", "--output",
  ]) if (!options[key]) fail(`missing required argument: ${key}`, 64);
  return options;
}

function absolute(value, label) {
  if (!path.isAbsolute(value)) fail(`${label} must be absolute`, 64);
  return path.resolve(value);
}

function readJson(file, label) {
  try { return JSON.parse(readFileSync(file, "utf8")); } catch { fail(`${label} is invalid JSON`); }
}

function runJson(command, argv, cwd, label) {
  const result = spawnSync(command, argv, { cwd, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  if (result.status !== 0) fail(`${label} failed`);
  try { return { document: JSON.parse(result.stdout), raw: result.stdout }; } catch { fail(`${label} returned invalid JSON`); }
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function treeHash(root) {
  const hash = createHash("sha256");
  const files = [];
  const walk = (directory, relative = "") => {
    for (const name of readdirSync(directory).sort()) {
      const file = path.join(directory, name);
      const next = path.join(relative, name);
      const stat = lstatSync(file);
      if (stat.isSymbolicLink()) fail(`acceptance tree contains a symlink: ${next}`);
      if (stat.isDirectory()) walk(file, next);
      else if (stat.isFile()) files.push({ file, relative: next });
      else fail(`acceptance tree contains a special file: ${next}`);
    }
  };
  walk(root);
  for (const row of files) {
    hash.update(row.relative);
    hash.update("\0");
    hash.update(readFileSync(row.file));
    hash.update("\0");
  }
  return { sha256: hash.digest("hex"), files };
}

const cli = parseCli(process.argv.slice(2));
const candidatePath = absolute(cli["--candidate"], "candidate");
const componentsPath = absolute(cli["--components"], "components");
const repository = absolute(cli["--repository"], "repository");
const personalRoot = absolute(cli["--personal-root"], "personal root");
const ait = absolute(cli["--ait"], "ait");
const serverBin = absolute(cli["--server-bin"], "server binary");
const personalBin = absolute(cli["--personal-bin"], "personal binary");
const communityBin = absolute(cli["--community-bin"], "community binary");
const communityCliBin = absolute(cli["--community-cli-bin"], "community CLI binary");
const output = absolute(cli["--output"], "output");
const seed = Number(cli["--seed"]);
if (!Number.isSafeInteger(seed) || seed <= 0 || seed >= 2147483647) fail("Web admission seed is invalid", 64);
if (existsSync(output)) fail("Web admission input already exists", 73);
for (const [file, label] of [[candidatePath, "candidate"], [componentsPath, "components"], [ait, "ait"], [serverBin, "server binary"], [personalBin, "Personal binary"], [communityBin, "Community binary"], [communityCliBin, "Community CLI binary"]]) {
  if (!existsSync(file) || !lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) fail(`${label} must be a regular file`, 66);
}

const candidate = readJson(candidatePath, "candidate");
if (candidate.contract !== "ait.release.operator.pre-tag-candidate-binding/v1" || candidate.status !== "ready_for_immutable_tag" || candidate.release?.channel !== "stable") {
  fail("candidate is not ready for stable Web admission");
}
const components = readJson(componentsPath, "components");
for (const key of ["core", "server", "runner", "python", "node"]) {
  if (!/^SNP-[0-9A-F]{12}$/.test(components[key] ?? "")) fail(`Web admission component Snapshot is invalid: ${key}`);
}

const config = runJson(ait, ["config", "show", "--json"], repository, "AIT configuration");
const status = runJson(ait, ["status", "--json", "--full"], repository, "AIT status");
const queue = runJson(ait, ["queue", "summary", "--remote", "origin", "--json"], repository, "AIT queue");
const worktrees = runJson(ait, ["worktree", "list", "--json"], repository, "AIT worktrees");
const remote = runJson(ait, ["repo", "show", "--remote", "origin", "--json"], repository, "remote Repository");
const personalStatus = runJson(ait, ["status", "--json"], personalRoot, "Personal source status");
if (
  config.document.repo_name !== "ait-web-test" || config.document.repository_index !== 23 ||
  config.document.id_namespace_prefix?.value !== "WT" || config.document.workflow_mode?.value !== "solo_remote" ||
  status.document.current_line !== "main" || status.document.workspace_dirty !== false || status.document.worktree_name !== null ||
  status.document.worktree_hygiene?.manual_review_candidate_count !== 0 || status.document.worktree_hygiene?.stale_count !== 0 ||
  status.document.reconciliation?.total_finding_count !== 0 || queue.document.summary?.attention_required_count !== 0 ||
  queue.document.summary?.dirty_worktree_count !== 0 || queue.document.remote?.reviewer_inbox?.count !== 0 ||
  remote.document.repository?.repository_index !== 23 || remote.document.repository?.repository_name !== "ait-web-test" ||
  remote.document.repository?.namespace !== "WT" || remote.document.repository?.tombstoned !== false
) fail("Web admission entry authority is not clean solo_remote index 23/WT/main");

const acceptance = treeHash(path.join(repository, "acceptance"));
const evidenceRoot = path.dirname(output);
mkdirSync(path.join(evidenceRoot, "evidence"), { recursive: true, mode: 0o700 });
for (const [name, value] of [
  ["starting-config.json", config.raw], ["starting-status.json", status.raw], ["starting-queue.json", queue.raw],
  ["starting-worktrees.json", worktrees.raw], ["remote-repository.json", remote.raw],
]) writeFileSync(path.join(evidenceRoot, "evidence", name), value, { mode: 0o600, flag: "wx" });
writeFileSync(path.join(evidenceRoot, "evidence", "acceptance-set.sha256"), `${acceptance.sha256}\n`, { mode: 0o600, flag: "wx" });

const release = candidate.release;
const authority = candidate.candidate_authority?.record;
const document = {
  contract: "ait.release.web-admission-input/v1",
  id: `WEB-ADMISSION-${release.version}-${new Date().toISOString().slice(0, 10).replaceAll("-", "")}`,
  status: "bound",
  started_at: new Date().toISOString(),
  operator: process.env.USER ?? "unknown",
  release: { ...release, repository: release.repository ?? authority?.publisher?.repository },
  qualification: candidate.qualification,
  candidate_authority_sha256: candidate.candidate_authority?.sha256,
  components: { ...components, personal: personalStatus.document.head_snapshot_id },
  executables: Object.fromEntries([
    ["ait", ait], ["server", serverBin], ["personal", personalBin], ["community", communityBin], ["community_cli", communityCliBin],
  ].map(([name, file]) => [name, { path: file, sha256: sha256(readFileSync(file)) }])),
  web_test: {
    repository_index: 23,
    namespace: "WT",
    head_snapshot_id: status.document.head_snapshot_id,
    acceptance_set_sha256: acceptance.sha256,
    starting_authority_digest: sha256(Buffer.from(`${status.raw}\0${queue.raw}`)),
    entry_gate: { line: "main", dirty: false, changed: 0, is_worktree: false, action_required: false, worktree_candidates: 0 },
  },
  server: { url: cli["--server-url"] },
  browser_evidence: {
    solo_local: { path: "evidence/solo-local-browser.json", sha256: null },
    solo_remote: { path: "evidence/solo-remote-browser.json", sha256: null },
  },
  execution: {
    order: ["foundation", "solo_local", "solo_remote", "randomized_3000", "closeout"],
    random_seed: seed,
    random_groups: 3000,
    result_labels: ["foundation", "solo-local", "solo-remote", `random-${seed}`],
  },
  mutation: { candidate_binaries: false, public_endpoints: false, registry: false, tag: false },
};
if (!/^REL-FAM-[0-9A-F]{16}$/.test(document.release?.id ?? "") || !/^SNP-[0-9A-F]{12}$/.test(document.web_test.head_snapshot_id ?? "")) {
  fail("Web admission release identity is incomplete");
}
writeFileSync(output, `${JSON.stringify(document, null, 2)}\n`, { mode: 0o400, flag: "wx" });
process.stdout.write(`${output}\n`);
