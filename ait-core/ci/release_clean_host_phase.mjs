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
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const PHASE_CONTRACT = "ait.release.clean-host.phase/v1";
const MATRIX_CONTRACT = "ait.release.clean-host.matrix/v1";
const MAX_CAPTURE = 16 * 1024 * 1024;
const COMMAND_TIMEOUT_MS = 10 * 60 * 1000;

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
    if (!allowed.includes(status)) {
      fail(
        `${options.label ?? command} failed with ${status}: ${tail(stderr || stdout, 1200)}`,
      );
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
  if (
    matrix.contract !== MATRIX_CONTRACT ||
    matrix.matrix_revision !== "distribution-target-32-2026-08-17.1" ||
    matrix.row_count !== 32 ||
    !Array.isArray(matrix.rows)
  ) {
    fail("phase matrix is not the exact 32-row authority");
  }
  const row = matrix.rows.find((candidate) => candidate.id === options["row-id"]);
  if (!row) {
    fail("phase row is absent from the clean-host matrix");
  }
  if (
    config.contract !== "ait.release.family.endpoints/v1" ||
    status.contract !== "ait.release.operator.status/v1" ||
    status.status !== "published_pending_clean_host_smoke" ||
    status.release?.id !== config.release?.id ||
    status.release?.version !== config.release?.version ||
    status.release?.tag !== config.release?.tag ||
    matrix.release?.version !== config.release?.version ||
    matrix.release?.tag !== config.release?.tag
  ) {
    fail("phase config, status, and matrix do not bind one pending release");
  }
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

function releaseBinding(config, options) {
  return {
    id: config.release.id,
    version: config.release.version,
    python_version: config.release.python_version,
    channel: config.release.channel,
    tag: config.release.tag,
    source_commit: config.release.source_commit,
    endpoint_config_sha256: sha256File(options.config),
    operator_status_sha256: sha256File(options.status),
  };
}

async function fetchBytes(url, label) {
  const response = await fetch(url, {
    redirect: "follow",
    headers: { "User-Agent": "ait-native-clean-host/v1" },
  });
  if (!response.ok) {
    fail(`${label} returned HTTP ${response.status}: ${url}`, 69);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function fetchJson(url, label) {
  const bytes = await fetchBytes(url, label);
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    fail(`${label} is not JSON: ${url}`, 69);
  }
}

async function releaseRecord(repository, tag) {
  return fetchJson(
    `https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(tag)}`,
    `GitHub Release ${tag}`,
  );
}

async function verifyCandidateTag(config) {
  const repository = config.endpoints.github.repository;
  const tag = config.release.tag;
  const ref = await fetchJson(
    `https://api.github.com/repos/${repository}/git/ref/tags/${encodeURIComponent(tag)}`,
    "candidate Git tag",
  );
  let commit = ref.object?.sha;
  if (ref.object?.type === "tag") {
    const tagObject = await fetchJson(
      `https://api.github.com/repos/${repository}/git/tags/${ref.object.sha}`,
      "candidate annotated Git tag",
    );
    if (tagObject.object?.type !== "commit") {
      fail("candidate annotated tag does not resolve directly to a commit");
    }
    commit = tagObject.object.sha;
  }
  if (commit !== config.release.source_commit) {
    fail("candidate public tag does not resolve to the configured source commit");
  }
}

async function downloadReleaseAsset(record, name, destination) {
  const matches = (record.assets ?? []).filter((asset) => asset.name === name);
  if (matches.length !== 1 || !/^sha256:[0-9a-f]{64}$/.test(matches[0].digest ?? "")) {
    fail(`GitHub Release asset is absent, duplicate, or lacks digest: ${name}`);
  }
  const bytes = await fetchBytes(matches[0].browser_download_url, `release asset ${name}`);
  if (`sha256:${sha256Bytes(bytes)}` !== matches[0].digest) {
    fail(`GitHub Release asset digest differs after download: ${name}`);
  }
  writeFileSync(destination, bytes, { mode: 0o755 });
  if (process.platform !== "win32") {
    chmodSync(destination, 0o755);
  }
  return { name, digest: matches[0].digest, size_bytes: bytes.length };
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
  for (const required of ["ait task start --from", "ait snapshot create", "ait task land"]) {
    if (!agents.includes(required)) {
      fail(`generated AGENTS.md is missing current workflow spelling: ${required}`);
    }
  }
}

function initializePriorState(recorder, aitSpec, root) {
  mkdirSync(root, { recursive: false, mode: 0o755 });
  jsonSpec(recorder, aitSpec, ["init", "--json"], {
    cwd: root,
    label: "prior ait init",
  });
  const configPath = path.join(root, ".ait", "config.json");
  requireRegularFile(configPath, "prior repository config");
  return { config_sha256: sha256File(configPath) };
}

function firstLand(recorder, aitSpec, root, expectedText, priorState = null) {
  if (!existsSync(root)) {
    mkdirSync(root, { recursive: false, mode: 0o755 });
    jsonSpec(recorder, aitSpec, ["init", "--json"], {
      cwd: root,
      label: "candidate ait init",
    });
  }
  const configPath = path.join(root, ".ait", "config.json");
  requireRegularFile(configPath, "candidate repository config");
  if (priorState && sha256File(configPath) !== priorState.config_sha256) {
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
  const worktree = started.worktree?.open_path ?? started.worktree?.path;
  if (!/^LT-[0-9]{4,}$/.test(taskId ?? "") || !path.isAbsolute(worktree ?? "")) {
    fail("candidate task start returned no exact Task or worktree");
  }
  const landedFile = path.join(worktree, "first-land.txt");
  writeFileSync(landedFile, expectedText, { encoding: "utf8", mode: 0o644 });
  const snapshot = jsonSpec(
    recorder,
    aitSpec,
    ["snapshot", "create", "--message", "Clean-host first Snapshot", "--json"],
    { cwd: worktree, label: "candidate Snapshot create" },
  );
  if (!/^SNP-[0-9A-F]{12}$/.test(snapshot.snapshot_id ?? "")) {
    fail("candidate Snapshot identity is invalid");
  }
  if (snapshot.parent_snapshot_id !== null) {
    fail("clean-host first land did not author the first Snapshot on an empty default Line");
  }
  const landed = jsonSpec(
    recorder,
    aitSpec,
    ["task", "land", taskId, "--local", "--json"],
    { cwd: worktree, label: "candidate task land" },
  );
  if (
    landed.task_status !== "completed" ||
    landed.closeout_status !== "complete" ||
    landed.plan_checklist_closeout?.status !== "synced"
  ) {
    fail("candidate first land did not complete exact Task and Plan closeout");
  }
  if (readFileSync(path.join(root, "first-land.txt"), "utf8") !== expectedText) {
    fail("candidate first land reported success without materializing the file");
  }
  if (!readFileSync(sprintPath, "utf8").includes("- [x] Materialize the exact clean-host file.")) {
    fail("candidate first land did not close the exact sprint checklist item");
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
    "run",
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
  try {
    await pollHealth(`http://127.0.0.1:${port}/healthz`, child);
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
  }
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

async function githubContext(config, row, version, root, recorder) {
  const repository = config.endpoints.github.repository;
  const tag = `v${version}`;
  const release = await releaseRecord(repository, tag);
  const bin = path.join(root, "github-bin");
  mkdirSync(bin, { recursive: true, mode: 0o755 });
  const assets = [];
  for (const component of ["ait", "ait-agent", "ait-agent-worker", "ait-server", "ait-runner"]) {
    const suffix = row.executable_suffix;
    const name = `${component}-${version}-${row.target}${suffix}`;
    const destination = path.join(bin, `${component}${suffix}`);
    assets.push(await downloadReleaseAsset(release, name, destination));
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
  requireRegularFile(paths.python, "venv Python");
  return paths;
}

function pipInstall(recorder, python, pythonVersion, upgrade = false) {
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
  args.push(`ait-native==${pythonVersion}`);
  recorder.run(python, args, { label: upgrade ? "PyPI exact upgrade" : "PyPI exact install" });
}

function pythonContext(recorder, root, pythonVersion, upgrade = false) {
  const paths = existsSync(root) ? venvPaths(root) : createVenv(recorder, root);
  pipInstall(recorder, paths.python, pythonVersion, upgrade);
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

function npmPackageRoot(prefix) {
  return process.platform === "win32"
    ? path.join(prefix, "node_modules", "@wa120", "ait-native")
    : path.join(prefix, "lib", "node_modules", "@wa120", "ait-native");
}

function npmInstall(recorder, prefix, version) {
  mkdirSync(prefix, { recursive: true, mode: 0o755 });
  recorder.run(
    requireCommand(process.platform === "win32" ? "npm.cmd" : "npm"),
    [
      "install",
      "--global",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      "--prefix",
      prefix,
      `@wa120/ait-native@${version}`,
    ],
    { label: "npm exact install or upgrade" },
  );
}

function nodeContext(recorder, prefix, version) {
  npmInstall(recorder, prefix, version);
  const packageRoot = npmPackageRoot(prefix);
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
      recorder.run(
        requireCommand(process.platform === "win32" ? "npm.cmd" : "npm"),
        [
          "uninstall",
          "--global",
          "--ignore-scripts",
          "--prefix",
          prefix,
          "@wa120/ait-native",
        ],
        { label: "npm uninstall" },
      );
      if (existsSync(packageRoot)) {
        fail("npm uninstall retained the top-level package");
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

async function homebrewFormula(config, version, root) {
  const release = await releaseRecord(config.endpoints.github.repository, `v${version}`);
  const formulaName = path.basename(config.endpoints.homebrew.formula_path);
  const destination = path.join(root, `${version}-${formulaName}`);
  await downloadReleaseAsset(release, formulaName, destination);
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

async function homebrewContext(config, version, root, recorder, upgrade = false) {
  const formulaPath = await homebrewFormula(config, version, root);
  const formula = path.basename(config.endpoints.homebrew.formula_path, ".rb");
  if (upgrade) {
    brewCommand(recorder, ["upgrade", "--formula", formulaPath], "Homebrew exact upgrade");
  } else {
    brewCommand(recorder, ["install", "--formula", formulaPath], "Homebrew exact install");
  }
  const prefix = brewCommand(recorder, ["--prefix", formula], "Homebrew formula origin").stdout.trim();
  if (!path.isAbsolute(prefix)) {
    fail("Homebrew formula prefix is not absolute");
  }
  const aitPath = path.join(prefix, "bin", "ait");
  const serverPath = path.join(prefix, "bin", "ait-server");
  requireRegularFile(aitPath, "Homebrew ait command");
  requireRegularFile(serverPath, "Homebrew ait-server command");
  return {
    ait: commandSpec(aitPath),
    server: commandSpec(serverPath),
    origin: prefix,
    formula,
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
      if (existsSync(aitPath) || existsSync(serverPath)) {
        fail("Homebrew uninstall retained exposed commands");
      }
    },
  };
}

function debianVersion(version) {
  return version.replace(/-rc\.([1-9]\d*)$/, "~rc.$1");
}

async function configureApt(config, root, recorder) {
  const keyUrl = `${config.endpoints.apt.base_url}/ait-native-archive-keyring.gpg`;
  const key = await fetchBytes(keyUrl, "APT archive keyring");
  const localKey = path.join(root, "ait-native-archive-keyring.gpg");
  const localSource = path.join(root, "ait-native.list");
  writeFileSync(localKey, key, { mode: 0o644 });
  writeFileSync(
    localSource,
    `deb [signed-by=/usr/share/keyrings/ait-native-archive-keyring.gpg] ${config.endpoints.apt.base_url} ${config.endpoints.apt.suite} ${config.endpoints.apt.component}\n`,
    { encoding: "utf8", mode: 0o644 },
  );
  recorder.run("sudo", ["install", "-m", "0644", localKey, "/usr/share/keyrings/ait-native-archive-keyring.gpg"], {
    label: "APT install archive keyring",
  });
  recorder.run("sudo", ["install", "-m", "0644", localSource, "/etc/apt/sources.list.d/ait-native.list"], {
    label: "APT install exact source route",
  });
  recorder.run("sudo", ["apt-get", "update"], { label: "APT signed repository update" });
  for (const identity of ["ait-native", "ait-runner"]) {
    const result = recorder.run("apt-cache", ["search", "--names-only", `^${identity}$`], {
      label: `APT exact search ${identity}`,
    });
    if (!result.stdout.split(/\r?\n/).some((line) => line.startsWith(`${identity} -`))) {
      fail(`APT exact search did not discover ${identity}`);
    }
  }
}

function aptContext(row, version, recorder, upgrade = false) {
  const packageName = row.lifecycle === "standalone-runner" ? "ait-runner" : "ait-native";
  const expectedVersion = debianVersion(version);
  const args = [
    "apt-get",
    "install",
    "--yes",
    "--no-install-recommends",
    `${packageName}=${expectedVersion}`,
  ];
  if (upgrade) {
    args.splice(2, 0, "--only-upgrade");
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
      recorder.run("sudo", ["apt-get", "remove", "--purge", "--yes", packageName], {
        label: "APT uninstall",
      });
      const query = recorder.run("dpkg-query", ["-W", packageName], {
        label: "APT uninstall readback",
        allowed: [1],
      });
      if (query.status !== 1) {
        fail("APT uninstall retained the package receipt");
      }
    },
  };
  if (packageName === "ait-native") {
    context.ait = commandSpec("/usr/bin/ait");
    context.server = commandSpec("/usr/bin/ait-server");
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

async function wingetManifests(config, version, root) {
  const release = await releaseRecord(config.endpoints.github.repository, `v${version}`);
  const manifestRoot = path.join(root, `winget-${version}`);
  mkdirSync(manifestRoot, { recursive: true, mode: 0o755 });
  for (const name of [
    "Weita.AitNative.yaml",
    "Weita.AitNative.locale.en-US.yaml",
    "Weita.AitNative.installer.yaml",
  ]) {
    const destination = path.join(manifestRoot, name);
    await downloadReleaseAsset(release, name, destination);
    chmodSync(destination, 0o644);
  }
  return manifestRoot;
}

function wingetArgs(action, manifestRoot) {
  return [
    action,
    "--manifest",
    manifestRoot,
    "--accept-package-agreements",
    "--accept-source-agreements",
    "--disable-interactivity",
  ];
}

async function wingetContext(config, version, root, recorder, upgrade = false) {
  const winget = requireCommand("winget.exe");
  const manifestRoot = await wingetManifests(config, version, root);
  recorder.run(winget, ["validate", "--manifest", manifestRoot], {
    label: "WinGet manifest validation",
  });
  recorder.run(winget, wingetArgs(upgrade ? "upgrade" : "install", manifestRoot), {
    label: upgrade ? "WinGet exact manifest upgrade" : "WinGet exact manifest install",
  });
  const list = recorder.run(
    winget,
    ["list", "--id", config.endpoints.winget.identity, "--exact", "--disable-interactivity"],
    { label: "WinGet package receipt readback" },
  );
  if (!list.stdout.includes(version)) {
    fail("WinGet list does not report the exact installed version");
  }
  const aitPath = requireCommand("ait.exe");
  const serverPath = requireCommand("ait-server.exe");
  return {
    ait: commandSpec(aitPath),
    server: commandSpec(serverPath),
    origin: path.dirname(realpathSync(aitPath)),
    inactive() {
      assertNoServerProcess(recorder);
    },
    async lifecycle() {
      const script =
        "$link=Get-Item (Get-Command ait-server.exe).Source;" +
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
      recorder.run(
        winget,
        [
          "uninstall",
          "--id",
          config.endpoints.winget.identity,
          "--exact",
          "--disable-interactivity",
        ],
        { label: "WinGet uninstall" },
      );
      const readback = spawnSync("where.exe", ["ait.exe"], { encoding: "utf8" });
      if (readback.status === 0) {
        fail("WinGet uninstall retained the ait portable alias");
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

function dockerImageDigest(recorder, reference) {
  const result = recorder.run(
    "docker",
    ["image", "inspect", "--format", "{{json .RepoDigests}}", reference],
    { label: "OCI image digest readback" },
  );
  const digests = JSON.parse(result.stdout);
  return Array.isArray(digests) ? digests : [];
}

function ociContext(config, status, row, version, recorder, candidate) {
  requireCommand("docker");
  const platform = dockerPlatform(row);
  const reference = candidate
    ? `${row.identity}@${candidateOciDigest(status, row)}`
    : `${row.identity}:${version}`;
  recorder.run("docker", ["pull", "--platform", platform, reference], {
    label: candidate ? "OCI candidate digest pull" : "OCI exact prior version pull",
  });
  const digests = dockerImageDigest(recorder, reference);
  if (candidate && !digests.some((value) => value.endsWith(`@${candidateOciDigest(status, row)}`))) {
    fail("OCI candidate pull did not resolve to the dossier digest");
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
      try {
        const portText = recorder
          .run("docker", ["port", name, "8088/tcp"], { label: "OCI server port readback" })
          .stdout.trim();
        const match = portText.match(/:(\d+)$/);
        if (!match) {
          fail("OCI server published port is invalid");
        }
        await pollHealth(`http://127.0.0.1:${match[1]}/healthz`);
      } finally {
        recorder.run("docker", ["rm", "--force", name], {
          label: "OCI explicit server stop",
          allowed: [0, 1],
        });
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
  const commands = {
    github: ["node"],
    pypi: [process.platform === "win32" ? "python" : commandExists("python3") ? "python3" : "python"],
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
  switch (row.channel) {
    case "github":
      return githubContext(config, row, version, root, recorder);
    case "pypi":
      return pythonContext(recorder, path.join(root, "pypi-venv"), pythonVersion, upgrade);
    case "npm":
      return nodeContext(recorder, path.join(root, "npm-prefix"), version);
    case "homebrew":
      return homebrewContext(config, version, root, recorder, upgrade);
    case "apt":
      return aptContext(row, version, recorder, upgrade);
    case "winget":
      return wingetContext(config, version, root, recorder, upgrade);
    case "oci":
      return ociContext(config, status, row, version, recorder, candidate);
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
  await verifyCandidateTag(config);
  probePackageManager(row, recorder);
  mark(checks, "runner_target");
  mark(checks, "package_manager");
  if (row.channel === "apt") {
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
  await verifyCandidateTag(config);
  probePackageManager(row, recorder);
  mark(checks, "runner_target");
  mark(checks, "package_manager");
  if (row.channel === "apt") {
    await configureApt(config, root, recorder);
  }
  const priorRoot = row.channel === "github" ? path.join(root, "prior-github") : root;
  mkdirSync(priorRoot, { recursive: true, mode: 0o755 });
  const prior = await installChannel({
    config,
    status,
    row,
    version: options["prior-version"],
    pythonVersion: options["prior-python-version"],
    root: priorRoot,
    recorder,
    upgrade: false,
    candidate: false,
  });
  exactContextVersion(prior, row, options["prior-version"], recorder);
  exerciseBinding(prior, row);
  mark(checks, "prior_exact_install");
  let priorState = null;
  let repositoryRoot = null;
  if (row.lifecycle === "product") {
    if (!prior.ait) {
      fail("prior product did not expose ait for state creation");
    }
    repositoryRoot = path.join(root, "upgrade-repository");
    priorState = initializePriorState(recorder, prior.ait, repositoryRoot);
    mark(checks, "prior_state");
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
    release: releaseBinding(config, options),
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
