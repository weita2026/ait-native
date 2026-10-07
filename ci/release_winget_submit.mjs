#!/usr/bin/env node

import { verifyPreReview } from './release_winget_pre_review_verify.mjs';

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail("usage: release_winget_submit.mjs {submit|probe|status|pre-review-dispatch|pre-review-bind} --version <semver> --manifests <absolute-dir> --output <absolute-json> [--release-repository <owner/repo>] [--fork <owner/repo>] [--upstream <owner/repo>]", 64);
}

function parseCli(argv) {
  const mode = argv.shift();
  if (!new Set(["submit", "probe", "status", "pre-review-dispatch", "pre-review-bind"]).has(mode)) usage();
  const options = {
    "--release-repository": "weita2026/ait-native",
    "--fork": "weita2026/winget-pkgs",
    "--upstream": "microsoft/winget-pkgs",
  };
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || (options[key] !== undefined && !new Set(["--release-repository", "--fork", "--upstream"]).has(key))) usage();
    options[key] = value;
  }
  for (const key of ["--version", "--manifests", "--output"]) if (!options[key]) usage();
  return { mode, options };
}

function gh(endpoint, method = "GET", body = null) {
  const argv = ["api"];
  if (method !== "GET") argv.push("-X", method);
  argv.push(endpoint);
  if (body !== null) argv.push("--input", "-");
  const result = spawnSync("gh", argv, {
    encoding: "utf8",
    input: body === null ? undefined : `${JSON.stringify(body)}\n`,
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.status !== 0) {
    const detail = result.stderr.trim().replace(/\s+/g, " ").slice(0, 500);
    fail(`GitHub API request failed: ${method} ${endpoint}${detail ? `: ${detail}` : ""}`);
  }
  try { return JSON.parse(result.stdout); } catch { fail(`GitHub API returned invalid JSON: ${endpoint}`); }
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function yamlField(contents, name) {
  const match = contents.match(new RegExp(`^${name}:\\s*["']?([^"'\\r\\n]+)["']?\\s*$`, "m"));
  return match?.[1]?.trim() ?? null;
}

function verifyRemoteFile(repository, branch, target, contents) {
  const document = gh(`repos/${repository}/contents/${target}?ref=${encodeURIComponent(branch)}`);
  const decoded = Buffer.from(String(document.content ?? "").replace(/\s/g, ""), "base64");
  if (!decoded.equals(contents)) fail(`WinGet branch content drifted: ${target}`);
}

function findPull(upstream, owner, branch) {
  const pulls = gh(`repos/${upstream}/pulls?state=all&head=${encodeURIComponent(`${owner}:${branch}`)}&base=master&per_page=100`);
  if (!Array.isArray(pulls) || pulls.length > 1) fail("WinGet pull request identity is ambiguous");
  return pulls[0] ?? null;
}

const { mode, options } = parseCli(process.argv.slice(2));
const version = options["--version"];
if (!/^\d+\.\d+\.\d+$/.test(version)) fail("WinGet version must be stable SemVer", 64);
const manifests = options["--manifests"];
const output = options["--output"];
if (!path.isAbsolute(manifests) || !path.isAbsolute(output)) fail("WinGet paths must be absolute", 64);
const fork = options["--fork"];
const upstream = options["--upstream"];
const releaseRepository = options["--release-repository"];
if (![fork, upstream, releaseRepository].every((row) => /^[^/]+\/[^/]+$/.test(row))) fail("WinGet repository is invalid", 64);
const [forkOwner] = fork.split("/");
const branch = `new-package-Weita.AitNative-${version}`;
const prefix = `manifests/w/Weita/AitNative/${version}`;
const names = ["Weita.AitNative.yaml", "Weita.AitNative.installer.yaml", "Weita.AitNative.locale.en-US.yaml"];
let manifestRoot = manifests;
if (!names.every((name) => existsSync(path.join(manifestRoot, name)))) {
  const candidates = [];
  const walk = (directory) => {
    if (names.every((name) => existsSync(path.join(directory, name)))) candidates.push(directory);
    for (const name of readdirSync(directory)) {
      const child = path.join(directory, name);
      if (statSync(child).isDirectory()) walk(child);
    }
  };
  if (!existsSync(manifests) || !statSync(manifests).isDirectory()) fail("WinGet manifest root is missing", 66);
  walk(manifests);
  if (candidates.length !== 1) fail("WinGet manifest set is missing or ambiguous", 66);
  [manifestRoot] = candidates;
}
const files = names.map((name) => {
  const source = path.join(manifestRoot, name);
  if (!existsSync(source)) fail(`WinGet manifest is missing: ${source}`, 66);
  const contents = readFileSync(source);
  const text = contents.toString("utf8");
  if (yamlField(text, "PackageIdentifier") !== "Weita.AitNative" || yamlField(text, "PackageVersion") !== version) {
    fail(`WinGet manifest identity is invalid: ${name}`);
  }
  return { name, source, target: `${prefix}/${name}`, contents, sha256: sha256(contents) };
});
const installerText = files.find((row) => row.name.endsWith(".installer.yaml")).contents.toString("utf8");
const installerUrls = [...installerText.matchAll(/^\s*InstallerUrl:\s*["']?([^"'\r\n]+)["']?\s*$/gm)].map((row) => row[1].trim());
const installerHashes = [...installerText.matchAll(/^\s*InstallerSha256:\s*([0-9A-Fa-f]{64})\s*$/gm)].map((row) => row[1].toLowerCase());
if (installerUrls.length !== 2 || installerHashes.length !== 2) fail("WinGet installer manifest must contain exactly two URL/hash pairs");
const installers = installerUrls.map((url, index) => ({ url, sha256: installerHashes[index] }));
for (const installer of installers) {
  const expectedPrefix = `https://github.com/${releaseRepository}/releases/download/v${version}/ait-native-${version}-`;
  if (!installer.url.startsWith(expectedPrefix) || !installer.url.endsWith("-pc-windows-msvc.zip")) {
    fail("WinGet installer URL does not bind the immutable release tag");
  }
}

async function verifyPublicAssets() {
  const release = gh(`repos/${releaseRepository}/releases/tags/v${version}`);
  for (const installer of installers) {
    const assetName = new URL(installer.url).pathname.split("/").at(-1);
    const asset = release.assets?.find((row) => row.name === assetName);
    if (!asset || asset.browser_download_url !== installer.url) fail(`WinGet release asset is missing: ${assetName}`);
    const response = await fetch(installer.url);
    if (!response.ok) fail(`WinGet release asset download failed: ${assetName}`);
    const actual = sha256(Buffer.from(await response.arrayBuffer()));
    if (actual !== installer.sha256) fail(`WinGet release asset digest differs from the manifest: ${assetName}`);
  }
}

function validateReceipt() {
  if (!existsSync(output)) return false;
  let receipt;
  try { receipt = JSON.parse(readFileSync(output, "utf8")); } catch { fail("WinGet receipt is invalid JSON"); }
  if (
    receipt.contract !== "ait.release.winget-submission/v1" || receipt.version !== version ||
    receipt.release_repository !== releaseRepository || receipt.fork !== fork || receipt.upstream !== upstream || receipt.branch !== branch ||
    JSON.stringify(receipt.manifests?.map((row) => [row.path, row.sha256])) !== JSON.stringify(files.map((row) => [row.target, row.sha256]))
  ) fail("WinGet receipt differs from the requested submission");
  const pull = findPull(upstream, forkOwner, branch);
  if (!pull || pull.number !== receipt.pull_request.number) fail("WinGet receipt pull request is missing or drifted");
  return { receipt, pull };
}

const releaseId = options['--release-id'] ?? (options['--release-record'] ? JSON.parse(readFileSync(options['--release-record'], 'utf8')).release?.id : undefined);
if (mode === 'pre-review-dispatch') {
  if (!/^REL-FAM-[0-9A-F]{16}$(?![\s\S])/.test(releaseId ?? '')) fail('Pre-review release identity is required');
  if (!/^[a-f0-9]{40}$/.test(options['--head-sha'] ?? '')) fail('Pre-review expected control commit is required');
  const head = gh(`repos/${releaseRepository}/git/ref/heads/main`);
  if (head.object.sha !== options['--head-sha']) fail('Pre-review control main differs from admitted head');
  const encoded = Buffer.from(JSON.stringify(Object.fromEntries(files.map(file => [file.name,file.contents.toString('base64')])))).toString('base64');
  const result = spawnSync('gh',['workflow','run','ait-release-winget-pre-review.yml','--repo',releaseRepository,'--ref','main','-f',`version=${version}`,'-f',`release_id=${releaseId}`,'-f',`manifests_b64=${encoded}`],{stdio:'inherit'});
  process.exit(result.status ?? 1);
}
if (mode === 'pre-review-bind') {
  let receipt;
  try { receipt = verifyPreReview({runId:options['--pre-review-run-id'],releaseId,repository:releaseRepository,version,files}); }
  catch (error) { fail(error.message); }
  writeFileSync(output, JSON.stringify({contract:'ait.release.winget-pre-review-binding/v1',version,...receipt},null,2)+'\n',{flag:'wx',mode:0o600});
  process.exit(0);
}

if (mode === "probe") {
  process.exit(validateReceipt() ? 0 : 1);
}

if (mode === "status") {
  const validated = validateReceipt();
  if (!validated) fail("WinGet submission receipt is missing", 66);
  const pull = gh(`repos/${upstream}/pulls/${validated.pull.number}`);
  if (pull.merged_at) {
    if (!/^[a-f0-9]{40}$/.test(pull.merge_commit_sha ?? "")) fail("WinGet merge identity is invalid");
    for (const file of files) verifyRemoteFile(upstream, pull.merge_commit_sha, file.target, file.contents);
  }
  const status = {
    contract: "ait.release.winget-status/v1",
    version,
    submission_sha256: sha256(readFileSync(output)),
    pull_request: { number: pull.number, url: pull.html_url, state: pull.state, merged_at: pull.merged_at, merge_commit_sha: pull.merge_commit_sha },
    status: pull.merged_at ? "merged" : pull.state === "closed" ? "rejected" : "submitted",
  };
  if (options["--status-output"]) {
    const target = options["--status-output"];
    if (!path.isAbsolute(target)) fail("WinGet status path must be absolute");
    const history = path.join(path.dirname(target), "winget-observations");
    mkdirSync(history, { recursive: true, mode: 0o700 });
    const bytes = JSON.stringify(status, null, 2) + "\n";
    const observation = path.join(history, sha256(bytes) + ".json");
    if (!existsSync(observation)) writeFileSync(observation, bytes, { flag: "wx", mode: 0o600 });
    if (status.status === "rejected") fail("WinGet pull request closed without merge");
    if (status.status !== "merged") fail("WinGet review is pending", 75);
    if (existsSync(target)) {
      if (readFileSync(target, "utf8") !== bytes) fail("WinGet completion evidence drifted");
    } else writeFileSync(target, bytes, { flag: "wx", mode: 0o600 });
  }
  process.stdout.write(JSON.stringify(status, null, 2) + "\n");
  process.exit(0);
}

if (existsSync(output)) {
  const validated = validateReceipt();
  process.stdout.write(`WinGet ${version}: PR ${validated.pull.number} already submitted\n`);
  process.exit(0);
}

let preReview;
try {
  const bound = options['--pre-review-record'] ? JSON.parse(readFileSync(options['--pre-review-record'],'utf8')) : null;
  preReview = verifyPreReview({runId: options['--pre-review-run-id'] ?? (bound ? String(bound.workflow_run_id) : undefined), releaseId, repository: releaseRepository, version, files});
} catch (error) { fail(error.message); }

await verifyPublicAssets();

let pull = findPull(upstream, forkOwner, branch);
if (!pull) {
  let branchRef = null;
  const branchLookup = spawnSync("gh", ["api", `repos/${fork}/git/ref/heads/${branch}`], { encoding: "utf8" });
  if (branchLookup.status === 0) {
    branchRef = JSON.parse(branchLookup.stdout);
  } else {
    const forkRef = gh(`repos/${fork}/git/ref/heads/master`);
    const upstreamRef = gh(`repos/${upstream}/git/ref/heads/master`);
    const ancestry = gh(`repos/${upstream}/compare/${forkRef.object.sha}...${upstreamRef.object.sha}`);
    if (!new Set(["ahead", "identical"]).has(ancestry.status)) {
      fail("WinGet fork master is not an ancestor of upstream master");
    }
    const forkCommit = gh(`repos/${fork}/git/commits/${forkRef.object.sha}`);
    const treeRows = [];
    for (const file of files) {
      const blob = gh(`repos/${fork}/git/blobs`, "POST", { content: file.contents.toString("base64"), encoding: "base64" });
      treeRows.push({ path: file.target, mode: "100644", type: "blob", sha: blob.sha });
    }
    const tree = gh(`repos/${fork}/git/trees`, "POST", { base_tree: forkCommit.tree.sha, tree: treeRows });
    const commit = gh(`repos/${fork}/git/commits`, "POST", {
      message: `New package: Weita.AitNative version ${version}`,
      tree: tree.sha,
      parents: [forkRef.object.sha],
    });
    branchRef = gh(`repos/${fork}/git/refs`, "POST", { ref: `refs/heads/${branch}`, sha: commit.sha });
  }
  for (const file of files) verifyRemoteFile(fork, branch, file.target, file.contents);
  pull = gh(`repos/${upstream}/pulls`, "POST", {
    title: `New package: Weita.AitNative version ${version}`,
    head: `${forkOwner}:${branch}`,
    base: "master",
    body: `Adds the frozen Weita.AitNative ${version} portable manifests for x64 and arm64. The installer hashes are bound to the immutable v${version} release assets.`,
  });
}
for (const file of files) verifyRemoteFile(fork, branch, file.target, file.contents);

const receipt = {
  contract: "ait.release.winget-submission/v1",
  version,
  identity: "Weita.AitNative",
  release_repository: releaseRepository,
  fork,
  upstream,
  branch,
  manifests: files.map((file) => ({ path: file.target, sha256: file.sha256 })),
  pull_request: { number: pull.number, url: pull.html_url, state: pull.state },
  status: pull.merged_at ? "merged" : "submitted",
  pre_review: preReview,
  submitted_at: new Date().toISOString(),
};
writeFileSync(output, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600, flag: "wx" });
process.stdout.write(`WinGet ${version}: PR ${pull.number} submitted\n`);
