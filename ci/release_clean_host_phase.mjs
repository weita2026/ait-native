#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const PHASE_CONTRACT = "ait.release.clean-host.phase/v1";
const MATRIX_CONTRACT = "ait.release.clean-host.matrix/v1";
const MAX_CAPTURE = 16 * 1024 * 1024;
const COMMAND_TIMEOUT_MS = 10 * 60 * 1000;
const CHECKSUM_ASSET_NAME = /^[A-Za-z0-9][A-Za-z0-9._@+~-]*$/;
const MATRIX_ROW_COUNTS = {
  "distribution-target-32-2026-08-17.2": 32,
  "distribution-target-runner-bundle-32-2026-08-26.1": 32,
};

function usage(message) {
  if (message) {
    process.stderr.write(`${message}\n`);
  }
  process.stderr.write(
    "usage: release_clean_host_phase.mjs run --matrix <json> --row-id <id> " +
      "--config <json> --status <json> --phase <install|upgrade> " +
      "--prior-version <semver> --prior-python-version <pep440> " +
      "--output <absolute-json>\n",
  );
  process.exit(64);
}

function fail(message, code = 65) {
  const error = new Error(message);
  error.exitCode = code;
  throw error;
}

function parseOptions(argv) {
  const allowed = [
    "matrix",
    "row-id",
    "config",
    "status",
    "phase",
    "prior-version",
    "prior-python-version",
    "output",
  ];
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined) {
      usage("clean-host phase option requires a value");
    }
    const name = key.slice(2);
    if (!allowed.includes(name) || Object.hasOwn(options, name)) {
      usage(`unsupported or repeated clean-host phase option: ${key}`);
    }
    options[name] = value;
  }
  for (const name of allowed) {
    if (!Object.hasOwn(options, name)) {
      usage(`missing clean-host phase option: --${name}`);
    }
  }
  if (!["install", "upgrade"].includes(options.phase)) {
    usage("--phase must be install or upgrade");
  }
  return options;
}

function requireRegularFile(file, label) {
  let stat;
  try {
    stat = lstatSync(file);
  } catch {
    fail(`${label} is unavailable: ${file}`, 66);
  }
  if (!stat.isFile() || stat.isSymbolicLink()) {
    fail(`${label} must be a regular non-symlink file: ${file}`, 66);
  }
}

function requireExecutableFile(file, label) {
  let target;
  let stat;
  try {
    target = realpathSync(file);
    stat = statSync(target);
  } catch {
    fail(`${label} is unavailable: ${file}`, 66);
  }
  if (!stat.isFile()) {
    fail(`${label} must resolve to a regular file: ${file}`, 66);
  }
  if (process.platform !== "win32" && (stat.mode & 0o111) === 0) {
    fail(`${label} is not executable: ${file}`, 66);
  }
  return target;
}

function readJson(file, label) {
  requireRegularFile(file, label);
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    fail(`${label} is not valid JSON: ${error.message}`);
  }
}

function requireNewOutput(file) {
  if (!path.isAbsolute(file)) {
    fail("clean-host phase output must be absolute", 64);
  }
  if (existsSync(file)) {
    fail(`clean-host phase output already exists: ${file}`, 73);
  }
  const parent = path.dirname(file);
  const stat = lstatSync(parent);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail("clean-host phase output parent must be a real directory", 73);
  }
}

function sortedValue(value) {
  if (Array.isArray(value)) {
    return value.map(sortedValue);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortedValue(value[key])]),
    );
  }
  return value;
}

function encoded(value) {
  return `${JSON.stringify(sortedValue(value), null, 2)}\n`;
}

function sha256Bytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  requireRegularFile(file, "checksum input");
  return sha256Bytes(readFileSync(file));
}

const candidateAssetIndexes = new Map();

function candidateRoot(status) {
  if (status.status !== "frozen_candidate_pending_clean_host") {
    return null;
  }
  const root = process.env.AIT_CLEAN_HOST_CANDIDATE_ROOT ?? "";
  if (!path.isAbsolute(root)) {
    fail("prepublish clean-host phase requires an absolute candidate root", 64);
  }
  let stat;
  try {
    stat = lstatSync(root);
  } catch {
    fail(`prepublish candidate root is unavailable: ${root}`, 66);
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail("prepublish candidate root must be a real directory", 66);
  }
  const canonical = realpathSync(root);
  const receipt = path.join(canonical, "ait-release.prepublish-stage.json");
  requireRegularFile(receipt, "prepublish candidate stage receipt");
  if (sha256File(receipt) !== status.candidate?.stage_receipt_sha256) {
    fail("prepublish candidate stage receipt digest differs from its status");
  }
  return canonical;
}

function candidateAssetIndex(root) {
  const assets = path.join(root, "assets");
  if (candidateAssetIndexes.has(assets)) {
    return candidateAssetIndexes.get(assets);
  }
  const checksumFile = path.join(assets, "SHA256SUMS");
  requireRegularFile(checksumFile, "prepublish candidate asset checksums");
  const index = new Map();
  for (const line of readFileSync(checksumFile, "utf8").split(/\r?\n/)) {
    if (!line) {
      continue;
    }
    const match = line.match(/^([0-9a-f]{64})  (.+)$/);
    if (!match || !CHECKSUM_ASSET_NAME.test(match[2]) || index.has(match[2])) {
      fail(`prepublish candidate checksums contain an invalid or duplicate row: ${line}`);
    }
    index.set(match[2], match[1]);
  }
  candidateAssetIndexes.set(assets, index);
  return index;
}

function localCandidateAsset(root, name) {
  if (path.basename(name) !== name) {
    fail(`prepublish candidate asset name is unsafe: ${name}`);
  }
  const expected = candidateAssetIndex(root).get(name);
  if (!expected) {
    fail(`prepublish candidate asset is not checksummed: ${name}`);
  }
  const file = path.join(root, "assets", name);
  requireRegularFile(file, `prepublish candidate asset ${name}`);
  if (sha256File(file) !== expected) {
    fail(`prepublish candidate asset digest drifted: ${name}`);
  }
  return file;
}

function tail(value, size = 4096) {
  const text = String(value ?? "");
  return text.length <= size ? text : text.slice(text.length - size);
}

function shellCommandText(command, args) {
  return [command, ...args]
    .map((value) => (/^[A-Za-z0-9_./:@%+=,-]+$/.test(value) ? value : JSON.stringify(value)))
    .join(" ");
}

class Recorder {
  constructor() {
    this.commands = [];
    this.observations = {};
  }

  run(command, args = [], options = {}) {
    const result = spawnSync(command, args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
      encoding: "utf8",
      maxBuffer: MAX_CAPTURE,
      timeout: options.timeout ?? COMMAND_TIMEOUT_MS,
      windowsHide: true,
    });
    const status = result.status ?? (result.error ? 127 : 0);
    const stdout = result.stdout ?? "";
    const stderr = result.stderr ?? result.error?.message ?? "";
    this.commands.push({
      label: options.label ?? "command",
      command: shellCommandText(command, args),
      cwd: options.cwd ?? null,
      exit_status: status,
      stdout_sha256: sha256Bytes(stdout),
      stderr_sha256: sha256Bytes(stderr),
      stdout_tail: tail(stdout),
      stderr_tail: tail(stderr),
    });
    const allowed = options.allowed ?? [0];
    if (options.recordOnly !== true && !allowed.includes(status)) {
      // The head of the output carries the actual error line for commands
      // that append long usage text; keep the head and tail halves of the
      // excerpt budget so the failure stays diagnosable.
      const text = stderr || stdout;
      const excerpt =
        text.length <= 1200
          ? text
          : `${text.slice(0, 600)}\n[...]\n${tail(text, 600)}`;
      fail(`${options.label ?? command} failed with ${status}: ${excerpt}`);
    }
    return { status, stdout, stderr };
  }
}

function commandExists(command) {
  const locator = process.platform === "win32" ? "where.exe" : "which";
  const result = spawnSync(locator, [command], {
    encoding: "utf8",
    windowsHide: true,
  });
  return result.status === 0 ? result.stdout.trim().split(/\r?\n/)[0] : null;
}

function requireCommand(command) {
  const resolved = commandExists(command);
  if (!resolved) {
    fail(`required clean-host command is unavailable: ${command}`, 69);
  }
  return resolved;
}

function nativePlatform(row) {
  const expectedPlatform = { macos: "darwin", linux: "linux", windows: "win32" }[row.os];
  const expectedArch = row.architecture === "x86_64" ? "x64" : "arm64";
  if (process.platform !== expectedPlatform || process.arch !== expectedArch) {
    fail(
      `runner target mismatch for ${row.id}: expected ${expectedPlatform}/${expectedArch}, got ${process.platform}/${process.arch}`,
    );
  }
}

function githubRunner(row) {
  const label = process.env.AIT_CLEAN_HOST_RUNNER_LABEL ?? "";
  const runId = process.env.GITHUB_RUN_ID ?? "";
  const runAttempt = process.env.GITHUB_RUN_ATTEMPT ?? "";
  const job = process.env.GITHUB_JOB ?? "";
  if (
    process.env.GITHUB_ACTIONS !== "true" ||
    label !== row.runner ||
    !/^[1-9][0-9]*$/.test(runId) ||
    !/^[1-9][0-9]*$/.test(runAttempt) ||
    !job
  ) {
    fail("phase execution is not bound to the declared GitHub-hosted matrix job");
  }
  return {
    label,
    target_verified: true,
    github_hosted: true,
    run_id: runId,
    run_attempt: runAttempt,
    job,
    runner_name: process.env.RUNNER_NAME ?? null,
    runner_os: process.env.RUNNER_OS ?? null,
    runner_arch: process.env.RUNNER_ARCH ?? null,
    image_os: process.env.ImageOS ?? null,
    image_version: process.env.ImageVersion ?? null,
    node_platform: process.platform,
    node_architecture: process.arch,
  };
}

function validateInputs(options, matrix, config, status) {
  const expectedRows = MATRIX_ROW_COUNTS[matrix.matrix_revision];
  if (
    matrix.contract !== MATRIX_CONTRACT ||
    !expectedRows ||
    matrix.row_count !== expectedRows ||
    !Array.isArray(matrix.rows) ||
    matrix.rows.length !== expectedRows
  ) {
    fail("phase matrix is not an admitted exact-row authority");
  }
  const row = matrix.rows.find((candidate) => candidate.id === options["row-id"]);
  if (!row) {
    fail("phase row is absent from the clean-host matrix");
  }
  const frozenCandidate =
    status.contract === "ait.release.prepublish.candidate/v1" &&
    status.status === "frozen_candidate_pending_clean_host" &&
    status.release?.source_commit === config.release?.source_commit &&
    /^([0-9a-f]{64})$/.test(status.candidate?.stage_receipt_sha256 ?? "");
  if (
    config.contract !== "ait.release.family.endpoints/v1" ||
    !frozenCandidate ||
    status.release?.id !== config.release?.id ||
    status.release?.version !== config.release?.version ||
    status.release?.tag !== config.release?.tag ||
    matrix.release?.version !== config.release?.version ||
    matrix.release?.tag !== config.release?.tag
  ) {
    fail("phase config, status, and matrix do not bind one pending candidate");
  }
  candidateRoot(status);
  const candidateVersion = config.release.version;
  const candidatePython = config.release.python_version;
  const priorVersion = options["prior-version"];
  const priorPython = options["prior-python-version"];
  if (!/^\d+\.\d+\.\d+(?:-rc\.[1-9]\d*)?$/.test(priorVersion)) {
    fail("prior release version is not an exact supported SemVer");
  }
  const expectedPriorPython = priorVersion.replace(/-rc\.([1-9]\d*)$/, "rc$1");
  if (priorPython !== expectedPriorPython || priorVersion === candidateVersion) {
    fail("prior Python version or candidate separation is invalid");
  }
  if (candidatePython !== candidateVersion.replace(/-rc\.([1-9]\d*)$/, "rc$1")) {
    fail("candidate Python version is inconsistent");
  }
  return row;
}

function releaseBinding(config, status, options) {
  const binding = {
    id: config.release.id,
    version: config.release.version,
    python_version: config.release.python_version,
    channel: config.release.channel,
    tag: config.release.tag,
    source_commit: config.release.source_commit,
    endpoint_config_sha256: sha256File(options.config),
    operator_status_sha256: sha256File(options.status),
  };
  const artifactDigest = process.env.AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST ?? "";
  if (!/^sha256:[0-9a-f]{64}$/.test(artifactDigest)) {
    fail("prepublish phase lacks the immutable candidate artifact digest", 64);
  }
  binding.verification_stage = "prepublication";
  binding.candidate_stage_receipt_sha256 = status.candidate.stage_receipt_sha256;
  binding.candidate_artifact_digest = artifactDigest;
  return binding;
}

async function fetchBytes(url, label) {
  // A stalled connection to the release or repository host has held a leg
  // until the job timeout; bound every attempt and retry transient
  // failures, while HTTP error statuses keep failing closed immediately.
  const attempts = 3;
  let lastError = null;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const response = await fetch(url, {
        redirect: "follow",
        headers: { "User-Agent": "ait-native-clean-host/v1" },
        signal: AbortSignal.timeout(120_000),
      });
      if (!response.ok) {
        fail(`${label} returned HTTP ${response.status}: ${url}`, 69);
      }
      return Buffer.from(await response.arrayBuffer());
    } catch (error) {
      if (error?.exitCode !== undefined) {
        throw error;
      }
      lastError = error;
    }
  }
  fail(`${label} did not complete after ${attempts} bounded attempts: ${url}: ${lastError}`, 69);
}

function releaseDownloadUrl(repository, tag, name) {
  return `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
}

const releaseAssetIndexes = new Map();

async function releaseAssetIndex(repository, tag) {
  const key = `${repository}@${tag}`;
  if (releaseAssetIndexes.has(key)) {
    return releaseAssetIndexes.get(key);
  }
  const bytes = await fetchBytes(
    releaseDownloadUrl(repository, tag, "SHA256SUMS"),
    `release checksum inventory ${tag}`,
  );
  const index = new Map();
  for (const line of bytes.toString("utf8").split(/\r?\n/)) {
    if (!line) {
      continue;
    }
    const match = line.match(/^([0-9a-f]{64})  (.+)$/);
    if (!match || !CHECKSUM_ASSET_NAME.test(match[2]) || index.has(match[2])) {
      fail(`release checksum inventory contains an invalid or duplicate row: ${line}`);
    }
    index.set(match[2], match[1]);
  }
  if (index.size === 0) {
    fail(`release checksum inventory is empty: ${tag}`);
  }
  releaseAssetIndexes.set(key, index);
  return index;
}

function verifyCandidateTag(config, recorder) {
  const repository = config.endpoints.github.repository;
  const tag = config.release.tag;
  const remote = `https://github.com/${repository}.git`;
  const result = recorder.run(
    requireCommand("git"),
    ["-c", "credential.helper=", "ls-remote", "--tags", remote, tag, `${tag}^{}`],
    {
      label: "candidate anonymous Git tag readback",
      env: { ...process.env, GIT_ASKPASS: process.platform === "win32" ? "" : "/usr/bin/false", GIT_TERMINAL_PROMPT: "0" },
    },
  );
  const rows = result.stdout
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => line.split(/\s+/));
  const peeled = rows.filter((row) => row[1] === `refs/tags/${tag}^{}`);
  const direct = rows.filter((row) => row[1] === `refs/tags/${tag}`);
  if (peeled.length !== 1 || direct.length !== 1) {
    fail("candidate public tag is missing, ambiguous, or not annotated");
  }
  const commit = peeled[0][0];
  if (commit !== config.release.source_commit) {
    fail("candidate public tag does not resolve to the configured source commit");
  }
}

async function downloadReleaseAsset(repository, tag, name, destination) {
  const index = await releaseAssetIndex(repository, tag);
  const expected = index.get(name);
  if (!expected) {
    fail(`GitHub Release checksum inventory does not declare asset: ${name}`);
  }
  const bytes = await fetchBytes(
    releaseDownloadUrl(repository, tag, name),
    `release asset ${name}`,
  );
  const actual = sha256Bytes(bytes);
  if (actual !== expected) {
    fail(`GitHub Release asset digest differs after download: ${name}`);
  }
  writeFileSync(destination, bytes, { mode: 0o755 });
  if (process.platform !== "win32") {
    chmodSync(destination, 0o755);
  }
  return { name, digest: `sha256:${actual}`, size_bytes: bytes.length };
}

function commandSpec(command, argsPrefix = [], environment = process.env) {
  return { command, argsPrefix, environment };
}

function runSpec(recorder, spec, args, options = {}) {
  return recorder.run(spec.command, [...spec.argsPrefix, ...args], {
    ...options,
    env: { ...process.env, ...spec.environment, ...(options.env ?? {}) },
  });
}

function exactVersion(recorder, spec, component, version) {
  const result = runSpec(recorder, spec, ["--version"], {
    label: `${component} exact version`,
  });
  const expected = `${component} ${version}`;
  if (result.stdout.trim() !== expected) {
    fail(`installed ${component} version differs: expected ${expected}, got ${result.stdout.trim()}`);
  }
}

function jsonSpec(recorder, spec, args, options = {}) {
  const result = runSpec(recorder, spec, args, options);
  try {
    return JSON.parse(result.stdout);
  } catch {
    fail(`${options.label ?? "AIT command"} did not emit one JSON document`);
  }
}

function generatedWorkflowCurrent(agents) {
  for (const forbidden of ["workflow tier", "--profile quick", "workflow local-land"]) {
    if (agents.includes(forbidden)) {
      fail(`generated AGENTS.md retained removed workflow spelling: ${forbidden}`);
    }
  }
  for (const required of ["ait task start --from", "ait snapshot create", "ait task finish"]) {
    if (!agents.includes(required)) {
      fail(`generated AGENTS.md is missing current workflow spelling: ${required}`);
    }
  }
}

function knownRc10WindowsInitRegression(error, priorVersion) {
  if (process.platform !== "win32" || priorVersion !== "1.0.0-rc.10") {
    return false;
  }
  return /^prior ait init failed with 1: Error: sync Binary DB file .*[/\\]\.ait-init-[^/\\]+[/\\]binary-db[/\\]line_name_payload\.bin: Failed to sync file .*[/\\]\.ait-init-[^/\\]+[/\\]binary-db[/\\]line_name_payload\.bin: Access is denied\. \(os error 5\)\s*$/s.test(
    error.message,
  );
}

function initializePriorState(recorder, aitSpec, root, priorVersion) {
  mkdirSync(root, { recursive: false, mode: 0o755 });
  try {
    jsonSpec(recorder, aitSpec, ["init", "--json"], {
      cwd: root,
      label: "prior ait init",
    });
  } catch (error) {
    if (!knownRc10WindowsInitRegression(error, priorVersion)) {
      throw error;
    }
    if (
      existsSync(path.join(root, ".ait")) ||
      readdirSync(root).some((entry) => entry.startsWith(".ait-init-"))
    ) {
      fail("known RC10 Windows init regression left partial repository authority behind");
    }
    return {
      available: false,
      expected_regression: "rc10_windows_read_only_fsync",
    };
  }
  const configPath = path.join(root, ".ait", "config.json");
  requireRegularFile(configPath, "prior repository config");
  return { available: true, config_sha256: sha256File(configPath) };
}

function firstLand(recorder, aitSpec, root, expectedText, priorState = null) {
  if (!existsSync(root)) {
    mkdirSync(root, { recursive: false, mode: 0o755 });
  }
  const configPath = path.join(root, ".ait", "config.json");
  if (!existsSync(configPath)) {
    jsonSpec(recorder, aitSpec, ["init", "--json"], {
      cwd: root,
      label: "candidate ait init",
    });
  }
  requireRegularFile(configPath, "candidate repository config");
  if (
    priorState?.available === true &&
    sha256File(configPath) !== priorState.config_sha256
  ) {
    fail("candidate upgrade replaced the prior repository configuration");
  }
  const sprintDirectory = path.join(root, "docs", "sprints");
  mkdirSync(sprintDirectory, { recursive: true, mode: 0o755 });
  const sprintPath = path.join(sprintDirectory, "clean_host.md");
  const sprintText =
    "# Clean Host First Land [plan-ref: clean-host/root]\n\n" +
    "## Work\n\n" +
    "- [ ] Materialize the exact clean-host file. [ref: clean-host/first-land]\n";
  writeFileSync(sprintPath, sprintText, { encoding: "utf8", mode: 0o644 });
  const started = jsonSpec(
    recorder,
    aitSpec,
    [
      "task",
      "start",
      "--from",
      "docs/sprints/clean_host.md#clean-host/first-land",
      "--intent",
      "Prove exact clean-host first land",
      "--json",
    ],
    { cwd: root, label: "candidate task start" },
  );
  const taskId = started.task_id;
  const worktree = started.edit_root;
  if (!/^LT-[0-9]{4,}$/.test(taskId ?? "") || !path.isAbsolute(worktree ?? "")) {
    fail("candidate task start returned no exact Task or worktree");
  }
  const landedFile = path.join(worktree, "first-land.txt");
  writeFileSync(landedFile, expectedText, { encoding: "utf8", mode: 0o644 });
  let landed = jsonSpec(
    recorder,
    aitSpec,
    [
      "task",
      "finish",
      taskId,
      "--message",
      "Clean-host first Snapshot",
      "--local",
      "--json",
    ],
    { cwd: worktree, label: "candidate task finish", allowed: [0, 2] },
  );
  const snapshot = jsonSpec(
    recorder,
    aitSpec,
    ["snapshot", "show", landed.landed_snapshot_id, "--json"],
    { cwd: root, label: "candidate integrated Snapshot show" },
  );
  if (!/^SNP-[0-9A-F]{12}$/.test(snapshot.snapshot_id ?? "")) {
    fail("candidate Snapshot identity is invalid");
  }
  if (snapshot.parent_snapshot_id !== null) {
    fail("clean-host first land did not author the first Snapshot on an empty default Line");
  }
  let resumedCloseout = false;
  if (
    landed.closeout?.task_status === "completed" &&
    landed.closeout?.status !== "complete"
  ) {
    // Finish consumes the bound worktree's Line head, so it must start
    // inside that worktree; Windows cannot remove a directory that is still
    // a process working directory, so the closeout reports partial with
    // exit 2. The closeout contract's idempotent_phase_resume finishes the exact
    // closeout from the repository root, where no process holds the
    // worktree.
    landed = jsonSpec(
      recorder,
      aitSpec,
      ["task", "finish", taskId, "--local", "--json"],
      { cwd: root, label: "candidate task finish closeout resume" },
    );
    resumedCloseout = true;
  }
  if (
    landed.closeout?.task_status !== "completed" ||
    landed.closeout?.status !== "complete" ||
    landed.closeout?.plan_status !== "synced"
  ) {
    fail("candidate first land did not complete exact Task and Plan closeout");
  }
  if (readFileSync(path.join(root, "first-land.txt"), "utf8") !== expectedText) {
    fail("candidate first land reported success without materializing the file");
  }
  if (!readFileSync(sprintPath, "utf8").includes("- [x] Materialize the exact clean-host file.")) {
    fail("candidate first land did not close the exact sprint checklist item");
  }
  if (resumedCloseout && existsSync(worktree)) {
    // The first attempt could not remove the bound worktree while it was
    // the process working directory; the resumed closeout already released
    // the binding, so the leftover directory is orphaned rehearsal debris.
    rmSync(worktree, { recursive: true, force: true });
  }
  if (existsSync(worktree)) {
    fail("candidate first land left its bound worktree behind");
  }
  const agents = readFileSync(path.join(root, "AGENTS.md"), "utf8");
  generatedWorkflowCurrent(agents);
  return { root, task_id: taskId, snapshot_id: snapshot.snapshot_id };
}

async function reservePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close((error) => (error ? reject(error) : resolve(address.port)));
    });
  });
}

async function pollHealth(url, child = null) {
  const deadline = Date.now() + 45_000;
  let lastError = "not attempted";
  while (Date.now() < deadline) {
    if (child && child.exitCode !== null) {
      fail(`ait-server exited before health check with ${child.exitCode}`);
    }
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = error.message;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  fail(`ait-server health check timed out: ${lastError}`);
}

async function foregroundServer(recorder, serverSpec, root) {
  const port = await reservePort();
  const dataRoot = path.join(root, "server-data");
  const args = [
    ...serverSpec.argsPrefix,
    "--data",
    dataRoot,
    "--listen",
    `127.0.0.1:${port}`,
    "--init-if-missing",
    "--defer-ci-admission",
  ];
  const child = spawn(serverSpec.command, args, {
    env: { ...process.env, ...serverSpec.environment },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  let lifecycleError = null;
  try {
    await pollHealth(`http://127.0.0.1:${port}/healthz`, child);
  } catch (error) {
    lifecycleError = error;
  } finally {
    child.kill("SIGTERM");
    await new Promise((resolve) => {
      const timer = setTimeout(() => {
        child.kill("SIGKILL");
        resolve();
      }, 10_000);
      child.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
    recorder.commands.push({
      label: "explicit foreground server lifecycle",
      command: shellCommandText(serverSpec.command, args),
      cwd: null,
      exit_status: child.exitCode,
      stdout_sha256: sha256Bytes(stdout),
      stderr_sha256: sha256Bytes(stderr),
      stdout_tail: tail(stdout),
      stderr_tail: tail(stderr),
    });
  }
  if (lifecycleError) {
    throw lifecycleError;
  }
  requireRegularFile(
    path.join(dataRoot, "binary-v0", "active.json"),
    "server active-generation marker",
  );
  return dataRoot;
}

function assertNoServerProcess(recorder) {
  if (process.platform === "win32") {
    const result = recorder.run(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "if (Get-Process -Name ait-server -ErrorAction SilentlyContinue) { exit 9 }",
      ],
      { label: "server inactive process check" },
    );
    return result.status === 0;
  }
  recorder.run("pgrep", ["-x", "ait-server"], {
    label: "server inactive process check",
    allowed: [1],
  });
  return true;
}

function assertNoRunnerProcess(recorder) {
  if (process.platform === "win32") {
    const result = recorder.run(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "if (Get-Process -Name ait-runner -ErrorAction SilentlyContinue) { exit 9 }",
      ],
      { label: "runner inactive process check" },
    );
    return result.status === 0;
  }
  recorder.run("pgrep", ["-x", "ait-runner"], {
    label: "runner inactive process check",
    allowed: [1],
  });
  return true;
}

function assertProductProcessesInactive(recorder, includesRunner) {
  assertNoServerProcess(recorder);
  if (includesRunner) {
    assertNoRunnerProcess(recorder);
  }
}

async function githubContext(config, row, version, root, recorder, candidateStage = null) {
  const repository = config.endpoints.github.repository;
  const tag = `v${version}`;
  const bin = path.join(root, "github-bin");
  mkdirSync(bin, { recursive: true, mode: 0o755 });
  const assets = [];
  for (const component of ["ait", "ait-agent", "ait-agent-worker", "ait-server", "ait-runner"]) {
    const suffix = row.executable_suffix;
    const name = `${component}-${version}-${row.target}${suffix}`;
    const destination = path.join(bin, `${component}${suffix}`);
    if (candidateStage) {
      const source = localCandidateAsset(candidateStage, name);
      const bytes = readFileSync(source);
      writeFileSync(destination, bytes, { mode: 0o755 });
      if (process.platform !== "win32") {
        chmodSync(destination, 0o755);
      }
      assets.push({
        name,
        digest: `sha256:${sha256Bytes(bytes)}`,
        size_bytes: bytes.length,
        source: "frozen_candidate_stage",
      });
    } else {
      assets.push(await downloadReleaseAsset(repository, tag, name, destination));
    }
  }
  const ait = commandSpec(path.join(bin, `ait${row.executable_suffix}`));
  const server = commandSpec(path.join(bin, `ait-server${row.executable_suffix}`));
  const runner = commandSpec(path.join(bin, `ait-runner${row.executable_suffix}`));
  recorder.observations.github_assets = assets;
  return {
    ait,
    server,
    runner,
    origin: bin,
    uninstall() {
      rmSync(bin, { recursive: true, force: false });
      if (existsSync(bin)) {
        fail("GitHub native uninstall retained its bounded install directory");
      }
    },
  };
}

function pythonCommand() {
  const configured = process.env.AIT_CLEAN_HOST_PYTHON ?? "";
  if (configured) {
    if (!path.isAbsolute(configured)) {
      fail("configured clean-host Python must be an absolute path", 64);
    }
    return requireExecutableFile(configured, "configured clean-host Python");
  }
  if (process.platform === "win32") {
    fail("Windows clean-host Python must come from the explicit setup-python output", 69);
  }
  return commandExists("python3") ?? commandExists("python") ?? fail("Python is unavailable", 69);
}

function venvPaths(root) {
  const python =
    process.platform === "win32"
      ? path.join(root, "Scripts", "python.exe")
      : path.join(root, "bin", "python");
  const scripts = process.platform === "win32" ? path.join(root, "Scripts") : path.join(root, "bin");
  return { python, scripts };
}

function createVenv(recorder, root) {
  recorder.run(pythonCommand(), ["-m", "venv", root], { label: "create isolated Python venv" });
  const paths = venvPaths(root);
  requireExecutableFile(paths.python, "venv Python");
  return paths;
}

function pipInstall(recorder, python, pythonVersion, upgrade = false, candidateStage = null) {
  const args = [
    "-m",
    "pip",
    "install",
    "--disable-pip-version-check",
    "--no-input",
  ];
  if (upgrade) {
    args.push("--upgrade");
  }
  if (candidateStage) {
    args.push("--no-index", "--find-links", path.join(candidateStage, "assets"));
  }
  args.push(`ait-native==${pythonVersion}`);
  recorder.run(python, args, { label: upgrade ? "PyPI exact upgrade" : "PyPI exact install" });
}

function pythonContext(recorder, root, pythonVersion, upgrade = false, candidateStage = null) {
  const paths = existsSync(root) ? venvPaths(root) : createVenv(recorder, root);
  pipInstall(recorder, paths.python, pythonVersion, upgrade, candidateStage);
  const suffix = process.platform === "win32" ? ".exe" : "";
  const aitPath = path.join(paths.scripts, `ait${suffix}`);
  const serverPath = path.join(paths.scripts, `ait-server${suffix}`);
  requireRegularFile(aitPath, "PyPI ait command");
  requireRegularFile(serverPath, "PyPI ait-server command");
  return {
    ait: commandSpec(aitPath),
    server: commandSpec(serverPath),
    origin: paths.scripts,
    binding() {
      const result = recorder.run(
        paths.python,
        [
          "-c",
          "import json; from ait_python import NativeRuntime; print(json.dumps(NativeRuntime().binding_info(), sort_keys=True))",
        ],
        { label: "PyPI direct PyO3 binding" },
      );
      const payload = JSON.parse(result.stdout);
      if (
        payload.runtime_authority !== "rust" ||
        payload.python_binding !== "pyo3" ||
        payload.process_transport_allowed !== false ||
        payload.version !== pythonVersion.replace(/rc([1-9]\d*)$/, "-rc.$1")
      ) {
        fail("PyPI direct binding metadata is not Rust/PyO3 authority");
      }
    },
    uninstall() {
      recorder.run(
        paths.python,
        ["-m", "pip", "uninstall", "--yes", "ait-native"],
        { label: "PyPI uninstall" },
      );
      if (existsSync(aitPath) || existsSync(serverPath)) {
        fail("PyPI uninstall retained exposed commands");
      }
    },
  };
}

function npmPackageRoot(prefix, packageName) {
  const match = packageName.match(/^(@[A-Za-z0-9._-]+)\/([A-Za-z0-9._-]+)$/);
  if (!match) {
    fail(`npm clean-host package name is unsupported: ${packageName}`);
  }
  const modules =
    process.platform === "win32"
      ? path.join(prefix, "node_modules")
      : path.join(prefix, "lib", "node_modules");
  return path.join(modules, match[1], match[2]);
}

function npmCommandSpec() {
  const npm = requireCommand(process.platform === "win32" ? "npm.cmd" : "npm");
  if (process.platform !== "win32") {
    return commandSpec(npm);
  }
  const node = requireCommand("node");
  const npmCli = path.join(path.dirname(realpathSync(npm)), "node_modules", "npm", "bin", "npm-cli.js");
  requireRegularFile(npmCli, "Windows npm CLI entrypoint");
  return commandSpec(node, [npmCli]);
}

function npmTargetSuffix(row) {
  const suffix = {
    "aarch64-apple-darwin": "darwin-arm64",
    "x86_64-apple-darwin": "darwin-x64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "x86_64-unknown-linux-gnu": "linux-x64",
    "aarch64-pc-windows-msvc": "win32-arm64",
    "x86_64-pc-windows-msvc": "win32-x64",
  }[row.target];
  if (!suffix) {
    fail(`npm candidate target is unsupported: ${row.target}`);
  }
  return suffix;
}

function npmInstall(recorder, prefix, version, row, candidateStage = null) {
  mkdirSync(prefix, { recursive: true, mode: 0o755 });
  const packages = candidateStage
    ? [
        localCandidateAsset(candidateStage, `wa120-ait-native-${npmTargetSuffix(row)}-${version}.tgz`),
        localCandidateAsset(candidateStage, `wa120-ait-native-${version}.tgz`),
      ]
    : [`@wa120/ait-native@${version}`];
  const candidateArgs = candidateStage ? ["--offline"] : [];
  runSpec(
    recorder,
    npmCommandSpec(),
    [
      "install",
      "--global",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      ...candidateArgs,
      "--prefix",
      prefix,
      ...packages,
    ],
    { label: "npm exact install or upgrade" },
  );
}

function nodeContext(recorder, prefix, version, row, candidateStage = null) {
  npmInstall(recorder, prefix, version, row, candidateStage);
  const packageName = "@wa120/ait-native";
  const platformPackageName = `@wa120/ait-native-${npmTargetSuffix(row)}`;
  const packageRoot = npmPackageRoot(prefix, packageName);
  const platformPackageRoot = npmPackageRoot(prefix, platformPackageName);
  const entry = path.join(packageRoot, "bin", "ait.mjs");
  const index = path.join(packageRoot, "src", "index.js");
  requireRegularFile(entry, "npm in-process ait entrypoint");
  requireRegularFile(index, "npm direct API entrypoint");
  const node = requireCommand("node");
  return {
    ait: commandSpec(node, [entry]),
    origin: packageRoot,
    binding() {
      const code =
        "import {pathToFileURL} from 'node:url';" +
        "const m=await import(pathToFileURL(process.argv[1]).href);" +
        "const r=new m.NativeRuntime();" +
        "process.stdout.write(JSON.stringify(r.bindingInfo()));";
      const result = recorder.run(node, ["--input-type=module", "-e", code, index], {
        label: "npm direct Node-API addon",
      });
      const payload = JSON.parse(result.stdout);
      if (
        payload.runtime_authority !== "rust" ||
        payload.node_binding !== "napi" ||
        payload.process_transport_allowed !== false ||
        payload.version !== version
      ) {
        fail("npm direct binding metadata is not exact Rust Node-API authority");
      }
      const inProcess = recorder.run(
        node,
        [
          "--input-type=module",
          "-e",
          "import {pathToFileURL} from 'node:url'; const m=await import(pathToFileURL(process.argv[1]).href); const status=new m.NativeRuntime().runCli(['--version']); if(status!==0) process.exit(status);",
          index,
        ],
        { label: "npm in-process ait command" },
      );
      if (inProcess.stdout.trim() !== `ait ${version}`) {
        fail("npm in-process command returned the wrong version");
      }
    },
    uninstall() {
      runSpec(
        recorder,
        npmCommandSpec(),
        [
          "uninstall",
          "--global",
          "--ignore-scripts",
          "--prefix",
          prefix,
          packageName,
          platformPackageName,
        ],
        { label: "npm uninstall" },
      );
      if (existsSync(packageRoot)) {
        fail("npm uninstall retained the top-level package");
      }
      if (existsSync(platformPackageRoot)) {
        fail("npm uninstall retained the target platform package");
      }
    },
  };
}

function brewEnvironment() {
  return {
    HOMEBREW_NO_AUTO_UPDATE: "1",
    HOMEBREW_NO_ANALYTICS: "1",
    HOMEBREW_NO_ENV_HINTS: "1",
  };
}

function homebrewFormulaFileName(config, version) {
  // The configured formula path names the candidate channel's route. A prior
  // release installs from its own channel route: RC identities use the
  // "-rc" formula, stable identities use the bare formula.
  const configured = path.basename(config.endpoints.homebrew.formula_path, ".rb");
  const stem = configured.replace(/-rc$/, "");
  return /-rc\./.test(version) ? `${stem}-rc.rb` : `${stem}.rb`;
}

async function homebrewFormula(config, version, root, candidateStage = null) {
  const formulaName = homebrewFormulaFileName(config, version);
  const destination = path.join(root, `${version}-${formulaName}`);
  if (candidateStage) {
    writeFileSync(destination, readFileSync(localCandidateAsset(candidateStage, formulaName)), {
      mode: 0o644,
    });
  } else {
    await downloadReleaseAsset(
      config.endpoints.github.repository,
      `v${version}`,
      formulaName,
      destination,
    );
  }
  chmodSync(destination, 0o644);
  return destination;
}

function brewCommand(recorder, args, label, allowed = [0]) {
  return recorder.run(requireCommand("brew"), args, {
    label,
    env: { ...process.env, ...brewEnvironment() },
    allowed,
  });
}

function seedHomebrewCandidateCache(recorder, formulaPath, candidateStage) {
  const formula = readFileSync(formulaPath, "utf8");
  const cacheRoot = brewCommand(recorder, ["--cache"], "Homebrew cache origin").stdout.trim();
  if (!path.isAbsolute(cacheRoot)) {
    fail("Homebrew cache origin is not absolute");
  }
  const downloads = path.join(cacheRoot, "downloads");
  mkdirSync(downloads, { recursive: true, mode: 0o755 });
  const resources = [];
  const pattern = /url "([^"]+)"\s+sha256 "([0-9a-f]{64})"/g;
  for (const match of formula.matchAll(pattern)) {
    const url = match[1];
    const expected = match[2];
    const name = path.basename(new URL(url).pathname);
    const source = localCandidateAsset(candidateStage, name);
    if (sha256File(source) !== expected) {
      fail(`Homebrew formula archive digest differs from candidate asset: ${name}`);
    }
    const cached = path.join(downloads, `${sha256Bytes(url)}--${name}`);
    writeFileSync(cached, readFileSync(source), { mode: 0o644 });
    resources.push({ name, url, sha256: expected, cache_path: cached });
  }
  if (resources.length === 0) {
    fail("Homebrew candidate formula contains no cacheable archive resource");
  }
  recorder.observations.homebrew_candidate_cache = resources;
}

async function homebrewContext(
  config,
  row,
  version,
  root,
  recorder,
  upgrade = false,
  candidateStage = null,
) {
  const formulaPath = await homebrewFormula(config, version, root, candidateStage);
  const formulaFileName = homebrewFormulaFileName(config, version);
  const formula = path.basename(formulaFileName, ".rb");
  const tap = config.endpoints.homebrew.tap;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(tap ?? "")) {
    fail("Homebrew endpoint configuration has no canonical tap identity");
  }
  brewCommand(recorder, ["tap", tap], "Homebrew exact tap registration");
  const tapRoot = brewCommand(
    recorder,
    ["--repository", tap],
    "Homebrew tap repository origin",
  ).stdout.trim();
  if (!path.isAbsolute(tapRoot)) {
    fail("Homebrew tap repository origin is not absolute");
  }
  const installedFormulaPath = path.join(
    tapRoot,
    path.dirname(config.endpoints.homebrew.formula_path),
    formulaFileName,
  );
  const formulaParent = path.dirname(installedFormulaPath);
  mkdirSync(formulaParent, { recursive: true, mode: 0o755 });
  writeFileSync(installedFormulaPath, readFileSync(formulaPath), { mode: 0o644 });
  if (sha256File(installedFormulaPath) !== sha256File(formulaPath)) {
    fail("Homebrew tap formula differs from the exact downloaded formula");
  }
  if (candidateStage) {
    seedHomebrewCandidateCache(recorder, installedFormulaPath, candidateStage);
  }
  const qualifiedFormula = `${tap}/${formula}`;
  if (upgrade) {
    // A stable candidate upgrading over an RC prior crosses formula names;
    // Homebrew cannot upgrade across formulae, so the prior channel formula
    // is replaced by an exact uninstall-then-install transition. Same-name
    // upgrades keep the native upgrade path.
    const stem = formula.replace(/-rc$/, "");
    const crossFormula = formula === stem ? `${stem}-rc` : stem;
    const installedFormulae = brewCommand(
      recorder,
      ["list", "--formula"],
      "Homebrew installed formula inventory",
    ).stdout;
    const crossInstalled = installedFormulae
      .split(/\r?\n/)
      .some((line) => line.trim() === crossFormula);
    if (crossInstalled) {
      brewCommand(
        recorder,
        ["uninstall", "--formula", crossFormula],
        "Homebrew prior channel replacement uninstall",
      );
      brewCommand(
        recorder,
        ["install", "--formula", qualifiedFormula],
        "Homebrew exact channel-transition install",
      );
    } else {
      brewCommand(
        recorder,
        ["upgrade", "--formula", qualifiedFormula],
        "Homebrew exact upgrade",
      );
    }
  } else {
    brewCommand(
      recorder,
      ["install", "--formula", qualifiedFormula],
      "Homebrew exact install",
    );
  }
  const prefix = brewCommand(
    recorder,
    ["--prefix", qualifiedFormula],
    "Homebrew formula origin",
  ).stdout.trim();
  if (!path.isAbsolute(prefix)) {
    fail("Homebrew formula prefix is not absolute");
  }
  const aitPath = path.join(prefix, "bin", "ait");
  const serverPath = path.join(prefix, "bin", "ait-server");
  const includesRunner = row.components.includes("ait-runner");
  const runnerPath = path.join(prefix, "bin", "ait-runner");
  requireRegularFile(aitPath, "Homebrew ait command");
  requireRegularFile(serverPath, "Homebrew ait-server command");
  if (includesRunner) {
    requireRegularFile(runnerPath, "Homebrew ait-runner command");
  }
  return {
    ait: commandSpec(aitPath),
    server: commandSpec(serverPath),
    runner: includesRunner ? commandSpec(runnerPath) : null,
    origin: prefix,
    formula: qualifiedFormula,
    inactive() {
      const result = brewCommand(
        recorder,
        ["services", "list"],
        "Homebrew inactive service readback",
      );
      const row = result.stdout
        .split(/\r?\n/)
        .find((line) => line.trim().split(/\s+/)[0] === formula);
      if (row && /\bstarted\b/.test(row)) {
        fail("Homebrew install or upgrade started ait-server implicitly");
      }
      assertProductProcessesInactive(recorder, includesRunner);
    },
    async lifecycle() {
      brewCommand(recorder, ["services", "start", formula], "Homebrew explicit service start");
      try {
        await pollHealth("http://127.0.0.1:8088/healthz");
      } finally {
        brewCommand(
          recorder,
          ["services", "stop", formula],
          "Homebrew explicit service stop",
          [0, 1],
        );
      }
    },
    uninstall() {
      brewCommand(recorder, ["uninstall", "--formula", formula], "Homebrew uninstall");
      if (
        existsSync(aitPath) ||
        existsSync(serverPath) ||
        (includesRunner && existsSync(runnerPath))
      ) {
        fail("Homebrew uninstall retained exposed commands");
      }
    },
  };
}

function debianVersion(version) {
  return version.replace(/-rc\.([1-9]\d*)$/, "~rc.$1");
}

function runnerBundleVersion(version) {
  const match = /^(\d+)\.(\d+)\./.exec(version);
  return Boolean(
    match &&
      (Number(match[1]) > 1 ||
        (Number(match[1]) === 1 && Number(match[2]) >= 1)),
  );
}

function packageRowForVersion(row, version) {
  if (
    row.lifecycle === "product" &&
    row.components.includes("ait-runner") &&
    ["homebrew", "apt", "winget"].includes(row.channel) &&
    !runnerBundleVersion(version)
  ) {
    return { ...row, components: ["ait", "ait-server"] };
  }
  return row;
}

function aptAcquireBounds() {
  // Hosted x86_64 runners intermittently stall inside apt's own HTTP client
  // against the repository host until the job timeout; force IPv4 and bound
  // every acquire with a timeout and retries.
  return [
    "-o", "Acquire::ForceIPv4=true",
    "-o", "Acquire::http::Timeout=30",
    "-o", "Acquire::https::Timeout=30",
    "-o", "Acquire::Retries=3",
  ];
}

async function configureApt(config, root, recorder, suite = config.endpoints.apt.suite) {
  const keyUrl = `${config.endpoints.apt.base_url}/ait-native-archive-keyring.gpg`;
  const key = await fetchBytes(keyUrl, "APT archive keyring");
  const localKey = path.join(root, "ait-native-archive-keyring.gpg");
  const localSource = path.join(root, "ait-native.list");
  writeFileSync(localKey, key, { mode: 0o644 });
  writeFileSync(
    localSource,
    `deb [signed-by=/usr/share/keyrings/ait-native-archive-keyring.gpg] ${config.endpoints.apt.base_url} ${suite} ${config.endpoints.apt.component}\n`,
    { encoding: "utf8", mode: 0o644 },
  );
  recorder.run("sudo", ["install", "-m", "0644", localKey, "/usr/share/keyrings/ait-native-archive-keyring.gpg"], {
    label: "APT install archive keyring",
  });
  recorder.run("sudo", ["install", "-m", "0644", localSource, "/etc/apt/sources.list.d/ait-native.list"], {
    label: "APT install exact source route",
  });
  recorder.run("sudo", ["apt-get", ...aptAcquireBounds(), "update"], {
    label: "APT signed repository update",
  });
  for (const identity of ["ait-native", "ait-runner"]) {
    const result = recorder.run("apt-cache", ["search", "--names-only", `^${identity}$`], {
      label: `APT exact search ${identity}`,
    });
    if (!result.stdout.split(/\r?\n/).some((line) => line.startsWith(`${identity} -`))) {
      fail(`APT exact search did not discover ${identity}`);
    }
  }
}

function aptContext(row, version, recorder, upgrade = false, candidateStage = null) {
  const packageName = row.lifecycle === "standalone-runner" ? "ait-runner" : "ait-native";
  const expectedVersion = debianVersion(version);
  const architecture = row.architecture === "arm64" ? "arm64" : "amd64";
  const selector = candidateStage
    ? localCandidateAsset(
        candidateStage,
        `${packageName}_${expectedVersion}_${architecture}.deb`,
      )
    : `${packageName}=${expectedVersion}`;
  const transitionalRunnerAlias =
    packageName === "ait-runner" && candidateStage && runnerBundleVersion(version);
  const selectors = transitionalRunnerAlias
    ? [
        localCandidateAsset(
          candidateStage,
          `ait-native_${expectedVersion}_${architecture}.deb`,
        ),
        selector,
      ]
    : [selector];
  const args = [
    "apt-get",
    ...aptAcquireBounds(),
    "install",
    "--yes",
    "--no-install-recommends",
    ...selectors,
  ];
  if (upgrade && !transitionalRunnerAlias) {
    args.splice(args.indexOf("install") + 1, 0, "--only-upgrade");
  }
  recorder.run("sudo", args, { label: upgrade ? "APT exact upgrade" : "APT exact install" });
  const installed = recorder
    .run("dpkg-query", ["-W", "-f=${Version}", packageName], {
      label: "APT installed package version",
    })
    .stdout.trim();
  if (installed !== expectedVersion) {
    fail(`APT selected ${installed} instead of ${expectedVersion}`);
  }
  const context = {
    origin: "/usr/bin",
    uninstall() {
      const removals = transitionalRunnerAlias ? [packageName, "ait-native"] : [packageName];
      recorder.run("sudo", ["apt-get", "remove", "--purge", "--yes", ...removals], {
        label: "APT uninstall",
      });
      const query = recorder.run("dpkg-query", ["-W", packageName], {
        label: "APT uninstall readback",
        allowed: [1],
      });
      if (query.status !== 1) {
        fail("APT uninstall retained the package receipt");
      }
      if (transitionalRunnerAlias && existsSync("/usr/bin/ait-runner")) {
        fail("APT transition-alias uninstall retained the bundled runner");
      }
    },
  };
  if (packageName === "ait-native") {
    const includesRunner = row.components.includes("ait-runner");
    context.ait = commandSpec("/usr/bin/ait");
    context.server = commandSpec("/usr/bin/ait-server");
    context.runner = includesRunner ? commandSpec("/usr/bin/ait-runner") : null;
    context.inactive = () => {
      const active = recorder.run("systemctl", ["is-active", "ait-server.service"], {
        label: "APT server inactive readback",
        allowed: [3, 4],
      });
      if (active.status === 0) {
        fail("APT install or upgrade started ait-server implicitly");
      }
      const enabled = recorder.run("systemctl", ["is-enabled", "ait-server.service"], {
        label: "APT server disabled readback",
        allowed: [1],
      });
      if (enabled.status === 0) {
        fail("APT install or upgrade enabled ait-server implicitly");
      }
      assertProductProcessesInactive(recorder, includesRunner);
    };
    context.lifecycle = async () => {
      recorder.run("sudo", ["systemctl", "daemon-reload"], { label: "APT systemd reload" });
      recorder.run("sudo", ["systemctl", "enable", "--now", "ait-server.service"], {
        label: "APT explicit systemd start",
      });
      try {
        await pollHealth("http://127.0.0.1:8088/healthz");
      } finally {
        recorder.run("sudo", ["systemctl", "disable", "--now", "ait-server.service"], {
          label: "APT explicit systemd stop",
          allowed: [0, 1],
        });
      }
    };
  } else {
    context.runner = commandSpec("/usr/bin/ait-runner");
  }
  return context;
}

async function startCandidateAssetServer(candidateStage) {
  const port = await reservePort();
  const node = requireCommand("node");
  const code =
    "const http=require('http'),fs=require('fs'),path=require('path');" +
    "const root=path.resolve(process.argv[1]),port=Number(process.argv[2]);" +
    "http.createServer((req,res)=>{" +
    "let name;try{name=decodeURIComponent(new URL(req.url,'http://localhost').pathname.slice(1));}catch{res.writeHead(400).end();return;}" +
    "if(!name||path.basename(name)!==name){res.writeHead(404).end();return;}" +
    "const file=path.join(root,name);let stat;try{stat=fs.lstatSync(file);}catch{res.writeHead(404).end();return;}" +
    "if(!stat.isFile()||stat.isSymbolicLink()){res.writeHead(404).end();return;}" +
    "res.writeHead(200,{'Content-Length':stat.size});fs.createReadStream(file).pipe(res);" +
    `}).listen(${port},'127.0.0.1');`;
  const child = spawn(node, ["-e", code, path.join(candidateStage, "assets"), String(port)], {
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  try {
    await pollHealth(`http://127.0.0.1:${port}/SHA256SUMS`, child);
  } catch (error) {
    child.kill();
    fail(`candidate asset transport failed: ${error.message}; ${tail(stderr || stdout)}`);
  }
  return {
    baseUrl: `http://127.0.0.1:${port}`,
    child,
    stop() {
      child.kill("SIGTERM");
    },
  };
}

async function wingetManifests(config, version, root, candidateStage = null, baseUrl = null) {
  const manifestRoot = path.join(root, `winget-${version}`);
  mkdirSync(manifestRoot, { recursive: true, mode: 0o755 });
  for (const name of [
    "Weita.AitNative.yaml",
    "Weita.AitNative.locale.en-US.yaml",
    "Weita.AitNative.installer.yaml",
  ]) {
    const destination = path.join(manifestRoot, name);
    if (candidateStage) {
      writeFileSync(destination, readFileSync(localCandidateAsset(candidateStage, name)), {
        mode: 0o644,
      });
    } else {
      await downloadReleaseAsset(
        config.endpoints.github.repository,
        `v${version}`,
        name,
        destination,
      );
    }
    chmodSync(destination, 0o644);
  }
  if (!candidateStage) {
    return { validationRoot: manifestRoot, installRoot: manifestRoot, overlay: null };
  }
  if (!baseUrl) {
    fail("WinGet prepublish transport URL is unavailable");
  }
  const overlayRoot = path.join(root, `winget-${version}-transport`);
  mkdirSync(overlayRoot, { recursive: true, mode: 0o755 });
  const overlay = [];
  for (const name of [
    "Weita.AitNative.yaml",
    "Weita.AitNative.locale.en-US.yaml",
    "Weita.AitNative.installer.yaml",
  ]) {
    const original = path.join(manifestRoot, name);
    const destination = path.join(overlayRoot, name);
    let text = readFileSync(original, "utf8");
    if (name.endsWith(".installer.yaml")) {
      let replacements = 0;
      // The generated stable manifest quotes its URLs; accept an optional
      // matched double quote and preserve the original quoting byte-for-byte
      // in the rewritten transport line.
      text = text.replace(
        /^(\s*InstallerUrl:\s*)("?)(\S+?)\2\s*$/gm,
        (_line, prefix, quote, url) => {
          const assetName = path.basename(new URL(url).pathname);
          localCandidateAsset(candidateStage, assetName);
          replacements += 1;
          return `${prefix}${quote}${baseUrl}/${encodeURIComponent(assetName)}${quote}`;
        },
      );
      if (replacements !== 2) {
        fail("WinGet candidate manifest did not expose exactly two installer URLs");
      }
    }
    writeFileSync(destination, text, { encoding: "utf8", mode: 0o644 });
    overlay.push({
      name,
      original_sha256: sha256File(original),
      transport_sha256: sha256File(destination),
      installer_url_overlay: name.endsWith(".installer.yaml"),
    });
  }
  return { validationRoot: manifestRoot, installRoot: overlayRoot, overlay };
}

function wingetArgs(action, manifestRoot) {
  // Hosted runners always execute as administrator, and WinGet refuses to
  // uninstall a user-scope package from an administrator session; machine
  // scope keeps the portable registration and removal deterministic.
  return [
    action,
    "--manifest",
    manifestRoot,
    "--scope",
    "machine",
    "--disable-interactivity",
  ];
}

async function wingetContext(
  config,
  row,
  version,
  root,
  recorder,
  upgrade = false,
  candidateStage = null,
) {
  const winget = requireCommand("winget.exe");
  // Installing from a local manifest requires the LocalManifestFiles
  // setting; hosted runners never persist it, validation alone is exempt,
  // and the enablement is idempotent for an administrator session.
  recorder.run(winget, ["settings", "--enable", "LocalManifestFiles"], {
    label: "WinGet local manifest enablement",
  });
  const transport = candidateStage ? await startCandidateAssetServer(candidateStage) : null;
  let manifests;
  try {
    manifests = await wingetManifests(
      config,
      version,
      root,
      candidateStage,
      transport?.baseUrl,
    );
    const validation = recorder.run(
      winget,
      ["validate", "--manifest", manifests.validationRoot],
      {
        label: "WinGet manifest validation",
        allowed: [0, 0x8a150028],
      },
    );
    if (
      validation.status === 0x8a150028 &&
      !validation.stdout.startsWith("Manifest validation succeeded with warnings.\r\n")
    ) {
      fail("WinGet warning exit did not report exact successful validation-with-warnings");
    }
    recorder.run(
      winget,
      wingetArgs(upgrade ? "upgrade" : "install", manifests.installRoot),
      {
        label: upgrade ? "WinGet exact manifest upgrade" : "WinGet exact manifest install",
      },
    );
  } finally {
    transport?.stop();
  }
  if (manifests.overlay) {
    recorder.observations.winget_candidate_transport_overlay = {
      payload_bytes_unchanged: true,
      original_manifests_validated: true,
      files: manifests.overlay,
    };
  }
  // Source-correlated listing cannot see a local-manifest portable install
  // on hosted runners, so the list stays recorded observability while the
  // receipt authority is the Windows uninstall registration that WinGet
  // itself writes for the package identity.
  recorder.run(
    winget,
    [
      "list",
      "--id",
      config.endpoints.winget.identity,
      "--exact",
      "--disable-interactivity",
      "--accept-source-agreements",
    ],
    { label: "WinGet source-correlated list observability", recordOnly: true },
  );
  const registrationScript =
    "$identity = '" +
    config.endpoints.winget.identity +
    "';" +
    "$roots = @(" +
    "'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall'," +
    "'HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall'," +
    "'HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall'" +
    ");" +
    "$versions = foreach ($root in $roots) {" +
    "  Get-ChildItem -Path $root -ErrorAction SilentlyContinue |" +
    "    Where-Object { $_.PSChildName -like ($identity + '*') } |" +
    "    ForEach-Object { (Get-ItemProperty -Path $_.PSPath).DisplayVersion }" +
    "};" +
    "if (-not $versions) { exit 3 };" +
    "$versions -join \"`n\"";
  const registration = recorder.run(
    "powershell.exe",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", registrationScript],
    { label: "WinGet package registration readback" },
  );
  if (!registration.stdout.includes(version)) {
    fail("WinGet package registration does not report the exact installed version");
  }
  // The portable install modifies the persisted PATH, which this already
  // running process and its children never observe; resolve the aliases
  // through WinGet's Links directories first.
  const wingetAliasPath = (name) => {
    const candidates = [];
    if (process.env.ProgramFiles) {
      candidates.push(path.join(process.env.ProgramFiles, "WinGet", "Links", name));
    }
    if (process.env.LOCALAPPDATA) {
      candidates.push(path.join(process.env.LOCALAPPDATA, "Microsoft", "WinGet", "Links", name));
    }
    for (const candidate of candidates) {
      if (existsSync(candidate)) {
        return candidate;
      }
    }
    return null;
  };
  const aitPath = wingetAliasPath("ait.exe") ?? requireCommand("ait.exe");
  const serverPath = wingetAliasPath("ait-server.exe") ?? requireCommand("ait-server.exe");
  const includesRunner = row.components.includes("ait-runner");
  const runnerPath = includesRunner
    ? wingetAliasPath("ait-runner.exe") ?? requireCommand("ait-runner.exe")
    : null;
  return {
    ait: commandSpec(aitPath),
    server: commandSpec(serverPath),
    runner: runnerPath ? commandSpec(runnerPath) : null,
    origin: path.dirname(realpathSync(aitPath)),
    inactive() {
      assertProductProcessesInactive(recorder, includesRunner);
    },
    async lifecycle() {
      const script =
        `$link=Get-Item '${serverPath}';` +
        "$serverPath=@($link.Target)[0];" +
        "if(-not $serverPath){$serverPath=$link.FullName}" +
        "elseif(-not [IO.Path]::IsPathRooted($serverPath)){$serverPath=Join-Path $link.DirectoryName $serverPath};" +
        "$ctl=Join-Path (Split-Path -Parent $serverPath) 'ait-server-control.ps1';" +
        "& $ctl start; if($LASTEXITCODE){exit $LASTEXITCODE}";
      recorder.run("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script], {
        label: "WinGet explicit user-session start",
      });
      try {
        await pollHealth("http://127.0.0.1:8088/healthz");
      } finally {
        recorder.run(
          "powershell.exe",
          [
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script.replace("& $ctl start", "& $ctl stop"),
          ],
          { label: "WinGet explicit user-session stop", allowed: [0, 1] },
        );
      }
    },
    uninstall() {
      // Source-correlated identity lookup cannot see the local-manifest
      // portable install on hosted runners; the local manifest names the
      // exact package for removal instead.
      recorder.run(
        winget,
        [
          "uninstall",
          "--manifest",
          manifests.validationRoot,
          "--disable-interactivity",
          "--accept-source-agreements",
        ],
        { label: "WinGet uninstall" },
      );
      if (existsSync(aitPath)) {
        fail("WinGet uninstall retained the ait portable alias");
      }
      if (runnerPath && existsSync(runnerPath)) {
        fail("WinGet uninstall retained the ait-runner portable alias");
      }
      const readback = spawnSync("where.exe", ["ait.exe"], { encoding: "utf8" });
      if (readback.status === 0) {
        fail("WinGet uninstall retained the ait portable alias on PATH");
      }
    },
  };
}

function dockerPlatform(row) {
  return row.architecture === "arm64" ? "linux/arm64" : "linux/amd64";
}

function candidateOciDigest(status, row) {
  const key = row.lifecycle === "standalone-server" ? "server" : "runner";
  const digest = status.platforms?.oci?.[key];
  if (!/^sha256:[0-9a-f]{64}$/.test(digest ?? "")) {
    fail(`operator status lacks the exact ${key} OCI digest`);
  }
  return digest;
}

function candidateOciArchive(status, row, candidateStage) {
  const component = row.lifecycle === "standalone-server" ? "ait-server" : "ait-runner";
  const architecture = row.architecture === "arm64" ? "arm64" : "amd64";
  const descriptor = status.candidate?.oci?.[component]?.[architecture];
  if (
    !descriptor ||
    !/^[A-Za-z0-9][A-Za-z0-9._-]*\.docker\.tar$/.test(descriptor.archive ?? "") ||
    !/^[0-9a-f]{64}$/.test(descriptor.sha256 ?? "") ||
    typeof descriptor.reference !== "string" ||
    !descriptor.reference
  ) {
    fail(`prepublish status lacks exact ${component}/${architecture} OCI archive authority`);
  }
  const archive = path.join(candidateStage, "oci-archives", descriptor.archive);
  requireRegularFile(archive, "prepublish OCI archive");
  if (sha256File(archive) !== descriptor.sha256) {
    fail(`prepublish ${component}/${architecture} OCI archive digest drifted`);
  }
  return { ...descriptor, archive, component, architecture };
}

function dockerImageDigest(recorder, reference) {
  const result = recorder.run(
    "docker",
    ["image", "inspect", "--format", "{{json .RepoDigests}}", reference],
    { label: "OCI image digest readback" },
  );
  const digests = JSON.parse(result.stdout);
  return Array.isArray(digests) ? digests : [];
}

function ociContext(
  config,
  status,
  row,
  version,
  recorder,
  candidate,
  candidateStage = null,
) {
  requireCommand("docker");
  const platform = dockerPlatform(row);
  let reference;
  if (candidateStage) {
    const descriptor = candidateOciArchive(status, row, candidateStage);
    recorder.run("docker", ["load", "--input", descriptor.archive], {
      label: "OCI frozen candidate archive load",
    });
    reference = descriptor.reference;
    const imageId = recorder
      .run("docker", ["image", "inspect", "--format", "{{.Id}}", reference], {
        label: "OCI frozen candidate image identity",
      })
      .stdout.trim();
    if (descriptor.image_id && imageId !== descriptor.image_id) {
      fail("OCI loaded candidate image identity differs from the staged archive");
    }
    recorder.observations.oci_candidate_archive = {
      component: descriptor.component,
      architecture: descriptor.architecture,
      sha256: descriptor.sha256,
      image_id: imageId,
      reference,
    };
  } else {
    reference = candidate
      ? `${row.identity}@${candidateOciDigest(status, row)}`
      : `${row.identity}:${version}`;
    recorder.run("docker", ["pull", "--platform", platform, reference], {
      label: candidate ? "OCI candidate digest pull" : "OCI exact prior version pull",
    });
    const digests = dockerImageDigest(recorder, reference);
    if (
      candidate &&
      !digests.some((value) => value.endsWith(`@${candidateOciDigest(status, row)}`))
    ) {
      fail("OCI candidate pull did not resolve to the dossier digest");
    }
  }
  const component = row.lifecycle === "standalone-server" ? "ait-server" : "ait-runner";
  return {
    reference,
    origin: reference,
    component,
    exactVersion() {
      const result = recorder.run(
        "docker",
        ["run", "--rm", "--platform", platform, reference, "--version"],
        { label: "OCI command version" },
      );
      if (result.stdout.trim() !== `${component} ${version}`) {
        fail("OCI command version differs from the selected immutable image");
      }
    },
    inactive() {
      const result = recorder.run(
        "docker",
        ["ps", "--quiet", "--filter", `ancestor=${reference}`],
        { label: "OCI inactive-by-default readback" },
      );
      if (result.stdout.trim()) {
        fail("OCI pull started a container implicitly");
      }
    },
    async lifecycle(root) {
      if (row.lifecycle === "standalone-runner") {
        recorder.run(
          "docker",
          ["run", "--rm", "--platform", platform, reference, "--help"],
          { label: "OCI explicit runner lifecycle" },
        );
        return;
      }
      const name = `ait-clean-host-${process.pid}-${Date.now()}`;
      const volume = `${name}-data`;
      recorder.run("docker", ["volume", "create", volume], { label: "OCI server data volume" });
      recorder.run(
        "docker",
        [
          "run",
          "--detach",
          "--name",
          name,
          "--platform",
          platform,
          "--publish",
          "127.0.0.1::8088",
          "--volume",
          `${volume}:/var/lib/ait`,
          reference,
        ],
        { label: "OCI explicit server start" },
      );
      let lifecycleError = null;
      try {
        const portText = recorder
          .run("docker", ["port", name, "8088/tcp"], { label: "OCI server port readback" })
          .stdout.trim();
        const match = portText.match(/:(\d+)$/);
        if (!match) {
          fail("OCI server published port is invalid");
        }
        await pollHealth(`http://127.0.0.1:${match[1]}/healthz`);
      } catch (error) {
        lifecycleError = error;
      } finally {
        recorder.run("docker", ["logs", name], {
          label: "OCI server logs before removal",
          recordOnly: true,
        });
        recorder.run("docker", ["inspect", name], {
          label: "OCI server inspect before removal",
          recordOnly: true,
        });
        recorder.run("docker", ["rm", "--force", name], {
          label: "OCI explicit server stop",
          allowed: [0, 1],
        });
      }
      if (lifecycleError) {
        throw lifecycleError;
      }
      const volumeReadback = recorder.run("docker", ["volume", "inspect", volume], {
        label: "OCI retained user-data volume",
      });
      if (!volumeReadback.stdout.includes(volume)) {
        fail("OCI server lifecycle did not retain its data volume");
      }
      recorder.observations.oci_retained_volume = volume;
      recorder.observations.oci_phase_root = root;
    },
    uninstall() {
      recorder.run("docker", ["image", "rm", "--force", reference], {
        label: "OCI image uninstall",
      });
      recorder.run("docker", ["image", "inspect", reference], {
        label: "OCI image uninstall readback",
        allowed: [1],
      });
    },
  };
}

function probePackageManager(row, recorder) {
  if (row.channel === "pypi") {
    recorder.observations.package_manager_commands = { python: pythonCommand() };
    return;
  }
  const commands = {
    github: ["node"],
    npm: [process.platform === "win32" ? "npm.cmd" : "npm", "node"],
    homebrew: ["brew"],
    apt: ["sudo", "apt-get", "apt-cache", "dpkg-query", "systemctl"],
    winget: ["winget.exe", "powershell.exe"],
    oci: ["docker"],
  }[row.channel];
  if (!commands) {
    fail(`unsupported clean-host channel: ${row.channel}`);
  }
  const resolved = Object.fromEntries(commands.map((command) => [command, requireCommand(command)]));
  if (row.channel === "apt") {
    recorder.run("sudo", ["-n", "true"], { label: "APT passwordless privilege probe" });
  }
  recorder.observations.package_manager_commands = resolved;
}

function assertInstalledOrigin(context, row) {
  if (row.channel === "oci") {
    if (context.origin !== context.reference) {
      fail("OCI installed origin differs from the immutable reference");
    }
    return;
  }
  const specifications = [context.ait, context.server, context.runner].filter(Boolean);
  if (specifications.length === 0 || !path.isAbsolute(context.origin)) {
    fail("installed channel exposes no absolute command origin");
  }
  for (const specification of specifications) {
    const ownedPath = specification.argsPrefix.find((value) => path.isAbsolute(value)) ?? specification.command;
    const realOwned = realpathSync(ownedPath);
    const realOrigin = realpathSync(context.origin);
    if (realOwned !== realOrigin && !realOwned.startsWith(`${realOrigin}${path.sep}`)) {
      fail(`installed command escaped its channel origin: ${realOwned}`);
    }
  }
}

function exactContextVersion(context, row, version, recorder) {
  if (row.channel === "oci") {
    context.exactVersion();
    return;
  }
  if (context.ait) {
    exactVersion(recorder, context.ait, "ait", version);
  }
  if (context.server) {
    exactVersion(recorder, context.server, "ait-server", version);
  }
  if (context.runner) {
    exactVersion(recorder, context.runner, "ait-runner", version);
  }
}

function exerciseBinding(context, row) {
  if (row.binding && typeof context.binding !== "function") {
    fail(`${row.binding} row exposes no direct binding check`);
  }
  if (context.binding) {
    context.binding();
  }
}

async function exerciseLifecycle(context, row, root, recorder) {
  if (row.service === null) {
    return null;
  }
  if (row.channel === "oci") {
    context.inactive();
    await context.lifecycle(root);
    return recorder.observations.oci_retained_volume ?? null;
  }
  if (typeof context.inactive === "function") {
    context.inactive();
  } else {
    assertNoServerProcess(recorder);
  }
  if (typeof context.lifecycle === "function") {
    await context.lifecycle(root);
    if (typeof context.inactive === "function") {
      context.inactive();
    }
  } else if (context.server) {
    return foregroundServer(recorder, context.server, root);
  } else {
    fail("declared service row exposes no explicit lifecycle");
  }
  return null;
}

async function installChannel({
  config,
  status,
  row,
  version,
  pythonVersion,
  root,
  recorder,
  upgrade,
  candidate,
}) {
  const candidateStage = candidate ? candidateRoot(status) : null;
  switch (row.channel) {
    case "github":
      return githubContext(config, row, version, root, recorder, candidateStage);
    case "pypi":
      return pythonContext(
        recorder,
        path.join(root, "pypi-venv"),
        pythonVersion,
        upgrade,
        candidateStage,
      );
    case "npm":
      return nodeContext(
        recorder,
        path.join(root, "npm-prefix"),
        version,
        row,
        candidateStage,
      );
    case "homebrew":
      return homebrewContext(config, row, version, root, recorder, upgrade, candidateStage);
    case "apt":
      return aptContext(row, version, recorder, upgrade, candidateStage);
    case "winget":
      return wingetContext(config, row, version, root, recorder, upgrade, candidateStage);
    case "oci":
      return ociContext(
        config,
        status,
        row,
        version,
        recorder,
        candidate,
        candidateStage,
      );
    default:
      fail(`unsupported clean-host channel: ${row.channel}`);
  }
}

function mark(checks, name) {
  if (!Object.hasOwn(checks, name)) {
    fail(`phase attempted undeclared evidence check: ${name}`);
  }
  checks[name] = true;
}

function markBindingChecks(checks, row) {
  if (row.binding === "python") {
    mark(checks, "python_direct_binding");
  } else if (row.binding === "node") {
    mark(checks, "node_direct_addon");
    mark(checks, "node_in_process_command");
  }
}

async function executeInstall({ config, status, row, checks, recorder, root }) {
  verifyCandidateTag(config, recorder);
  probePackageManager(row, recorder);
  mark(checks, "runner_target");
  mark(checks, "package_manager");
  if (row.channel === "apt" && !candidateRoot(status)) {
    await configureApt(config, root, recorder);
  }
  const context = await installChannel({
    config,
    status,
    row,
    version: config.release.version,
    pythonVersion: config.release.python_version,
    root,
    recorder,
    upgrade: false,
    candidate: true,
  });
  mark(checks, "candidate_install");
  assertInstalledOrigin(context, row);
  mark(checks, "installed_origin");
  exactContextVersion(context, row, config.release.version, recorder);
  mark(checks, "command_version");
  if (row.channel === "oci") {
    mark(checks, "immutable_image_digest");
  }
  exerciseBinding(context, row);
  markBindingChecks(checks, row);
  const lifecycleData = await exerciseLifecycle(context, row, root, recorder);
  if (row.service !== null) {
    mark(checks, "component_inactive_default");
    mark(checks, "explicit_component_lifecycle");
  }
  let repository = null;
  if (row.lifecycle === "product") {
    if (!context.ait) {
      fail("product row did not expose the installed ait command");
    }
    repository = firstLand(
      recorder,
      context.ait,
      path.join(root, "candidate-repository"),
      `clean-host ${row.id} ${config.release.version}\n`,
    );
    mark(checks, "first_land");
    mark(checks, "generated_workflow_current");
  }
  if (typeof context.uninstall !== "function") {
    fail("install phase exposes no channel-native uninstall");
  }
  context.uninstall();
  mark(checks, "uninstall");
  if (row.lifecycle === "product") {
    requireRegularFile(
      path.join(repository.root, ".ait", "config.json"),
      "retained repository authority",
    );
    requireRegularFile(path.join(repository.root, "first-land.txt"), "retained landed file");
    mark(checks, "user_data_retained");
  }
  recorder.observations.lifecycle_data = lifecycleData;
}

async function executeUpgrade({ options, config, status, row, checks, recorder, root }) {
  verifyCandidateTag(config, recorder);
  probePackageManager(row, recorder);
  mark(checks, "runner_target");
  mark(checks, "package_manager");
  const priorRow = packageRowForVersion(row, options["prior-version"]);
  if (row.channel === "apt") {
    // The prior release installs from its own channel suite. An RC prior
    // lives on the testing suite even when the candidate publishes stable;
    // the candidate itself upgrades from the frozen local package, so the
    // configured route only ever serves the prior baseline.
    const priorSuite = /-rc\./.test(options["prior-version"]) ? "testing" : "stable";
    await configureApt(config, root, recorder, priorSuite);
  }
  const priorRoot = row.channel === "github" ? path.join(root, "prior-github") : root;
  mkdirSync(priorRoot, { recursive: true, mode: 0o755 });
  const prior = await installChannel({
    config,
    status,
    row: priorRow,
    version: options["prior-version"],
    pythonVersion: options["prior-python-version"],
    root: priorRoot,
    recorder,
    upgrade: false,
    candidate: false,
  });
  exactContextVersion(prior, priorRow, options["prior-version"], recorder);
  exerciseBinding(prior, priorRow);
  mark(checks, "prior_exact_install");
  let priorState = null;
  let repositoryRoot = null;
  if (row.lifecycle === "product") {
    if (!prior.ait) {
      fail("prior product did not expose ait for state creation");
    }
    repositoryRoot = path.join(root, "upgrade-repository");
    priorState = initializePriorState(
      recorder,
      prior.ait,
      repositoryRoot,
      options["prior-version"],
    );
    recorder.observations.prior_repository_baseline = priorState;
    mark(checks, "prior_baseline");
  }
  const candidateRoot = row.channel === "github" ? path.join(root, "candidate-github") : root;
  mkdirSync(candidateRoot, { recursive: true, mode: 0o755 });
  const candidate = await installChannel({
    config,
    status,
    row,
    version: config.release.version,
    pythonVersion: config.release.python_version,
    root: candidateRoot,
    recorder,
    upgrade: !["github", "oci"].includes(row.channel),
    candidate: true,
  });
  mark(checks, "candidate_upgrade");
  assertInstalledOrigin(candidate, row);
  mark(checks, "installed_origin");
  exactContextVersion(candidate, row, config.release.version, recorder);
  mark(checks, "command_version");
  if (row.channel === "oci") {
    mark(checks, "immutable_image_digest");
  }
  exerciseBinding(candidate, row);
  markBindingChecks(checks, row);
  await exerciseLifecycle(candidate, row, root, recorder);
  if (row.service !== null) {
    mark(checks, "component_inactive_default");
    mark(checks, "explicit_component_lifecycle");
  }
  if (row.lifecycle === "product") {
    firstLand(
      recorder,
      candidate.ait,
      repositoryRoot,
      `clean-host upgrade ${row.id} ${config.release.version}\n`,
      priorState,
    );
    mark(checks, "candidate_land");
    mark(checks, "generated_workflow_current");
  }
}

async function runPhase(options) {
  requireNewOutput(options.output);
  const matrix = readJson(options.matrix, "clean-host matrix");
  const config = readJson(options.config, "endpoint configuration");
  const status = readJson(options.status, "operator status");
  const row = validateInputs(options, matrix, config, status);
  nativePlatform(row);
  const runner = githubRunner(row);
  const checks = Object.fromEntries(row.required_checks[options.phase].map((name) => [name, false]));
  const recorder = new Recorder();
  const temporaryParent = realpathSync(process.env.RUNNER_TEMP ?? os.tmpdir());
  const root = mkdtempSync(path.join(temporaryParent, `ait-clean-host-${options.phase}-`));
  let phaseError = null;
  try {
    const parameters = { options, config, status, row, checks, recorder, root };
    if (options.phase === "install") {
      await executeInstall(parameters);
    } else {
      await executeUpgrade(parameters);
    }
    if (!Object.values(checks).every((value) => value === true)) {
      fail("phase completed without every declared check");
    }
  } catch (error) {
    phaseError = {
      message: error.message,
      category: error.exitCode === 69 ? "environment_capability" : "lifecycle_failure",
    };
  }
  const evidence = {
    contract: PHASE_CONTRACT,
    status: phaseError ? "fail" : "pass",
    phase: options.phase,
    release: releaseBinding(config, status, options),
    row,
    runner,
    prior: {
      version: options["prior-version"],
      python_version: options["prior-python-version"],
      selector: "exact_immutable_version",
    },
    checks,
    observations: recorder.observations,
    commands: recorder.commands,
    error: phaseError,
  };
  writeFileSync(options.output, encoded(evidence), { encoding: "utf8", mode: 0o644, flag: "wx" });
  process.stdout.write(`${options.output}\n`);
  if (phaseError) {
    process.stderr.write(`clean-host ${row.id} ${options.phase} failed: ${phaseError.message}\n`);
    process.exitCode = 1;
  }
}

async function main() {
  if (process.argv[2] !== "run") {
    usage(process.argv[2] ? `unsupported clean-host phase command: ${process.argv[2]}` : null);
  }
  const options = parseOptions(process.argv.slice(3));
  await runPhase(options);
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitCode ?? 70);
});
