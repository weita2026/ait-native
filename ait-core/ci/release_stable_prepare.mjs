#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TASK_RE = /^[A-Z]+T-[0-9]{4}$/;
const SNAPSHOT_RE = /^SNP-[0-9A-F]{12}$/;
const COMPONENTS = ["core", "server", "runner", "python", "node"];
const REMOTE_COMPONENTS = new Set(["runner", "python", "node"]);

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail("usage: release_stable_prepare.mjs <plan|run> --request <absolute-request.json>", 64);
}

function parseCli(argv) {
  const mode = argv.shift();
  if (!new Set(["plan", "run"]).has(mode) || argv.shift() !== "--request" || argv.length !== 1) usage();
  const requestPath = argv[0];
  if (!path.isAbsolute(requestPath)) fail("stable release request path must be absolute", 64);
  return { mode, requestPath: path.resolve(requestPath) };
}

function readJson(file, label = file) {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    fail(`${label} is invalid JSON`);
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function stable(value) {
  return /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(value ?? "");
}

function stableAdvance(prior, next) {
  if (!stable(prior) || !stable(next)) return false;
  const left = prior.split(".").map(Number);
  const right = next.split(".").map(Number);
  return right[0] === left[0]
    ? right[1] === left[1]
      ? right[2] === left[2] + 1
      : right[1] === left[1] + 1 && right[2] === 0
    : right[0] === left[0] + 1 && right[1] === 0 && right[2] === 0;
}

function absolute(value, label) {
  if (typeof value !== "string" || !path.isAbsolute(value)) fail(`${label} must be absolute`);
  return path.resolve(value);
}

function realDirectory(value, label) {
  const resolved = absolute(value, label);
  if (!existsSync(resolved) || !lstatSync(resolved).isDirectory() || lstatSync(resolved).isSymbolicLink()) {
    fail(`${label} must be a real directory`, 66);
  }
  return resolved;
}

function regularFile(value, label) {
  const resolved = absolute(value, label);
  if (!existsSync(resolved) || !lstatSync(resolved).isFile() || lstatSync(resolved).isSymbolicLink()) {
    fail(`${label} must be a regular file`, 66);
  }
  return resolved;
}

function executable(value, label) {
  const resolved = regularFile(value, label);
  try { accessSync(resolved, constants.X_OK); } catch { fail(`${label} must be executable`, 66); }
  return resolved;
}

function validateRequest(requestPath) {
  const bytes = readFileSync(requestPath);
  const request = readJson(requestPath, "stable release request");
  if (request.contract !== "ait.release.request/v1" || request.release?.channel !== "stable") {
    fail("stable release request contract is invalid");
  }
  if (!stableAdvance(request.release.prior_version, request.release.version)) {
    fail("stable release request must be one patch, minor, or major advance");
  }
  request.records_root = absolute(request.records_root, "records_root");
  const roots = request.roots ?? {};
  for (const component of COMPONENTS) roots[component] = realDirectory(roots[component], `roots.${component}`);
  request.roots = roots;
  const paths = request.paths ?? {};
  for (const key of ["public_source", "web_test_root", "personal_root"]) {
    paths[key] = realDirectory(paths[key], `paths.${key}`);
  }
  for (const key of ["personal_bin", "community_bin", "community_cli_bin", "chrome"]) {
    paths[key] = regularFile(paths[key], `paths.${key}`);
  }
  request.paths = paths;
  if (!/^http:\/\/(127\.0\.0\.1|localhost):[0-9]+\/$/.test(request.web?.server_url ?? "")) {
    fail("stable release Web server URL is invalid");
  }
  request.web.server_data = realDirectory(request.web.server_data, "web.server_data");
  if (!Number.isSafeInteger(request.web.seed) || request.web.seed <= 0 || request.web.seed >= 2147483647) fail("stable release Web seed is invalid");
  if (!/^[^/]+\/[^/]+$/.test(request.github_repository ?? "") || !/^[^/]+\/[^/]+$/.test(request.winget_fork ?? "")) {
    fail("stable release GitHub repository input is invalid");
  }
  if (request.prior) {
    request.prior.family = regularFile(request.prior.family, "prior.family");
    request.prior.coordinator = regularFile(request.prior.coordinator, "prior.coordinator");
  }
  for (const [component, row] of Object.entries(request.adopt ?? {})) {
    if (!COMPONENTS.includes(component) || typeof row !== "object" || row === null) fail("stable release adoption input is invalid");
    if (!TASK_RE.test(row.task ?? "")) fail(`adopt.${component}.task is invalid`);
    row.edit_root = absolute(row.edit_root, `adopt.${component}.edit_root`);
    if (row.snapshot !== undefined && !SNAPSHOT_RE.test(row.snapshot)) fail(`adopt.${component}.snapshot is invalid`);
    if (!new Set(["active", "finished_local", "snapshot_ready"]).has(row.stage)) fail(`adopt.${component}.stage is invalid`);
  }
  const refresh = request.server_refresh;
  if (!refresh || !Array.isArray(refresh.stop_argv) || refresh.stop_argv.length === 0 ||
      !Array.isArray(refresh.start_argv) || refresh.start_argv.length === 0 ||
      typeof refresh.health_url !== "string" || !/^http:\/\/(127\.0\.0\.1|localhost):[0-9]+\/healthz$/.test(refresh.health_url)) {
    fail("stable release request requires an exact server_refresh lifecycle");
  }
  for (const field of ["stop_argv", "start_argv"]) {
    if (refresh[field].some((entry) => typeof entry !== "string" || entry.length === 0)) fail(`server_refresh.${field} is invalid`);
  }
  if (refresh.env !== undefined && (typeof refresh.env !== "object" || refresh.env === null || Array.isArray(refresh.env) ||
      Object.entries(refresh.env).some(([key, value]) => !/^[A-Z][A-Z0-9_]*$/.test(key) || typeof value !== "string"))) {
    fail("server_refresh.env is invalid");
  }
  return { request, requestHash: sha256(bytes) };
}

function releasePath() {
  const entries = [
    path.dirname(process.execPath),
    path.join(os.homedir(), ".cargo", "bin"),
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    process.env.PATH ?? "",
  ];
  return [...new Set(entries.filter(Boolean))].join(path.delimiter);
}

function findCommand(name, searchPath) {
  for (const directory of searchPath.split(path.delimiter)) {
    const candidate = path.join(directory, name);
    try {
      if (statSync(candidate).isFile()) { accessSync(candidate, constants.X_OK); return candidate; }
    } catch {}
  }
  return null;
}

async function authorityPreflight(coreRoot) {
  const config = readJson(path.join(coreRoot, ".ait", "config.json"), "Core AIT config");
  const remoteName = config.default_remote ?? "origin";
  const remote = config.remotes?.[remoteName]?.url;
  if (typeof remote !== "string" || !/^https?:\/\/[^\s]+$/.test(remote)) fail("Core AIT remote URL is unavailable");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 5000);
  let response;
  try {
    response = await fetch(`${remote.replace(/\/$/, "")}/v1/native/repository-authorities/0`, { signal: controller.signal });
  } catch {
    fail("AIT Repository authority API is unavailable before release preparation", 69);
  } finally {
    clearTimeout(timer);
  }
  if (!response.ok) fail(`AIT Repository authority API returned HTTP ${response.status} before release preparation`, 69);
  const payload = await response.json().catch(() => null);
  const name = payload?.repository?.repository_name ?? payload?.repository?.name ?? payload?.repository?.repo_name;
  if (name !== "ait-core") fail("AIT Repository authority 0 is not ait-core");
  return remote;
}

function ensureVersion(document, expected, label) {
  if (document?.family?.version !== expected && document?.package?.version !== expected) {
    fail(`${label} version is not ${expected}`);
  }
}

function writeJsonAtomic(file, value) {
  const temporary = `${file}.tmp`;
  writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  renameSync(temporary, file);
}

function copyOrVerify(source, destination, label = destination) {
  const sourceBytes = readFileSync(source);
  if (existsSync(destination)) {
    if (!lstatSync(destination).isFile() || lstatSync(destination).isSymbolicLink()) {
      fail(`${label} is not a regular retained file`);
    }
    if (sha256(readFileSync(destination)) !== sha256(sourceBytes)) {
      fail(`${label} differs from its create-once source`);
    }
    return destination;
  }
  const temporary = `${destination}.pending`;
  if (existsSync(temporary)) rmSync(temporary, { force: true });
  writeFileSync(temporary, sourceBytes, { mode: 0o600, flag: "wx" });
  renameSync(temporary, destination);
  return destination;
}

function writeOrVerify(destination, bytes, label = destination) {
  const expected = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  if (existsSync(destination)) {
    if (!lstatSync(destination).isFile() || lstatSync(destination).isSymbolicLink()) {
      fail(`${label} is not a regular retained file`);
    }
    if (sha256(readFileSync(destination)) !== sha256(expected)) {
      fail(`${label} differs from its create-once content`);
    }
    return destination;
  }
  const temporary = `${destination}.pending`;
  if (existsSync(temporary)) rmSync(temporary, { force: true });
  writeFileSync(temporary, expected, { mode: 0o600, flag: "wx" });
  renameSync(temporary, destination);
  return destination;
}

function materializeDirectory(destination, produce, verify) {
  if (existsSync(destination)) {
    if (!lstatSync(destination).isDirectory() || lstatSync(destination).isSymbolicLink()) {
      fail(`retained release output is not a real directory: ${destination}`);
    }
    verify(destination);
    return destination;
  }
  const pending = `${destination}.pending`;
  if (existsSync(pending)) rmSync(pending, { recursive: true, force: true });
  produce(pending);
  if (!existsSync(pending) || !lstatSync(pending).isDirectory() || lstatSync(pending).isSymbolicLink()) {
    fail(`release step did not produce a real directory: ${destination}`);
  }
  verify(pending);
  renameSync(pending, destination);
  return destination;
}

function writeExact(file, content) {
  if (existsSync(file)) {
    if (readFileSync(file, "utf8") !== content) fail(`existing generated release card differs: ${file}`);
    return;
  }
  mkdirSync(path.dirname(file), { recursive: true, mode: 0o755 });
  writeFileSync(file, content, { mode: 0o644, flag: "wx" });
}

function count(text, token) {
  return text.split(token).length - 1;
}

function replaceExact(file, from, to, expected, mutate = true) {
  const text = readFileSync(file, "utf8");
  const oldCount = count(text, from);
  const newCount = count(text, to);
  if (from === to) {
    if (oldCount !== expected) {
      throw new Error(`release token inventory differs in ${file}: expected ${expected}, found=${oldCount}`);
    }
    return;
  }
  if (oldCount === expected && newCount === 0) {
    if (mutate) writeFileSync(file, text.split(from).join(to));
  } else if (oldCount === 0 && newCount === expected) {
    return;
  } else {
    throw new Error(`release token inventory differs in ${file}: expected ${expected}, old=${oldCount}, new=${newCount}`);
  }
}

function replacePackageVersion(file, packageName, from, to, mutate = true) {
  const oldToken = `name = "${packageName}"\nversion = "${from}"`;
  const newToken = `name = "${packageName}"\nversion = "${to}"`;
  replaceExact(file, oldToken, newToken, 1, mutate);
}

function manifestSnapshot(file) {
  const text = readFileSync(file, "utf8");
  const rows = [...text.matchAll(/^snapshot = "(SNP-[0-9A-F]{12})"$/gm)];
  if (rows.length !== 1) throw new Error(`external manifest Snapshot inventory differs: ${file}`);
  return rows[0][1];
}

function replaceComponentSources(root, component, priorVersion, version, coreSnapshot, mutate = true) {
  if (component === "core") {
    replaceExact(path.join(root, "rust", "Cargo.toml"), priorVersion, version, 1, mutate);
    const lock = path.join(root, "rust", "Cargo.lock");
    for (const name of ["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]) {
      replacePackageVersion(lock, name, priorVersion, version, mutate);
    }
    for (const [relative, expected] of [
      ["ait-release.json", 1], ["ait-release-family.json", 9],
      ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1],
    ]) replaceExact(path.join(root, relative), priorVersion, version, expected, mutate);
    return ["ait-release-family.json", "ait-release.json", "ci/native_bootstrap_matrix.json", "ci/release_repository_authorities.json", "rust/Cargo.lock", "rust/Cargo.toml"];
  }
  if (component === "server") {
    replaceExact(path.join(root, "rust/crates/ait-server/Cargo.toml"), priorVersion, version, 1, mutate);
    replacePackageVersion(path.join(root, "rust/Cargo.lock"), "ait-server", priorVersion, version, mutate);
    replaceExact(path.join(root, "ait-release.json"), priorVersion, version, 1, mutate);
    replaceExact(path.join(root, "rust/crates/ait-server/src/router/http/binary_tests.rs"), priorVersion, version, 1, mutate);
    return ["ait-release.json", "rust/Cargo.lock", "rust/crates/ait-server/Cargo.toml", "rust/crates/ait-server/src/router/http/binary_tests.rs"];
  }
  const oldPin = manifestSnapshot(path.join(root, "ait-external.toml"));
  if (!SNAPSHOT_RE.test(coreSnapshot ?? "")) throw new Error("accepted Core Snapshot is required before consumer preparation");
  if (component === "runner") {
    replaceExact(path.join(root, "Cargo.toml"), priorVersion, version, 1, mutate);
    replacePackageVersion(path.join(root, "Cargo.lock"), "ait-runner", priorVersion, version, mutate);
    for (const relative of ["ait-release.json", "ait-external.toml", "ait-external.lock"]) {
      replaceExact(path.join(root, relative), priorVersion, version, 1, mutate);
    }
    for (const relative of ["ait-external.toml", "ait-external.lock", "tests/cli.rs"]) {
      replaceExact(path.join(root, relative), oldPin, coreSnapshot, 1, mutate);
    }
    return ["Cargo.lock", "Cargo.toml", "ait-external.lock", "ait-external.toml", "ait-release.json", "tests/cli.rs"];
  }
  if (component === "python") {
    const counts = new Map([
      ["pyproject.toml", 1], ["ait-release.json", 7], ["ait-external.toml", 1], ["ait-external.lock", 1],
      ["src/ait_python/__init__.py", 1], ["tests/test_package.py", 2], ["tests/test_release_adapter.py", 14],
    ]);
    for (const [relative, expected] of counts) replaceExact(path.join(root, relative), priorVersion, version, expected, mutate);
    for (const relative of ["ait-external.toml", "ait-external.lock", "tests/test_package.py"]) {
      replaceExact(path.join(root, relative), oldPin, coreSnapshot, 1, mutate);
    }
    return [...counts.keys()].sort();
  }
  const counts = new Map([
    ["package.json", 7], ["ait-release.json", 8], ["ait-external.toml", 1], ["ait-external.lock", 1],
    ["lib/npm-payload-contract.json", 7], ["test/payload-package.test.js", 3], ["test/package.test.js", 1],
    ["test/cli.test.js", 2], ["test/release-adapter.test.js", 9], ["test/runtime.test.js", 1],
    ["ci/run.ps1", 2], ["ci/run.sh", 2], ["release/release-adapter.mjs", 2],
    ["release/npm-payload-package.mjs", 1], ["scripts/installed-smoke.mjs", 1],
    ["scripts/native-build.mjs", 1], ["src/runtime.js", 1],
  ]);
  for (const [relative, expected] of counts) replaceExact(path.join(root, relative), priorVersion, version, expected, mutate);
  replaceExact(path.join(root, "test/package.test.js"), priorVersion.replaceAll(".", "\\."), version.replaceAll(".", "\\."), 1, mutate);
  replaceExact(path.join(root, "test/release-adapter.test.js"), priorVersion.replaceAll(".", "\\."), version.replaceAll(".", "\\."), 2, mutate);
  const pinCounts = new Map([
    ["ait-external.toml", 1], ["ait-external.lock", 1], ["lib/npm-payload-contract.json", 6],
    ["test/package.test.js", 2], ["ci/run.ps1", 1], ["ci/run.sh", 1],
    ["release/release-adapter.mjs", 1], ["release/npm-payload-package.mjs", 1],
    ["scripts/native-build.mjs", 1], ["src/runtime.js", 1],
  ]);
  for (const [relative, expected] of pinCounts) replaceExact(path.join(root, relative), oldPin, coreSnapshot, expected, mutate);
  return [...counts.keys()].sort();
}

function card(component, version, snapshot = null) {
  const cap = { core: "Core", server: "Server", runner: "Runner", python: "Python", node: "Node" }[component];
  const ref = `release-${version.replaceAll(".", "-")}/${component}-version`;
  const binding = snapshot ? ` and bind Core ${snapshot}` : "";
  return {
    ref,
    relative: `docs/sprints/release_${version.replaceAll(".", "_")}_automated.md`,
    content: `# Stable ${version} ${cap} Preparation [plan-ref: ait-${component}/release/${version}-automated]\n\n## Goal\n\nPrepare the exact ${cap} source for stable ${version}${binding}.\n\n## Scope\n\n- Change only the admitted version and dependency authority files.\n- Preserve behavior, dependencies, licenses, and historical releases.\n\n## Acceptance criteria\n\n- The normalized diff contains only the declared release paths.\n- The repository test entrypoint passes.\n- The public Task and Snapshot are recorded for release closeout.\n\n## Work item\n\n- [ ] [ref: ${ref}] Prepare and verify the stable ${version} ${cap} source.\n`,
  };
}

function coordinatorCard(version, coreSnapshot) {
  if (!SNAPSHOT_RE.test(coreSnapshot ?? "")) throw new Error("coordinator Core Snapshot is invalid");
  const identity = coreSnapshot.toLowerCase();
  const ref = `release-${version.replaceAll(".", "-")}/family-coordinator-${identity}`;
  return {
    ref,
    relative: `docs/sprints/release_${version.replaceAll(".", "_")}_coordinator_${identity}.md`,
    content: `# Stable ${version} Family Coordination [plan-ref: ait-core/release/${version}-coordinator-${identity}]\n\n## Goal\n\nBind the five accepted component Snapshots for stable ${version}.\n\n## Scope\n\n- Change only component source Snapshot selectors in the root family manifest.\n- Preserve versions, distribution policy, and component order.\n\n## Acceptance criteria\n\n- Every family component selects its prepared public Snapshot.\n- The normalized diff contains only ait-release-family.json.\n- The coordinator Task and Snapshot are retained for source export.\n\n## Work item\n\n- [ ] [ref: ${ref}] Bind the stable ${version} component Snapshots.\n`,
  };
}

function acceptedComponentCloseouts(request, state) {
  return COMPONENTS.map((component) => {
    const row = component === "core" ? state.coordinator : state.components[component];
    if (
      !row || !TASK_RE.test(row.task ?? "") || !SNAPSHOT_RE.test(row.snapshot ?? "") ||
      typeof row.edit_root !== "string" || !path.isAbsolute(row.edit_root) ||
      !new Set(["finished_local", "snapshot_ready"]).has(row.stage)
    ) {
      fail(`release closeout identity is incomplete: ${component}`);
    }
    return {
      id: component,
      task: row.task,
      snapshot: row.snapshot,
      patchset: null,
      repository_root: request.roots[component],
      edit_root: row.edit_root,
      remote: "origin",
      finish_local_before_ready: row.stage === "finished_local",
      review_message: reviewMessage(component),
    };
  });
}

function closeoutReadinessProbes(ait, closeouts) {
  return closeouts.map((row) => ({
    id: `preflight-closeout-${row.id}`,
    cwd: row.finish_local_before_ready === true ? row.repository_root : row.edit_root,
    argv: [ait, "workflow", "ready", row.task, "--remote", row.remote ?? "origin"],
  }));
}

function reviewMessage(component) {
  const cap = component[0].toUpperCase() + component.slice(1);
  return `Reviewed files: ${cap} stable release authority paths. Findings: Version and source bindings match the frozen release request. Risks: Limited to the validated release delta. Tests: The repository release entrypoint and required dependency checks passed. Recommendation: Approve.`;
}

async function main() {
  const { mode, requestPath } = parseCli(process.argv.slice(2));
  const { request, requestHash } = validateRequest(requestPath);
  const searchPath = releasePath();
  const requiredCommands = ["bash", "cargo", "diff", "gh", "git", "jq", "node", "npm", "python3", "tar"];
  const tools = Object.fromEntries(requiredCommands.map((name) => [name, findCommand(name, searchPath)]));
  for (const [name, value] of Object.entries(tools)) if (!value) fail(`required release command is unavailable: ${name}`, 69);
  const ait = executable(path.join(request.roots.core, ".ait/cargo-target/release/ait-cli"), "canonical AIT CLI");
  for (const field of ["stop_argv", "start_argv"]) {
    const command = request.server_refresh[field][0];
    if (!path.isAbsolute(command)) fail(`server_refresh.${field}[0] must be an absolute executable`, 64);
    executable(command, `server_refresh.${field}[0]`);
  }
  const auth = spawnSync(tools.gh, ["auth", "status"], { env: { ...process.env, PATH: searchPath }, stdio: "ignore" });
  if (auth.status !== 0) fail("GitHub CLI authentication is unavailable before release preparation", 69);
  const remoteUrl = await authorityPreflight(request.roots.core);
  const inventoryPin = "SNP-000000000000";
  for (const component of COMPONENTS) {
    replaceComponentSources(
      request.roots[component],
      component,
      request.release.prior_version,
      request.release.version,
      REMOTE_COMPONENTS.has(component) ? inventoryPin : null,
      false,
    );
  }
  const plan = [
    "preflight", "validate_inventories", "freeze_prior", "prepare_core", "verify_core_equivalence", "prepare_server",
    "prepare_runner", "prepare_python", "prepare_node", "coordinate_family", "refresh_coordinator_artifacts", "preflight_component_closeouts", "compile_accepted_input", "run_conductor",
  ];
  if (mode === "plan") {
    process.stdout.write(`${JSON.stringify({ contract: "ait.release.prepare-plan/v1", status: "pass", version: request.release.version, prior_version: request.release.prior_version, remote: remoteUrl, phases: plan }, null, 2)}\n`);
    return;
  }

  mkdirSync(request.records_root, { recursive: true, mode: 0o700 });
  const logs = path.join(request.records_root, "preparation-logs");
  mkdirSync(logs, { recursive: true, mode: 0o700 });
  const statePath = path.join(request.records_root, "preparation-state.json");
  let state;
  if (existsSync(statePath)) {
    state = readJson(statePath, "preparation state");
    if (state.contract !== "ait.release.preparation-state/v1" || state.request_sha256 !== requestHash) {
      fail("release request differs from the immutable preparation state");
    }
  } else {
    state = { contract: "ait.release.preparation-state/v1", request_sha256: requestHash, status: "running", version: request.release.version, steps: {}, components: {} };
    writeJsonAtomic(statePath, state);
  }
  const save = () => writeJsonAtomic(statePath, state);
  const logPath = (label) => path.join(logs, `${label}.log`);
  const runResult = (label, cwd, argv, extraEnv = {}, allowed = [0]) => {
    const result = spawnSync(argv[0], argv.slice(1), {
      cwd, env: { ...process.env, PATH: searchPath, ...extraEnv }, encoding: "utf8", maxBuffer: 64 * 1024 * 1024,
    });
    writeFileSync(logPath(label), `${result.stdout ?? ""}${result.stderr ?? ""}`, { mode: 0o600 });
    if (!allowed.includes(result.status)) fail(`release preparation step failed: ${label}; see ${logPath(label)}`, result.status || 1);
    return result;
  };
  const run = (label, cwd, argv, extraEnv = {}) => {
    return runResult(label, cwd, argv, extraEnv).stdout ?? "";
  };
  const runJson = (label, cwd, argv, extraEnv = {}) => {
    const stdout = run(label, cwd, argv, extraEnv);
    try { return JSON.parse(stdout); } catch { fail(`release preparation step returned invalid JSON: ${label}`); }
  };
  const done = (id) => state.steps[id]?.status === "pass";
  const complete = (id, data = {}) => { state.steps[id] = { status: "pass", ...data }; save(); };
  const refreshCoreArtifacts = (label) => {
    run(`${label}-build`, request.roots.core, [path.join(request.roots.core, "ait.sh"), "core", "build"]);
    run(`${label}-install`, request.roots.core, [path.join(request.roots.core, "ait.sh"), "core", "install", "--skip-build"]);
    const versionText = run(`${label}-version`, request.roots.core, [ait, "--version"]).trim();
    if (versionText !== `ait ${request.release.version}`) fail("canonical AIT CLI version differs after Core finish");
    const installed = executable(path.join(os.homedir(), ".local", "bin", "ait"), "installed canonical AIT CLI");
    if (sha256(readFileSync(installed)) !== sha256(readFileSync(ait))) fail("installed AIT CLI differs from the canonical artifact");
    const help = run(`${label}-snapshot-help`, request.roots.core, [ait, "snapshot", "create", "--help"]);
    if (/--profile|--intent|--validation/.test(help)) fail("canonical Snapshot help exposes a removed option");
  };

  if (!done("freeze_prior")) {
    const priorFamily = path.join(request.records_root, "prior-family.json");
    const priorCoordinator = path.join(request.records_root, "prior-coordinator.json");
    if (request.prior) {
      copyOrVerify(request.prior.family, priorFamily, "frozen prior family");
      copyOrVerify(request.prior.coordinator, priorCoordinator, "frozen prior coordinator");
    } else {
      const family = readJson(path.join(request.roots.core, "ait-release-family.json"));
      ensureVersion(family, request.release.prior_version, "canonical prior family");
      copyOrVerify(path.join(request.roots.core, "ait-release-family.json"), priorFamily, "frozen prior family");
      const status = runJson("prior-core-status", request.roots.core, [ait, "status", "--json"]);
      if (!SNAPSHOT_RE.test(status.head_snapshot_id ?? "")) fail("canonical prior Core head Snapshot is invalid");
      const coordinator = run("prior-core-coordinator", request.roots.core, [ait, "snapshot", "show", status.head_snapshot_id, "--json"]);
      writeOrVerify(priorCoordinator, coordinator, "frozen prior coordinator");
    }
    ensureVersion(readJson(priorFamily), request.release.prior_version, "frozen prior family");
    complete("freeze_prior", { family: priorFamily, coordinator: priorCoordinator });
  }

  const taskPayload = (component, row) => {
    const argv = [ait, "task", "show", row.task];
    if (!REMOTE_COMPONENTS.has(component)) argv.push("--local");
    argv.push("--json");
    const payload = runJson(`verify-task-${component}`, request.roots[component], argv);
    if (payload.task_id !== row.task) fail(`${component} Task readback differs from the recorded public Task`);
    return payload;
  };
  const verifySnapshot = (component, row) => {
    const payload = runJson(`verify-snapshot-${component}`, request.roots[component], [ait, "snapshot", "show", row.snapshot, "--json"]);
    if (payload.snapshot_id !== row.snapshot || !String(payload.line_name ?? "").startsWith(`feature/${row.task.toLowerCase()}`)) {
      fail(`${component} Snapshot does not belong to the recorded Task feature Line`);
    }
  };
  const validatePreparedRow = (component, row) => {
    if (!TASK_RE.test(row.task ?? "") || typeof row.edit_root !== "string" || !path.isAbsolute(row.edit_root)) {
      fail(`${component} preparation has no public Task and absolute edit root`);
    }
    if (!new Set(["active", "finished_local", "snapshot_ready"]).has(row.stage)) {
      fail(`${component} preparation stage is invalid`);
    }
    if (row.stage !== "active" && !SNAPSHOT_RE.test(row.snapshot ?? "")) {
      fail(`${component} prepared stage has no Snapshot`);
    }
  };

  const prepareComponent = (component, coreSnapshot = null) => {
    if (done(`prepare_${component}`)) return state.components[component];
    const root = request.roots[component];
    let row = state.components[component] ?? (request.adopt?.[component] ? { ...request.adopt[component] } : null);
    if (!row) {
      const generated = card(component, request.release.version, coreSnapshot);
      writeExact(path.join(root, generated.relative), generated.content);
      const args = [ait, "task", "start", "--from", `${generated.relative}#${generated.ref}`, "--intent", `Prepare ${component} stable ${request.release.version} release authority`];
      if (REMOTE_COMPONENTS.has(component)) args.push("--remote", "origin");
      else args.push("--local");
      args.push("--json");
      const started = runJson(`start-${component}`, root, args);
      row = {
        task: started.task_id,
        edit_root: started.edit_root,
        feature_line: started.worktree?.registered_line_name ?? `feature/${String(started.task_id).toLowerCase()}`,
        stage: "active",
        progress: "started",
      };
      validatePreparedRow(component, row);
      state.components[component] = row;
      save();
    } else if (!state.components[component]) {
      validatePreparedRow(component, row);
      state.components[component] = row;
      save();
    }
    validatePreparedRow(component, row);

    if (row.stage !== "active") {
      const task = taskPayload(component, row);
      const expectedStatus = row.stage === "finished_local" ? "completed" : "active";
      if (task.status !== expectedStatus) fail(`${component} adopted Task status differs from ${expectedStatus}`);
      verifySnapshot(component, row);
    } else {
      if (row.progress !== "finalizing") {
        if (!existsSync(row.edit_root) || !lstatSync(row.edit_root).isDirectory()) fail(`${component} edit root is unavailable`, 66);
        if (row.progress !== "sources_validated" && row.progress !== "tested") {
          const expectedPaths = replaceComponentSources(row.edit_root, component, request.release.prior_version, request.release.version, coreSnapshot);
          if (REMOTE_COMPONENTS.has(component)) {
            const external = runJson(`external-${component}`, row.edit_root, [ait, "external", "update", "ait-core", "--to", coreSnapshot, "--validate", "--no-recursive", "--json"]);
            if (external.validated !== true || external.validation?.summary?.passed !== true) fail(`${component} external validation did not pass`);
          }
          const diff = runJson(`diff-${component}`, row.edit_root, [ait, "diff", "--json"]);
          if (JSON.stringify([...(diff.changed_paths ?? [])].sort()) !== JSON.stringify([...expectedPaths].sort())) {
            fail(`${component} release diff paths differ from the locked inventory`);
          }
          const status = runJson(`worktree-status-${component}`, row.edit_root, [ait, "status", "--json"]);
          row.feature_line = status.line_name ?? row.feature_line ?? `feature/${row.task.toLowerCase()}`;
          row.base_snapshot = status.head_snapshot_id ?? diff.baseline_snapshot_id ?? null;
          row.progress = "sources_validated";
          save();
        }
        if (row.progress !== "tested") {
          const testArgv = component === "core"
            ? [tools.cargo, "check", "--manifest-path", "rust/Cargo.toml", "--locked", "-p", "ait-cli", "-p", "ait-agent-worker"]
            : [path.join(row.edit_root, "ci/run.sh"), "patchset"];
          run(`test-${component}`, row.edit_root, testArgv);
          row.progress = "tested";
          save();
        }
        row.progress = "finalizing";
        save();
      }

      const currentTask = taskPayload(component, row);
      if (component === "core" || component === "server") {
        if (currentTask.status === "completed") {
          const target = runJson(`recover-finish-${component}`, root, [ait, "line", "show", "main", "--json"]);
          row.snapshot = target.head_snapshot_id;
        } else if (currentTask.status === "active") {
          const finished = runJson(`finish-${component}`, row.edit_root, [ait, "task", "finish", row.task, "--message", `Prepare ${component} stable ${request.release.version}`, "--local", "--json"]);
          row.snapshot = finished.landed_snapshot_id;
        } else {
          fail(`${component} Task cannot resume from status ${currentTask.status}`);
        }
        row.stage = "finished_local";
      } else {
        const status = runJson(`recover-snapshot-status-${component}`, row.edit_root, [ait, "status", "--json"]);
        if (SNAPSHOT_RE.test(status.head_snapshot_id ?? "") && status.head_snapshot_id !== row.base_snapshot) {
          row.snapshot = status.head_snapshot_id;
        } else {
          const snapshot = runJson(`snapshot-${component}`, row.edit_root, [ait, "snapshot", "create", row.task, "--message", `Prepare ${component} stable ${request.release.version}`, "--json"]);
          row.snapshot = snapshot.snapshot_id ?? snapshot.created_snapshot_id ?? snapshot.head_snapshot_id;
        }
        row.stage = "snapshot_ready";
      }
      delete row.progress;
      if (!SNAPSHOT_RE.test(row.snapshot ?? "")) fail(`${component} preparation did not produce a Snapshot`);
      verifySnapshot(component, row);
      save();
    }
    state.components[component] = row;
    complete(`prepare_${component}`, { task: row.task, snapshot: row.snapshot, stage: row.stage });
    process.stdout.write(`${component}: ${row.task} ${row.snapshot}\n`);
    return row;
  };

  const core = prepareComponent("core");
  if (!done("publish_core_source")) {
    refreshCoreArtifacts("core");
    const line = `release/${request.release.version}-core-source`;
    const listed = runJson("core-line-list", request.roots.core, [ait, "line", "list", "--json"]);
    const rows = Array.isArray(listed) ? listed : listed.lines ?? [];
    const existing = rows.find((entry) => entry.line_name === line || entry.name === line);
    if (existing) {
      const head = existing.head_snapshot_id ?? existing.snapshot_id;
      if (head !== core.snapshot) fail("existing Core release Line points at another Snapshot");
    } else {
      runJson("core-line-create", request.roots.core, [ait, "line", "create", line, "--from-snapshot", core.snapshot, "--json"]);
    }
    runJson("core-line-push", request.roots.core, [ait, "push", "--remote", "origin", "--line", line, "--json"]);
    complete("publish_core_source", { line, snapshot: core.snapshot });
  }

  if (!done("verify_core_equivalence")) {
    const priorCoordinator = readJson(state.steps.freeze_prior.coordinator, "frozen prior coordinator");
    const priorSnapshot = priorCoordinator.snapshot_id;
    if (!SNAPSHOT_RE.test(priorSnapshot ?? "")) fail("prior coordinator Snapshot is invalid");
    const sourceCache = (label, snapshot, version) => {
      const parent = path.join(request.records_root, `${label}-core-cache`);
      const verifyCache = (candidate) => {
        const destination = path.join(candidate, "ait-core");
        if (!existsSync(destination) || !lstatSync(destination).isDirectory() || lstatSync(destination).isSymbolicLink()) {
          fail(`${label} Core source cache is incomplete`);
        }
        const evidence = readJson(path.join(candidate, "source-cache.evidence.json"), `${label} Core source-cache evidence`);
        if (evidence.contract !== "ait.release.source-cache/v1" || evidence.status !== "ready" ||
            evidence.source_snapshot !== snapshot || evidence.version !== version || evidence.workspace_clean !== true) {
          fail(`${label} Core source-cache evidence differs from the frozen request`);
        }
      };
      materializeDirectory(parent, (pending) => {
        const destination = path.join(pending, "ait-core");
        run(`${label}-core-cache`, request.roots.core, [path.join(request.roots.core, "ci/release_source_cache.sh"), ait, "ait-core", "0", "AC", snapshot, version, "Apache-2.0", path.join(request.roots.core, "ci/patch_ci.json"), destination], {
          AIT_RELEASE_SERVER_URL: remoteUrl,
          AIT_RELEASE_SOURCE_EVIDENCE_PATH: path.join(pending, "source-cache.evidence.json"),
        });
      }, verifyCache);
      const normalized = path.join(request.records_root, `${label}-core-normalized`);
      const normalizedRoot = materializeDirectory(normalized, (pending) => {
        run(`${label}-core-normalize`, request.roots.core, [tools.node, path.join(request.roots.core, "ci/release_source_normalize.mjs"), "--source", path.join(parent, "ait-core"), "--output", pending]);
      }, (candidate) => {
        for (const relative of ["ait-release.json", "rust/Cargo.toml", "ci/release_clean_host_phase.mjs"]) {
          if (!existsSync(path.join(candidate, relative)) || !lstatSync(path.join(candidate, relative)).isFile()) {
            fail(`${label} normalized Core source is missing ${relative}`);
          }
        }
      });
      return normalizedRoot;
    };
    const priorRoot = sourceCache("prior", priorSnapshot, request.release.prior_version);
    const finalRoot = sourceCache("final", core.snapshot, request.release.version);
    run("core-source-equivalence", request.roots.core, [tools.node, path.join(request.roots.core, "ci/release_source_equivalence.mjs"), "--prior", priorRoot, "--final", finalRoot, "--policy", path.join(request.roots.core, "release/core-version-equivalence.json"), "--output", path.join(request.records_root, "core-version-equivalence.json")]);
    complete("verify_core_equivalence", { evidence: path.join(request.records_root, "core-version-equivalence.json") });
  }

  const server = prepareComponent("server");
  if (!done("refresh_server")) {
    if (!done("build_server")) {
      run("server-build", request.roots.server, [path.join(request.roots.server, "ait.sh"), "core", "build"]);
      complete("build_server");
    }
    const health = async () => {
      try {
        const response = await fetch(request.server_refresh.health_url, { signal: AbortSignal.timeout(1000) });
        return response.ok;
      } catch { return false; }
    };
    const stopResult = runResult("server-stop", request.roots.server, request.server_refresh.stop_argv, request.server_refresh.env ?? {}, [0, 1]);
    if (stopResult.status !== 0 && await health()) fail("release server stop command failed while the old server remained healthy");
    let stopped = false;
    for (let attempt = 0; attempt < 30; attempt += 1) {
      if (!await health()) { stopped = true; break; }
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    if (!stopped) fail("release server did not stop before restart");
    run("server-start", request.roots.server, request.server_refresh.start_argv, request.server_refresh.env ?? {});
    let healthy = false;
    for (let attempt = 0; attempt < 45; attempt += 1) {
      if (await health()) { healthy = true; break; }
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    if (!healthy) fail("refreshed release server did not pass /healthz");
    complete("refresh_server", { status: "healthy" });
  }
  const runner = prepareComponent("runner", core.snapshot);
  const python = prepareComponent("python", core.snapshot);
  const node = prepareComponent("node", core.snapshot);

  if (!done("coordinate_family")) {
    let coordinator = state.coordinator;
    if (!coordinator) {
      const generated = coordinatorCard(request.release.version, core.snapshot);
      writeExact(path.join(request.roots.core, generated.relative), generated.content);
      const started = runJson("start-coordinator", request.roots.core, [ait, "task", "start", "--from", `${generated.relative}#${generated.ref}`, "--intent", `Bind stable ${request.release.version} family component Snapshots`, "--local", "--json"]);
      if (!TASK_RE.test(started.task_id ?? "") || typeof started.edit_root !== "string" || !path.isAbsolute(started.edit_root)) {
        fail("family coordinator start returned no public Task and edit root");
      }
      coordinator = { task: started.task_id, edit_root: started.edit_root, stage: "active", progress: "started" };
      state.coordinator = coordinator;
      save();
    }
    const selected = { "ait-core": core.snapshot, "ait-server": server.snapshot, "ait-runner": runner.snapshot, "ait-python": python.snapshot, "ait-node": node.snapshot };
    if (coordinator.stage === "active" && coordinator.progress !== "finalizing") {
      if (!existsSync(coordinator.edit_root) || !lstatSync(coordinator.edit_root).isDirectory()) fail("family coordinator edit root is unavailable", 66);
      const familyPath = path.join(coordinator.edit_root, "ait-release-family.json");
      const family = readJson(familyPath);
      for (const component of family.components ?? []) {
        const snapshot = selected[component.source_repository];
        if (!snapshot) fail(`family contains unknown source repository: ${component.source_repository}`);
        component.source_snapshot = snapshot;
      }
      if ((family.components ?? []).length !== 7) fail("family coordinator component inventory differs");
      writeFileSync(familyPath, `${JSON.stringify(family, null, 2)}\n`);
      const diff = runJson("diff-coordinator", coordinator.edit_root, [ait, "diff", "--json"]);
      if (JSON.stringify(diff.changed_paths) !== JSON.stringify(["ait-release-family.json"])) fail("family coordinator changed unexpected paths");
      coordinator.progress = "validated";
      save();
      coordinator.progress = "finalizing";
      save();
    }
    if (coordinator.stage === "active") {
      const task = taskPayload("core", coordinator);
      if (task.status === "completed") {
        const target = runJson("recover-finish-coordinator", request.roots.core, [ait, "line", "show", "main", "--json"]);
        coordinator.snapshot = target.head_snapshot_id;
      } else if (task.status === "active") {
        const finished = runJson("finish-coordinator", coordinator.edit_root, [ait, "task", "finish", coordinator.task, "--message", `Bind stable ${request.release.version} component Snapshots`, "--local", "--json"]);
        coordinator.snapshot = finished.landed_snapshot_id;
      } else {
        fail(`family coordinator Task cannot resume from status ${task.status}`);
      }
      coordinator.stage = "finished_local";
      delete coordinator.progress;
      if (!SNAPSHOT_RE.test(coordinator.snapshot ?? "")) fail("family coordinator finish did not return a Snapshot");
      verifySnapshot("core", coordinator);
      save();
    }
    if (coordinator.stage !== "finished_local" || taskPayload("core", coordinator).status !== "completed") {
      fail("family coordinator retained state is not completed locally");
    }
    verifySnapshot("core", coordinator);
    complete("coordinate_family", { task: coordinator.task, snapshot: coordinator.snapshot, stage: coordinator.stage });
  }

  if (!done("refresh_coordinator_artifacts")) {
    refreshCoreArtifacts("coordinator-core");
    complete("refresh_coordinator_artifacts");
  }

  if (!done("preflight_component_closeouts")) {
    const closeouts = acceptedComponentCloseouts(request, state);
    for (const probe of closeoutReadinessProbes(ait, closeouts)) {
      run(probe.id, probe.cwd, probe.argv);
    }
    complete("preflight_component_closeouts", {
      tasks: closeouts.map(({ id, task }) => ({ id, task })),
    });
  }

  if (!done("compile_accepted_input")) {
    const finalFamily = path.join(request.records_root, "final-family.json");
    const finalCoordinator = path.join(request.records_root, "final-coordinator.json");
    copyOrVerify(path.join(request.roots.core, "ait-release-family.json"), finalFamily, "frozen final family");
    const coordinator = run("final-coordinator-show", request.roots.core, [ait, "snapshot", "show", state.coordinator.snapshot, "--json"]);
    writeOrVerify(finalCoordinator, coordinator, "frozen final coordinator");
    const webComponents = path.join(request.records_root, "web-components.json");
    writeOrVerify(webComponents, `${JSON.stringify(Object.fromEntries(COMPONENTS.map((component) => [component, state.components[component].snapshot])), null, 2)}\n`, "derived Web component Snapshots");
    const closeouts = acceptedComponentCloseouts(request, state);
    const accepted = {
      contract: "ait.release.accepted-input/v1",
      release: request.release,
      records_root: request.records_root,
      github_repository: request.github_repository,
      winget_fork: request.winget_fork,
      paths: {
        core_root: request.roots.core,
        public_source: request.paths.public_source,
        prior_family: state.steps.freeze_prior.family,
        prior_coordinator: state.steps.freeze_prior.coordinator,
        final_family: finalFamily,
        final_coordinator: finalCoordinator,
        web_components: webComponents,
        web_test_root: request.paths.web_test_root,
        personal_root: request.paths.personal_root,
        personal_bin: request.paths.personal_bin,
        community_bin: request.paths.community_bin,
        community_cli_bin: request.paths.community_cli_bin,
        chrome: request.paths.chrome,
      },
      web: request.web,
      component_closeouts: closeouts,
    };
    const acceptedPath = path.join(request.records_root, "accepted-input.json");
    writeJsonAtomic(acceptedPath, accepted);
    state.accepted_input = acceptedPath;
    complete("compile_accepted_input", { path: acceptedPath, sha256: sha256(readFileSync(acceptedPath)) });
  }
  state.status = "prepared";
  save();
  process.stdout.write(`${state.accepted_input}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => fail(error?.message ?? String(error)));
}

export {
  acceptedComponentCloseouts,
  closeoutReadinessProbes,
  coordinatorCard,
  copyOrVerify,
  materializeDirectory,
  replaceComponentSources,
  stableAdvance,
  writeOrVerify,
};
