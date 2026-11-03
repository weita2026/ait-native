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
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import { qualificationCarrier, priorWebInput, correctedWTIdentity, sameProductComponents } from "./release_web_recovery_identity.mjs";
import { enterOperation } from "./release_operation_owner.mjs";
import { originalLocalSnapshot } from "./release_native_workflow.mjs";
import { validateReleaseDependencyGraph } from "./release_runner_dependency.mjs";
import { webCommunityPathsInScope } from "./release_web_controls.mjs";

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

export function validateRequest(requestPath) {
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
  roots.native = realDirectory(roots.native, "roots.native");
  request.roots = roots;
  const paths = request.paths ?? {};
  const personal = Object.hasOwn(paths, "personal_bin") || Object.hasOwn(paths, "personal_root") || Object.hasOwn(request.web ?? {}, "personal_snapshot");
  if (!personal && request.release.version !== "1.2.0") fail("Personal omission is authorized only for 1.2.0");
  const community = webCommunityPathsInScope(paths, request.release, personal);
  if (community !== Object.hasOwn(request.web ?? {}, "community_snapshot")) fail("Community source Snapshot and paths must be selected together");
  for (const key of ["public_source", "web_test_root", ...(personal ? ["personal_root"] : []), ...(community ? ["community_root"] : [])]) {
    paths[key] = realDirectory(paths[key], `paths.${key}`);
  }
  for (const key of [...(personal ? ["personal_bin"] : []), ...(community ? ["community_bin", "community_cli_bin", "chrome"] : [])]) {
    paths[key] = regularFile(paths[key], `paths.${key}`);
  }
  request.paths = paths;
  if (paths.web_components !== undefined) paths.web_components = regularFile(paths.web_components, "paths.web_components");
  if (!/^http:\/\/(127\.0\.0\.1|localhost):[0-9]+\/$/.test(request.web?.server_url ?? "")) {
    fail("stable release Web server URL is invalid");
  }
  request.web.server_data = realDirectory(request.web.server_data, "web.server_data");
  if (!Number.isSafeInteger(request.web.seed) || request.web.seed <= 0 || request.web.seed >= 2147483647) fail("stable release Web seed is invalid");
  for (const key of [...(personal ? ["personal_snapshot"] : []), ...(community ? ["community_snapshot"] : []), "acceptance_snapshot"]) {
    if (!SNAPSHOT_RE.test(request.web[key] ?? "")) fail(`web.${key} must pin an independently completed companion Snapshot`);
  }
  if (!/^[^/]+\/[^/]+$/.test(request.github_repository ?? "") || !/^[^/]+\/[^/]+$/.test(request.winget_fork ?? "")) {
    fail("stable release GitHub repository input is invalid");
  }
  for (const [component, row] of Object.entries(request.adopt ?? {})) {
    if (!COMPONENTS.includes(component) || typeof row !== "object" || row === null) fail("stable release adoption input is invalid");
    if (!TASK_RE.test(row.task ?? "")) fail(`adopt.${component}.task is invalid`);
    row.edit_root = absolute(row.edit_root, `adopt.${component}.edit_root`);
    if (row.snapshot !== undefined && !SNAPSHOT_RE.test(row.snapshot)) fail(`adopt.${component}.snapshot is invalid`);
    if (!new Set(["active", "finished_local", "snapshot_ready"]).has(row.stage)) fail(`adopt.${component}.stage is invalid`);
  }
  if (paths.web_components !== undefined && !admittedSourceRequest(request)) fail("qualified inputs require completed source admission");
  const refresh = request.server_refresh;
  if (refresh === undefined && admittedSourceRequest(request)) return { request, requestHash: sha256(bytes) };
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
  // This is a fixed Product compatibility message, not release authority.
  const retirement = "Chat integrations are retired in AIT 1.2.0";
  const parts = text.split(retirement);
  const oldCount = parts.reduce((total, part) => total + count(part, from), 0);
  const newCount = parts.reduce((total, part) => total + count(part, to), 0);
  if (from === to) {
    if (oldCount !== expected) {
      throw new Error(`release token inventory differs in ${file}: expected ${expected}, found=${oldCount}`);
    }
    return;
  }
  if (oldCount === expected && newCount === 0) {
    if (mutate) writeFileSync(file, parts.map(part => part.split(from).join(to)).join(retirement));
  } else if (oldCount === 0 && newCount === expected) {
    return;
  } else {
    throw new Error(`release token inventory differs in ${file}: expected ${expected}, old=${oldCount}, new=${newCount}`);
  }
}

function replacePythonAdapterTestVersions(file, from, to, mutate) {
  let text = readFileSync(file, "utf8");
  // Only these two assertions follow the package version. The other wheel
  // names/versions are synthetic payload and normalization fixtures.
  const authorities = [
    /(^\s*assert manifest\["package"\]\["version"\] == ")([^"]+)(")/gm,
    /(expected_wheel_name\(target, ')([^']+)('\))/g,
  ];
  for (const pattern of authorities) {
    const matches = [...text.matchAll(pattern)];
    if (matches.length !== 1 || ![from, to].includes(matches[0][2])) {
      throw new Error(`release version assertion inventory differs in ${file}`);
    }
    text = text.replace(pattern, (_match, prefix, _version, suffix) => prefix + to + suffix);
  }
  if (mutate) writeFileSync(file, text);
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

function replaceFamilyVersion(file, from, to, mutate) {
  const family = readJson(file, "family version authority");
  if (family.schema !== "ait.release.family/v3" || !Array.isArray(family.components) || !family.components.length ||
      ![from, to].includes(family.family?.version)) throw new Error(`family version authority is invalid: ${file}`);
  if (![5, 7].includes(family.components.length)) throw new Error(`family token inventory differs: ${file}`);
  replaceExact(file, from, to, family.components.length + 2, mutate);
}

function replaceComponentSources(root, component, priorVersion, version, coreSnapshot, mutate = true) {
  if (component === "native") {
    replaceFamilyVersion(path.join(root, "ait-release-family.json"), priorVersion, version, mutate);
    for (const [relative, expected] of [
      ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1],
    ]) replaceExact(path.join(root, relative), priorVersion, version, expected, mutate);
    const changed = ["ait-release-family.json", "ci/native_bootstrap_matrix.json", "ci/release_repository_authorities.json"];
    for (const relative of ["ait-release.json", "ait-external.toml", "ait-external.lock"]) {
      const file = path.join(root, relative);
      if (!existsSync(file)) continue;
      const text = readFileSync(file, "utf8");
      const pattern = relative.endsWith(".json") ? /("version":\s*")[^"]+(")/g : /(^version = ")[^"]+("$)/gm;
      const updated = text.replace(pattern, (match, before, after, offset) => {
        if (relative === "ait-external.lock") {
          const start = text.lastIndexOf("[[node]]", offset);
          const next = text.indexOf("[[node]]", offset);
          const row = text.slice(start, next < 0 ? undefined : next);
          if (/^parent_path = "\.ait-external\/ait-runner"$/m.test(row)) return match;
        }
        const current = match.slice(before.length, match.length - after.length);
        if (![priorVersion, version].includes(current)) throw new Error(`unexpected release metadata version: ${file}`);
        return before + version + after;
      });
      if (updated !== text && mutate) writeFileSync(file, updated);
      changed.push(relative);
    }
    return changed;
  }
  if (component === "core") {
    replaceExact(path.join(root, "rust", "Cargo.toml"), priorVersion, version, 1, mutate);
    const lock = path.join(root, "rust", "Cargo.lock");
    for (const name of ["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]) {
      replacePackageVersion(lock, name, priorVersion, version, mutate);
    }
    replaceExact(path.join(root, "ait-release.json"), priorVersion, version, 1, mutate);
    const changed = ["ait-release.json", "rust/Cargo.lock", "rust/Cargo.toml"];
    // Release control moved to ait-native. A Core tree created before that
    // boundary still carries these files; they advance with it when present.
    for (const [relative, expected] of [
      ["ait-release-family.json", 9], ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1],
    ]) {
      if (!existsSync(path.join(root, relative))) continue;
      if (relative === "ait-release-family.json") replaceFamilyVersion(path.join(root, relative), priorVersion, version, mutate);
      else replaceExact(path.join(root, relative), priorVersion, version, expected, mutate);
      changed.push(relative);
    }
    return changed.sort();
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
    replaceExact(path.join(root, "ait-release.json"), priorVersion, version, 1, mutate);
    return ["Cargo.lock", "Cargo.toml", "ait-release.json"];
  }
  if (component === "python") {
    const counts = new Map([
      ["pyproject.toml", 1], ["ait-release.json", 7], ["ait-external.toml", 1], ["ait-external.lock", 1],
      ["src/ait_python/__init__.py", 1], ["tests/test_package.py", 3], ["tests/test_runtime.py", 1],
    ]);
    for (const [relative, expected] of counts) replaceExact(path.join(root, relative), priorVersion, version, expected, mutate);
    replacePythonAdapterTestVersions(path.join(root, "tests/test_release_adapter.py"), priorVersion, version, mutate);
    for (const relative of ["ait-external.toml", "ait-external.lock", "tests/test_package.py"]) {
      replaceExact(path.join(root, relative), oldPin, coreSnapshot, 1, mutate);
    }
    return [...counts.keys(), "tests/test_release_adapter.py"].sort();
  }
  const counts = new Map([
    ["package.json", 7], ["ait-release.json", 8], ["ait-external.toml", 1], ["ait-external.lock", 1],
    ["lib/npm-payload-contract.json", 7], ["test/package.test.js", 1],
    ["test/release-adapter.test.js", 9], ["test/runtime.test.js", 2],
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

function bindingCard(version) {
  const ref = `release-${version.replaceAll(".", "-")}/external-binding`;
  return {
    ref,
    relative: `docs/sprints/release_${version.replaceAll(".", "_")}_binding.md`,
    content: `# Stable ${version} Component Binding [plan-ref: ait-native/release/${version}-binding]\n\n## Goal\n\nAdvance the family version authority and pin the five accepted component Snapshots for stable ${version}.\n\n## Scope\n\n- Change only the family template, bootstrap matrix, repository authorities, ait-external.toml, and ait-external.lock.\n- Preserve the family template, distribution policy, and component order.\n\n## Acceptance criteria\n\n- Every direct component external pins its prepared public Snapshot.\n- The normalized diff contains only the external manifest and lock.\n\n## Work item\n\n- [ ] [ref: ${ref}] Pin the stable ${version} component Snapshots.\n`,
  };
}

function acceptedComponentCloseouts(request, state) {
  return COMPONENTS.map((component) => {
    const row = state.components[component];
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
      finish_local_before_ready: true,
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

function requireAdmission(condition, detail) {
  if (!condition) throw Object.assign(new Error(`completed release admission: ${detail}`), { exitCode: 75 });
}

function admissionDocument(file, label) {
  try {
    requireAdmission(lstatSync(file).isFile() && !lstatSync(file).isSymbolicLink(), `${label} is not a regular file`);
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (error) { requireAdmission(false, `${label} is unavailable or invalid: ${error.message}`); }
}

function admittedLocalSnapshot(task, taskId, snapshot) {
  try { return originalLocalSnapshot(task, taskId, snapshot); }
  catch (error) { requireAdmission(false, error.message); }
}

export function readAdmissionJson(ait, cwd, argv, allowMissing = false) {
  const result = spawnSync(ait, argv, { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (result.status !== 0) {
    if (allowMissing && /^Error: Unknown Task: [A-Z]+T-[0-9]+\s*$/.test(result.stderr ?? "")) return null;
    requireAdmission(false, `native admission readback failed: ${argv.join(" ")}: ${result.stderr ?? result.error?.message ?? ""}`);
  }
  try { return JSON.parse(result.stdout); }
  catch { requireAdmission(false, `native admission readback is not JSON: ${argv.join(" ")}`); }
}

export function completedRemoteAdmission(task, taskId, snapshot, repository, repositoryIndex, readPatchset) {
  const land = task?.successful_land;
  requireAdmission(task?.task_id === taskId && task.status === "completed" && task.workflow_state === "completed",
    `${taskId} is not a completed origin Task`);
  requireAdmission(task.repo_name === repository && task.repository_index === repositoryIndex &&
    land?.task_id === taskId && land.repo_name === repository && land.repository_index === repositoryIndex,
  `${taskId} Repository identity differs`);
  requireAdmission(land.status === "succeeded" && land.governance_authority === true && land.target_line === "main" &&
    land.revision_snapshot_id === snapshot && land.landed_snapshot_id === snapshot &&
    land.result?.selected_revision_snapshot_id === snapshot && land.result?.landed_snapshot_id === snapshot &&
    land.result?.target_line === "main", `${taskId} has no exact successful governed finish for ${snapshot}`);
  requireAdmission(TASK_RE.test(taskId), "formal Task identity is invalid");
  const selected = new RegExp(`^${taskId}/(?:(C-[0-9]{2})/)?P-([0-9]{2})$`).exec(land.patchset_id ?? "");
  requireAdmission(selected && Number(selected[2]) > 0 &&
    new RegExp(`^${taskId}/L-[0-9]{2}$`).test(land.submission_id ?? "") &&
    task.current_patchset_id === land.patchset_id, `${taskId} formal acceptance identity differs`);
  const change = selected[1] ?? null, changeRef = change ? `${taskId}/${change}` : null;
  requireAdmission((land.change_id ?? null) === change && (land.change_ref ?? null) === changeRef,
    `${taskId} successful land Change identity differs`);
  // Native public lookup resolves both direct and revised-Change Patchsets.
  // Prove that resolution against the exact formal land; never expose its
  // internal Change reference or infer public ownership from string stripping.
  const reference = `${taskId}/P-${selected[2]}`;
  requireAdmission(typeof readPatchset === "function", `${taskId} public Patchset resolution is unavailable`);
  const patchset = readPatchset(reference);
  requireAdmission(patchset?.task_id === taskId && patchset.patchset_id === land.patchset_id &&
    patchset.patchset_number === Number(selected[2]) && (patchset.change_id ?? null) === change &&
    (patchset.change_ref ?? null) === changeRef && patchset.repo_name === repository &&
    patchset.repo_id === String(repositoryIndex) && patchset.revision_snapshot_id === snapshot,
  `${taskId} public Patchset resolved another formal identity or source`);
  requireAdmission(patchset.governance_authority === true && patchset.invalidated === false &&
    patchset.withdrawn === false && patchset.publish_state === "published" &&
    patchset.evaluation_state === "pass" && patchset.ci?.overall_status === "pass",
  `${taskId} public Patchset is not a passing governed admission`);
  return { task: taskId, snapshot, patchset: reference };
}

function externalDeclarations(root) {
  const parse = (relative, table) => {
    const file = path.join(root, relative), stat = lstatSync(file);
    requireAdmission(stat.isFile() && !stat.isSymbolicLink(), `${relative} is not a regular source file`);
    return readFileSync(file, "utf8").split(new RegExp(`^\\[\\[${table}\\]\\]\\s*$`, "m")).slice(1).map(block => {
      const row = {};
      for (const line of block.split(/^\s*\[/m)[0].split("\n")) {
        const value = /^([a-z_]+)\s*=\s*("(?:[^"\\]|\\.)*"|[0-9]+)\s*$/.exec(line);
        if (!value) continue;
        requireAdmission(!Object.hasOwn(row, value[1]), `duplicate ${relative} key`);
        row[value[1]] = JSON.parse(value[2]);
      }
      return row;
    });
  };
  const manifest = parse("ait-external.toml", "external");
  const locked = parse("ait-external.lock", "node").filter(row => row.parent_path === "");
  requireAdmission(manifest.length === 5 && locked.length === 5, "direct source inventory is not exactly five Repositories");
  const indices = { core: 0, server: 1, runner: 4, python: 2, node: 3 };
  return Object.fromEntries(COMPONENTS.map(component => {
    const name = `ait-${component}`, declared = manifest.filter(row => row.name === name), pins = locked.filter(row => row.name === name);
    requireAdmission(declared.length === 1 && pins.length === 1, `missing or ambiguous ${name} declaration`);
    const row = declared[0];
    for (const key of ["name", "repo_name", "repository_index", "remote", "line", "snapshot", "materialize_to", "license", "version"]) {
      requireAdmission(row[key] === pins[0][key], `${name} manifest/lock differs at ${key}`);
    }
    requireAdmission(row.repo_name === name && row.repository_index === indices[component] && row.remote === "origin" &&
      row.line === "main" && row.materialize_to === `.ait-external/${name}` && SNAPSHOT_RE.test(row.snapshot),
    `${name} source authority differs`);
    requireAdmission(row.license === (component === "server" ? "AGPL-3.0-only" : "Apache-2.0") && stable(row.version),
      `${name} license/version authority differs`);
    return [component, row];
  }));
}

export function admittedSourceRequest(request) {
  return COMPONENTS.every(component => new Set(["finished_local", "snapshot_ready"]).has(request.adopt?.[component]?.stage));
}

export function inspectAdmittedOperation(nativeRoot, sources, read, expectedBinding = null, expectedVersion = null) {
  const head = () => {
    const status = read(nativeRoot, ["status", "--json"]);
    requireAdmission(status.ok === true && status.repo_name === "ait-native" && status.line_name === "main" &&
      status.worktree === null && status.workspace?.dirty === false && status.workspace?.changed_count === 0 &&
      SNAPSHOT_RE.test(status.head_snapshot_id), "Native must be clean canonical main");
    return status.head_snapshot_id;
  };
  const snapshot = head();
  const files = ["ait-external.toml", "ait-external.lock"].map(file => sha256(readFileSync(path.join(nativeRoot, file))));
  const tasks = read(nativeRoot, ["task", "list", "--all", "--json"]);
  requireAdmission(Array.isArray(tasks), "Native Local Task inventory is unavailable");
  const matches = tasks.filter(task => task.status === "completed" && task.local_finish?.status === "succeeded" &&
    task.local_finish.target_line === "main" && task.local_finish.landed_snapshot_id === snapshot);
  requireAdmission(matches.length === 1, "Native head has missing or ambiguous successful Local Task provenance");
  const nativeTask = read(nativeRoot, ["task", "show", matches[0].task_id, "--local", "--json"]);
  admittedLocalSnapshot(nativeTask, matches[0].task_id, snapshot);
  requireAdmission(nativeTask.status === "completed" && nativeTask.repo_name === "ait-native" &&
    nativeTask.local_finish.target_line === "main" &&
    nativeTask.publication_state === "published" && nativeTask.published_remote_name === "origin" &&
    TASK_RE.test(nativeTask.published_task_id ?? ""), "Native has no original published Task mapping");
  completedRemoteAdmission(read(nativeRoot, ["task", "show", nativeTask.published_task_id, "--remote", "origin", "--json"]),
    nativeTask.published_task_id, snapshot, "ait-native", 28,
    reference => read(nativeRoot, ["patchset", "show", reference, "--remote", "origin", "--json"]));
  const binding = { task: nativeTask.task_id, snapshot, stage: "finished_local" };
  if (expectedBinding) requireAdmission(expectedBinding.task === binding.task && expectedBinding.snapshot === snapshot &&
    expectedBinding.stage === binding.stage, "Native head differs from the original prepared binding");
  const declarations = externalDeclarations(nativeRoot);
  const family = admissionDocument(path.join(nativeRoot, "ait-release-family.json"), "admitted family template");
  requireAdmission(stable(family.family?.version) && family.family.tag === `v${family.family.version}` &&
    (expectedVersion === null || family.family.version === expectedVersion) &&
    Object.values(declarations).every(row => row.version === family.family.version), "admitted release version differs");
  const status = read(nativeRoot, ["external", "status", "--json"]);
  requireAdmission(status.command === "external status" && Array.isArray(status.externals) && status.externals.length === 8 &&
    Array.isArray(status.lock_drifts) && status.lock_drifts.length === 0 && status.externals.every(row =>
      row.state === "materialized" && row.dirty === false && row.linked === false && row.lock_drift === false),
  "external source graph is not clean and materialized");
  validateReleaseDependencyGraph(nativeRoot, status);
  const closeouts = COMPONENTS.map(component => {
    const row = sources[component], declaration = declarations[component], name = declaration.repo_name;
    requireAdmission(row && TASK_RE.test(row.task ?? "") && row.snapshot === declaration.snapshot &&
      path.isAbsolute(row.edit_root ?? ""), `${name} requested source differs from the admitted lock`);
    const direct = status.externals.filter(item => item.parent_path === "" && item.name === name);
    // Product versions are proved by manifest/lock/family above. The retained
    // materialization status exposes source identity, not a version property.
    requireAdmission(direct.length === 1 && ["repo_name", "snapshot", "repository_index", "materialize_to", "license"]
      .every(key => direct[0][key] === declaration[key]), `${name} materialized identity differs`);
    const local = read(row.repository_root, ["task", "show", row.task, "--local", "--json"], true);
    requireAdmission(local || row.stage === "snapshot_ready", `${name} finished_local source has no original Local Task`);
    let taskId = row.task;
    if (local) {
      admittedLocalSnapshot(local, row.task, row.snapshot);
      requireAdmission(local.status === "completed" && local.local_finish.target_line === "main" && local.repo_name === name && local.publication_state === "published" &&
        local.published_remote_name === declaration.remote && TASK_RE.test(local.published_task_id ?? ""),
      `${name} original Local source has no published mapping`);
      taskId = local.published_task_id;
    }
    const proof = completedRemoteAdmission(read(row.repository_root,
      ["task", "show", taskId, "--remote", declaration.remote, "--json"]), taskId, row.snapshot, name, declaration.repository_index,
    reference => read(row.repository_root, ["patchset", "show", reference, "--remote", declaration.remote, "--json"]));
    return { id: component, ...proof, repository_root: row.repository_root, edit_root: row.edit_root,
      remote: declaration.remote, finish_local_before_ready: false, review_message: reviewMessage(component) };
  });
  requireAdmission(head() === snapshot && files.every((digest, index) => digest === sha256(readFileSync(path.join(nativeRoot,
    ["ait-external.toml", "ait-external.lock"][index])))), "Native source changed during admission readback");
  return { binding, closeouts, components: Object.fromEntries(COMPONENTS.map(component => [component, declarations[component].snapshot])) };
}

// The existing preparation state and accepted input are frozen by the plan's
// control_files. Re-prove native identities as well: equal source bytes do not
// establish that the same Task, head or component cohort is still selected.
export function inspectPreparedOperation(preparationPath, acceptedPath, read) {
  const preparation = admissionDocument(preparationPath, "completed preparation state");
  const accepted = admissionDocument(acceptedPath, "accepted release input");
  requireAdmission(preparation.contract === "ait.release.preparation-state/v1" && preparation.status === "prepared" &&
    preparation.accepted_input === acceptedPath && preparation.version === accepted.release?.version &&
    preparation.steps?.compile_accepted_input?.status === "pass" &&
    preparation.steps.compile_accepted_input.path === acceptedPath &&
    preparation.steps.compile_accepted_input.sha256 === sha256(readFileSync(acceptedPath)), "prepared accepted input differs");
  const rows = accepted.component_closeouts;
  requireAdmission(Array.isArray(rows) && rows.length === 5 && COMPONENTS.every(id => rows.filter(row => row.id === id).length === 1) &&
    rows.every(row => row.finish_local_before_ready === false && row.remote === "origin"), "completed source inventory differs");
  requireAdmission(admittedSourceRequest({ adopt: preparation.components }), "prepared source stages differ");
  const sources = Object.fromEntries(rows.map(row => [row.id, { ...preparation.components[row.id], repository_root: row.repository_root }]));
  const proof = inspectAdmittedOperation(accepted.paths.native_root, sources, read, preparation.binding, accepted.release.version);
  const selected = Object.fromEntries(COMPONENTS.map(id => [`ait-${id}`, proof.components[id]]));
  const pin = preparation.steps?.pin_externals;
  requireAdmission(pin?.status === "pass" && pin.task === proof.binding.task && pin.snapshot === proof.binding.snapshot &&
    Object.keys(pin.components ?? {}).length === 5 && Object.entries(selected).every(([key, value]) => pin.components[key] === value),
  "prepared Native component binding differs");
  requireAdmission(proof.closeouts.every(row => {
    const recorded = rows.find(item => item.id === row.id);
    return ["task", "snapshot", "patchset", "edit_root", "remote"].every(key => recorded[key] === row[key]);
  }), "prepared source identities differ");
  const web = admissionDocument(accepted.paths.web_components, "Web component map");
  requireAdmission(COMPONENTS.every(id => web[id] === proof.components[id]) && web.web_test === accepted.web.acceptance_snapshot &&
    web.personal === accepted.web.personal_snapshot && web.community === accepted.web.community_snapshot,
  "prepared Web source identities differ");
  return proof;
}

// An external, existing component map selects qualified inputs, not another
// operation's state. The ordinary request/preparation still proves the current
// Native publication. Rebind the original provider runs through their existing
// consumers; never import their conductor checkpoints or completion records.
export function qualifiedOperationInputs(accepted, requireQualificationSource = false) {
  const root = qualificationCarrier(accepted);
  if (!root) return null;
  requireAdmission(accepted.release.version === "1.2.0" && accepted.release.channel === "stable" &&
    !webCommunityPathsInScope(accepted.paths, accepted.release,
      Object.hasOwn(accepted.paths, "personal_bin") || Object.hasOwn(accepted.paths, "personal_root")),
  "qualified input reuse requires the owner-paused stable scope");
  requireAdmission(path.basename(accepted.paths.web_components) === "web-components.json" &&
    !path.resolve(accepted.records_root).startsWith(`${path.resolve(root)}${path.sep}`), "qualified inputs require a distinct fresh operation");
  const files = {};
  const read = name => {
    const file = path.join(root, name), value = admissionDocument(file, `qualified ${name}`);
    files[file] = sha256(readFileSync(file)); return value;
  };
  const candidate = read("candidate.json"), qualification = read("pre-rc-qualification.json");
  const admission = read("pre-tag-admission.json"), prepare = read("prepare.json"), receipts = read("receipts.json");
  const prior = read("accepted-input.json"), components = read("web-components.json"), plan = read("conductor-plan.json");
  const selectedComponents = admissionDocument(accepted.paths.web_components, "selected Web components");
  if (components.web_test !== accepted.web.acceptance_snapshot) {
    const historical = priorWebInput(accepted.paths.native_root, accepted.release);
    requireAdmission(historical?.root === root, "WT correction selected another qualification carrier");
    sameProductComponents(selectedComponents, historical.input);
    correctedWTIdentity(accepted, historical.input, selectedComponents);
  } else requireAdmission(isDeepStrictEqual(components, selectedComponents), "selected product components differ");
  requireAdmission(candidate.contract === "ait.release.operator.pre-tag-candidate-binding/v1" && candidate.status === "ready_for_immutable_tag" &&
    qualification.contract === "ait.release.operator.qualification-binding/v1" && qualification.status === "qualified_for_version_only_release" &&
    admission.contract === "ait.release.operator.pre-tag-admission/v1" && admission.status === "ready_for_component_receipts" &&
    prepare.contract === "ait.release.operator.prepare/v3" && prepare.status === "ready_for_component_receipts" &&
    receipts.contract === "ait.release.operator.receipt-binding/v2" && receipts.status === "ready_for_pre_tag_qualification",
  "qualified evidence inventory is incomplete");
  const same = isDeepStrictEqual;
  const authority = candidate.candidate_authority;
  requireAdmission(same(candidate.candidate_authority, receipts.candidate_authority) && same(candidate.source_dossier, receipts.source_dossier) &&
    same(candidate.release, { ...receipts.release, prior_version: accepted.release.prior_version }) &&
    candidate.release.version === accepted.release.version && candidate.release.channel === accepted.release.channel &&
    authority?.record?.prior_release?.version === accepted.release.prior_version &&
    same(prepare.pre_rc_admission, receipts.pre_rc_admission) &&
    receipts.pre_rc_admission.admission_sha256 === files[path.join(root, "pre-tag-admission.json")] &&
    admission.qualification.binding_sha256 === files[path.join(root, "pre-rc-qualification.json")] &&
    qualification.workflow.run_id === receipts.pre_rc_admission.qualification_workflow_run_id &&
    qualification.source.git_commit === receipts.pre_rc_admission.qualified_source_commit &&
    qualification.source.git_commit === admission.qualification.source_commit &&
    admission.release.source_commit === candidate.release.source_commit &&
    candidate.qualification.workflow_control_commit === candidate.release.source_commit &&
    candidate.qualification.clean_host_rows === 24 && candidate.qualification.tag_state_at_closeout === "absent" &&
    same(prior.release, accepted.release) && same(prior.component_closeouts, accepted.component_closeouts) &&
    ["native_root", "core_root", "public_source", "web_test_root"].every(key => prior.paths[key] === accepted.paths[key]) &&
    selectedComponents.web_test === accepted.web.acceptance_snapshot && !Object.hasOwn(components, "personal"),
  "qualified candidate, source or authority identity differs");
  for (const document of [candidate, qualification, admission, prepare, receipts]) {
    requireAdmission(document.mutation && Object.values(document.mutation).every(value => value === false), "qualified evidence records a mutation");
  }
  for (const value of [qualification.workflow.run_id, receipts.source_dossier.workflow_run_id, candidate.qualification.workflow_run_id]) {
    requireAdmission(Number.isSafeInteger(value) && value > 0, "qualified provider run identity is missing");
  }
  const git = args => {
    const result = spawnSync("git", ["-C", accepted.paths.public_source, ...args], { encoding: "utf8" });
    requireAdmission(result.status === 0, "qualified public source readback failed"); return result.stdout.trim();
  };
  requireAdmission(git(["rev-parse", "HEAD"]) === candidate.release.source_commit &&
    git(["rev-parse", "HEAD^"]) === qualification.source.git_commit && git(["status", "--porcelain", "--untracked-files=all"]) === "",
  "qualified public source changed");
  const publicPaths = new Set(git(["ls-tree", "-r", "--name-only", candidate.release.source_commit]).split("\n"));
  requireAdmission(plan.contract === "ait.release.conductor-plan/v1" && plan.records_root === root &&
    plan.release.version === accepted.release.version && plan.source?.control_files && Object.keys(plan.source.control_files).length > 0,
  "qualified source control binding is missing");
  for (const [file, digest] of Object.entries(plan.source.control_files)) {
    if (!file.startsWith(`${accepted.paths.native_root}${path.sep}`)) continue;
    const relative = path.relative(accepted.paths.native_root, file).split(path.sep).join("/");
    requireAdmission(existsSync(file) && (sha256(readFileSync(file)) === digest ||
      (!publicPaths.has(relative) && relative.startsWith("ci/release_"))), `qualified public control changed: ${relative}`);
  }
  const final = read("public-final/receipt.json");
  requireAdmission(final.contract === "ait.release.public-source/v1" && final.status === "complete" &&
    final.kind === "final" && final.head_sha === candidate.release.source_commit, "qualified public source receipt differs");
  const finalHead = path.join(root, "public-final/head.txt");
  requireAdmission(readFileSync(finalHead, "utf8").trim() === candidate.release.source_commit, "qualified final source head differs");
  files[finalHead] = sha256(readFileSync(finalHead));
  const qualifiedHead = path.join(root, "public-qualification/head.txt");
  requireAdmission(readFileSync(qualifiedHead, "utf8").trim() === qualification.source.git_commit, "qualified source head differs");
  files[qualifiedHead] = sha256(readFileSync(qualifiedHead));
  const qualified = { root, candidate, qualification, receipts, files,
    qualificationRoot: path.join(accepted.records_root, "qualified-public-source") };
  if (requireQualificationSource) {
    verifyQualifiedSource(qualified.qualificationRoot, qualified);
    const tracked = spawnSync("git", ["-C", qualified.qualificationRoot, "ls-files", "-z"], { encoding: "utf8" });
    requireAdmission(tracked.status === 0, "qualified source inventory is unavailable");
    for (const relative of tracked.stdout.split("\0").filter(Boolean)) {
      const file = regularFile(path.join(qualified.qualificationRoot, relative), "qualified source file");
      files[file] = sha256(readFileSync(file));
    }
  }
  return qualified;
}

function verifyQualifiedSource(directory, qualified) {
  const git = args => spawnSync("git", ["-C", directory, ...args], { encoding: "utf8" });
  requireAdmission(existsSync(directory) && !lstatSync(directory).isSymbolicLink(), "qualified source checkout is missing");
  const head = git(["rev-parse", "HEAD"]), status = git(["status", "--porcelain", "--untracked-files=all"]);
  requireAdmission(head.status === 0 && head.stdout.trim() === qualified.qualification.source.git_commit &&
    status.status === 0 && status.stdout.trim() === "", "qualified source checkout changed");
}

export function materializeQualifiedSource(accepted, qualified) {
  return materializeDirectory(qualified.qualificationRoot, directory => {
    for (const args of [["clone", "--quiet", "--no-hardlinks", "--no-checkout", "--", accepted.paths.public_source, directory],
      ["-C", directory, "checkout", "--quiet", "--detach", qualified.qualification.source.git_commit]]) {
      const result = spawnSync("git", args, { encoding: "utf8" });
      requireAdmission(result.status === 0, `qualified source materialization failed: ${result.stderr}`);
    }
  }, directory => verifyQualifiedSource(directory, qualified));
}

export function qualifiedConductorPhases(phases, accepted, qualified) {
  if (!qualified) return phases;
  const runIds = { public_qualification: qualified.qualification.workflow.run_id,
    component_receipts: qualified.receipts.source_dossier.workflow_run_id,
    clean_host_qualification: qualified.candidate.qualification.workflow_run_id };
  return phases.map(phase => ({ ...phase, actions: phase.actions
    .filter(action => !(phase.id === "public_qualification" && action.id === "publish-qualification-source"))
    .map(action => {
      if (action.type === "github_workflow" && runIds[phase.id]) return {
        id: action.id, type: "command", mutation: false, cwd: action.bind.cwd,
        argv: action.bind.argv.map((arg, index, argv) => phase.id === "public_qualification" && argv[index - 1] === "--source-root"
          ? qualified.qualificationRoot : arg.replaceAll("{run_id}", String(runIds[phase.id]))), outputs: action.bind.outputs,
      };
      if (action.id === "publish-final-source") {
        const argv = action.probe.argv.map(arg => arg === path.join(accepted.records_root, "public-qualification/head.txt")
          ? path.join(qualified.root, "public-qualification/head.txt") : arg);
        return { id: action.id, type: "command", mutation: false, cwd: action.probe.cwd, argv,
          adopt: { cwd: action.probe.cwd, argv }, outputs: action.outputs };
      }
      return action;
    }) }));
}

function reviewMessage(component) {
  const cap = component[0].toUpperCase() + component.slice(1);
  return `Reviewed files: ${cap} stable release authority paths await independent review. Findings: Reviewer decision required. Risks: Reviewer assessment required. Tests: Consult the native preparation results. Recommendation: Await the assigned Reviewer.`;
}

async function main() {
  const { mode, requestPath } = parseCli(process.argv.slice(2));
  await enterOperation(readJson(requestPath, "release request").records_root);
  const { request, requestHash } = validateRequest(requestPath);
  const searchPath = releasePath();
  const requiredCommands = ["bash", "cargo", "diff", "gh", "git", "jq", "node", "npm", "python3", "tar"];
  const tools = Object.fromEntries(requiredCommands.map((name) => [name, findCommand(name, searchPath)]));
  for (const [name, value] of Object.entries(tools)) if (!value) fail(`required release command is unavailable: ${name}`, 69);
  const ait = executable(process.env.AIT_RELEASE_AIT_BIN
    ? (path.isAbsolute(process.env.AIT_RELEASE_AIT_BIN) ? process.env.AIT_RELEASE_AIT_BIN : findCommand(process.env.AIT_RELEASE_AIT_BIN, searchPath))
    : path.join(request.roots.core, ".ait/cargo-target/release/ait-cli"), "release AIT CLI");
  const fullyAdopted = admittedSourceRequest(request);
  const sources = Object.fromEntries(COMPONENTS.map(id => [id, { ...request.adopt?.[id], repository_root: request.roots[id] }]));
  const readAdmission = (cwd, argv, allowMissing) => readAdmissionJson(ait, cwd, argv, allowMissing);
  for (const field of fullyAdopted ? [] : ["stop_argv", "start_argv"]) {
    const command = request.server_refresh[field][0];
    if (!path.isAbsolute(command)) fail(`server_refresh.${field}[0] must be an absolute executable`, 64);
    executable(command, `server_refresh.${field}[0]`);
  }
  const auth = spawnSync(tools.gh, ["auth", "status"], { env: { ...process.env, PATH: searchPath }, stdio: "ignore" });
  if (auth.status !== 0) fail("GitHub CLI authentication is unavailable before release preparation", 69);
  const remoteUrl = await authorityPreflight(request.roots.core);
  const inventoryPin = "SNP-000000000000";
  for (const component of fullyAdopted ? [] : [...COMPONENTS, "native"]) {
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
    "preflight", "validate_inventories", "prepare_core", "prepare_server",
    "prepare_runner", "prepare_python", "prepare_node", "publish_component_sources", "pin_externals", "preflight_component_closeouts", "compile_accepted_input", "run_conductor",
  ];
  if (mode === "plan") {
    if (fullyAdopted) inspectAdmittedOperation(request.roots.native, sources, readAdmission, null, request.release.version);
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

  const taskPayload = (component, row) => {
    const argv = [ait, "task", "show", row.task];
    argv.push("--local");
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
      args.push("--local");
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
            const externalArgs = component === "runner"
              ? [ait, "external", "update", "--locked", "--validate", "--no-recursive", "--json"]
              : [ait, "external", "update", "ait-core", "--to", coreSnapshot, "--validate", "--no-recursive", "--json"];
            const external = runJson(`external-${component}`, row.edit_root, externalArgs);
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
      {
        if (currentTask.status === "completed") {
          row.snapshot = originalLocalSnapshot(currentTask, row.task, row.snapshot ?? null);
        } else if (currentTask.status === "active") {
          fail(`${component}: independent Reviewer must finish the original Local source Task ${row.task}; retain ${row.edit_root} and resume this preparation afterward`, 75);
        } else {
          fail(`${component} Task cannot resume from status ${currentTask.status}`);
        }
        row.stage = "finished_local";
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

  // Every accepted component Snapshot must be reachable from the server
  // before ait-native pins it: the binding fetches Snapshots by identity, and
  // remote closeout happens later in the conductor.
  const publishComponentSource = (component, row) => {
    const step = `publish_${component}_source`;
    if (done(step)) return;
    const root = request.roots[component];
    const line = `release/${request.release.version}-${component}-source`;
    const listed = runJson(`${component}-line-list`, root, [ait, "line", "list", "--json"]);
    const rows = Array.isArray(listed) ? listed : listed.lines ?? [];
    const existing = rows.find((entry) => entry.line_name === line || entry.name === line);
    if (existing) {
      const head = existing.head_snapshot_id ?? existing.snapshot_id;
      if (head !== row.snapshot) fail(`existing ${component} release Line points at another Snapshot`);
    } else {
      runJson(`${component}-line-create`, root, [ait, "line", "create", line, "--from-snapshot", row.snapshot, "--json"]);
    }
    runJson(`${component}-line-push`, root, [ait, "push", "--remote", "origin", "--line", line, "--json"]);
    complete(step, { line, snapshot: row.snapshot });
  };

  let admittedCloseouts;
  if (fullyAdopted) {
    if (done("compile_accepted_input")) inspectPreparedOperation(statePath, state.accepted_input, readAdmission);
    const proof = inspectAdmittedOperation(request.roots.native, sources, readAdmission, state.binding ?? null, request.release.version);
    const selected = Object.fromEntries(COMPONENTS.map(id => [`ait-${id}`, proof.components[id]]));
    if (done("pin_externals")) requireAdmission(state.steps.pin_externals.task === proof.binding.task &&
      state.steps.pin_externals.snapshot === proof.binding.snapshot &&
      JSON.stringify(state.steps.pin_externals.components) === JSON.stringify(selected), "saved component pin differs");
    state.binding = proof.binding;
    for (const component of COMPONENTS) {
      const row = { ...request.adopt[component] };
      if (state.components[component]) requireAdmission(JSON.stringify(state.components[component]) === JSON.stringify(row), "saved adopted source differs");
      state.components[component] = row;
      complete(`prepare_${component}`, { task: row.task, snapshot: row.snapshot, stage: row.stage });
    }
    complete("pin_externals", { task: proof.binding.task, snapshot: proof.binding.snapshot, components: selected });
    admittedCloseouts = proof.closeouts;
    complete("preflight_component_closeouts", { tasks: proof.closeouts.map(({ id, task }) => ({ id, task })) });
  } else {
    const core = prepareComponent("core");
    if (!done("refresh_core_artifacts")) {
      refreshCoreArtifacts("core");
      complete("refresh_core_artifacts");
    }
    publishComponentSource("core", core);

    const server = prepareComponent("server");
    publishComponentSource("server", server);
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
    publishComponentSource("runner", runner);
    const python = prepareComponent("python", core.snapshot);
    publishComponentSource("python", python);
    const node = prepareComponent("node", core.snapshot);
    publishComponentSource("node", node);

    if (!done("pin_externals")) {
      const selected = { "ait-core": core.snapshot, "ait-server": server.snapshot, "ait-runner": runner.snapshot, "ait-python": python.snapshot, "ait-node": node.snapshot };
      for (const [repository, snapshot] of Object.entries(selected)) {
        if (!SNAPSHOT_RE.test(snapshot ?? "")) fail(`external binding has no accepted Snapshot for ${repository}`);
      }
      let binding = state.binding;
      if (!binding) {
        const generated = bindingCard(request.release.version);
        writeExact(path.join(request.roots.native, generated.relative), generated.content);
        const started = runJson("start-binding", request.roots.native, [ait, "task", "start", "--from", `${generated.relative}#${generated.ref}`, "--intent", `Pin stable ${request.release.version} component Snapshots`, "--local", "--json"]);
        if (!TASK_RE.test(started.task_id ?? "") || typeof started.edit_root !== "string" || !path.isAbsolute(started.edit_root)) {
          fail("external binding start returned no public Task and edit root");
        }
        binding = { task: started.task_id, edit_root: started.edit_root, stage: "active", progress: "started" };
        state.binding = binding;
        save();
      }
      if (binding.stage === "active" && binding.progress !== "finalizing") {
        if (!existsSync(binding.edit_root) || !lstatSync(binding.edit_root).isDirectory()) fail("external binding edit root is unavailable", 66);
        const nativePaths = replaceComponentSources(binding.edit_root, "native", request.release.prior_version, request.release.version, null);
        const template = readJson(path.join(binding.edit_root, "ait-release-family.json"), "family template");
        ensureVersion(template, request.release.version, "family template");
        if (!Array.isArray(template.components) || template.components.some((component) => component.source_snapshot !== "unbound")) {
          fail("family template must stay unbound; ait-external.lock is the binding");
        }
        for (const [repository, snapshot] of Object.entries(selected)) {
          run(`pin-${repository}`, binding.edit_root, [ait, "external", "update", repository, "--to", snapshot, "--no-recursive", "--json"]);
        }
        const status = runJson("external-status", binding.edit_root, [ait, "external", "status", "--json"]);
        const direct = (status.externals ?? []).filter((row) => row.parent_path === "");
        for (const [repository, snapshot] of Object.entries(selected)) {
          const row = direct.find((entry) => entry.name === repository);
          if (!row || row.snapshot !== snapshot || row.lock_drift !== false) fail(`external pin readback differs: ${repository}`);
        }
        const diff = runJson("diff-binding", binding.edit_root, [ait, "diff", "--json"]);
        const changed = [...(diff.changed_paths ?? [])].sort();
        const allowed = new Set(["ait-external.lock", "ait-external.toml", ...nativePaths]);
        if (changed.some((relative) => !allowed.has(relative))) {
          fail("external binding changed unexpected paths");
        }
        binding.progress = "validated";
        save();
        binding.progress = "finalizing";
        save();
      }
      if (binding.stage === "active") {
        const task = runJson("verify-task-binding", request.roots.native, [ait, "task", "show", binding.task, "--local", "--json"]);
        if (task.status === "completed") {
          binding.snapshot = originalLocalSnapshot(task, binding.task, binding.snapshot ?? null);
        } else if (task.status === "active") {
          fail(`binding: independent Reviewer must finish the original Local source Task ${binding.task}; retain ${binding.edit_root} and resume this preparation afterward`, 75);
        } else {
          fail(`external binding Task cannot resume from status ${task.status}`);
        }
        binding.stage = "finished_local";
        delete binding.progress;
        if (!SNAPSHOT_RE.test(binding.snapshot ?? "")) fail("external binding finish did not return a Snapshot");
        save();
      }
      run("materialize-externals", request.roots.native, [ait, "external", "update", "--locked", "--no-recursive", "--json"]);
      complete("pin_externals", { task: binding.task, snapshot: binding.snapshot, components: selected });
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
  }

  if (done("compile_accepted_input")) qualifiedOperationInputs(admissionDocument(state.accepted_input, "accepted release input"), true);

  if (!done("compile_accepted_input")) {
    let webComponents = request.paths.web_components ?? path.join(request.records_root, "web-components.json");
    const webSources = { ...Object.fromEntries(COMPONENTS.map(component => [component, state.components[component].snapshot])),
      personal: request.web.personal_snapshot, community: request.web.community_snapshot, web_test: request.web.acceptance_snapshot };
    if (request.paths.web_components && !isDeepStrictEqual(admissionDocument(webComponents, "qualified Web components"), JSON.parse(JSON.stringify(webSources)))) {
      const prior = priorWebInput(request.roots.native, request.release);
      requireAdmission(prior && path.dirname(webComponents) === prior.root, "qualified Web component Snapshots differ");
      correctedWTIdentity({ release: request.release, web: request.web,
        paths: { native_root: request.roots.native, web_test_root: request.paths.web_test_root, web_components: webComponents } }, prior.input,
      JSON.parse(JSON.stringify(webSources)));
      webComponents = path.join(request.records_root, "web-components.json");
      writeOrVerify(webComponents, `${JSON.stringify(webSources, null, 2)}\n`, "derived corrected Web component Snapshots");
    } else if (!request.paths.web_components) writeOrVerify(webComponents, `${JSON.stringify(webSources, null, 2)}\n`, "derived Web component Snapshots");
    const closeouts = admittedCloseouts ?? acceptedComponentCloseouts(request, state);
    const accepted = {
      contract: "ait.release.accepted-input/v1",
      release: request.release,
      records_root: request.records_root,
      github_repository: request.github_repository,
      winget_fork: request.winget_fork,
      paths: {
        core_root: request.roots.core,
        native_root: request.roots.native,
        public_source: request.paths.public_source,
        web_components: webComponents,
        web_test_root: request.paths.web_test_root,
        ...(request.paths.personal_root ? { personal_root: request.paths.personal_root, personal_bin: request.paths.personal_bin } : {}),
        ...(request.paths.community_bin ? { community_root: request.paths.community_root, community_bin: request.paths.community_bin,
          community_cli_bin: request.paths.community_cli_bin, chrome: request.paths.chrome } : {}),
      },
      web: request.web,
      component_closeouts: closeouts,
    };
    const acceptedPath = path.join(request.records_root, "accepted-input.json");
    const qualified = qualifiedOperationInputs(accepted);
    if (qualified) {
      requireAdmission(fullyAdopted, "qualified inputs require completed source admission");
      materializeQualifiedSource(accepted, qualified);
      mkdirSync(path.join(request.records_root, "public-final"), { recursive: true, mode: 0o700 });
      for (const name of ["receipt.json", "head.txt"]) copyOrVerify(path.join(qualified.root, "public-final", name),
        path.join(request.records_root, "public-final", name));
    }
    writeJsonAtomic(acceptedPath, accepted);
    state.accepted_input = acceptedPath;
    complete("compile_accepted_input", { path: acceptedPath, sha256: sha256(readFileSync(acceptedPath)) });
  }
  state.status = "prepared";
  save();
  process.stdout.write(`${state.accepted_input}\n`);
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => fail(error?.message ?? String(error), error?.exitCode ?? 65));
}

export {
  acceptedComponentCloseouts,
  closeoutReadinessProbes,
  bindingCard,
  copyOrVerify,
  materializeDirectory,
  replaceComponentSources,
  stableAdvance,
  writeOrVerify,
};
