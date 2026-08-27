#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";

const MATRIX_CONTRACT = "ait.release.clean-host.matrix/v1";
const PHASE_CONTRACT = "ait.release.clean-host.phase/v1";
const ROW_CONTRACT = "ait.release.clean-host.row/v1";
const AGGREGATE_CONTRACT = "ait.release.clean-host.aggregate/v1";

const PLATFORM_AUTHORITY = [
  {
    target: "aarch64-apple-darwin",
    runner: "macos-15",
    os: "macos",
    architecture: "arm64",
    executable_suffix: "",
  },
  {
    target: "x86_64-apple-darwin",
    runner: "macos-15-intel",
    os: "macos",
    architecture: "x86_64",
    executable_suffix: "",
  },
  {
    target: "aarch64-unknown-linux-gnu",
    runner: "ubuntu-22.04-arm",
    os: "linux",
    architecture: "arm64",
    executable_suffix: "",
  },
  {
    target: "x86_64-unknown-linux-gnu",
    runner: "ubuntu-22.04",
    os: "linux",
    architecture: "x86_64",
    executable_suffix: "",
  },
  {
    target: "aarch64-pc-windows-msvc",
    runner: "windows-11-arm",
    os: "windows",
    architecture: "arm64",
    executable_suffix: ".exe",
  },
  {
    target: "x86_64-pc-windows-msvc",
    runner: "windows-2025",
    os: "windows",
    architecture: "x86_64",
    executable_suffix: ".exe",
  },
];

const LEGACY_DISTRIBUTION_AUTHORITY = [
  {
    channel: "github",
    role: "product",
    identity: "weita2026/ait-native",
    components: [
      "ait",
      "ait-agent",
      "ait-agent-worker",
      "ait-server",
      "ait-runner",
      "ait-python",
      "ait-node",
    ],
    targetOs: ["macos", "linux", "windows"],
    lifecycle: "product",
    binding: null,
    service: "foreground",
  },
  {
    channel: "pypi",
    role: "product",
    identity: "ait-native",
    components: ["ait", "ait-server", "ait-python"],
    targetOs: ["macos", "linux", "windows"],
    lifecycle: "product",
    binding: "python",
    service: "foreground",
  },
  {
    channel: "npm",
    role: "product",
    identity: "@wa120/ait-native",
    components: ["ait-node"],
    targetOs: ["macos", "linux", "windows"],
    lifecycle: "product",
    binding: "node",
    service: null,
  },
  {
    channel: "homebrew",
    role: "product",
    identity: "ait-native",
    components: ["ait", "ait-server"],
    targetOs: ["macos", "linux"],
    lifecycle: "product",
    binding: null,
    service: "brew-services",
  },
  {
    channel: "apt",
    role: "product",
    identity: "ait-native",
    components: ["ait", "ait-server"],
    targetOs: ["linux"],
    lifecycle: "product",
    binding: null,
    service: "systemd",
  },
  {
    channel: "apt",
    role: "standalone",
    identity: "ait-runner",
    components: ["ait-runner"],
    targetOs: ["linux"],
    lifecycle: "standalone-runner",
    binding: null,
    service: null,
  },
  {
    channel: "winget",
    role: "product",
    identity: "Weita.AitNative",
    components: ["ait", "ait-server"],
    targetOs: ["windows"],
    lifecycle: "product",
    binding: null,
    service: "user-session-controller",
  },
  {
    channel: "oci",
    role: "standalone",
    identity: "ghcr.io/weita2026/ait-server",
    components: ["ait-server"],
    targetOs: ["linux"],
    lifecycle: "standalone-server",
    binding: null,
    service: "container",
  },
  {
    channel: "oci",
    role: "standalone",
    identity: "ghcr.io/weita2026/ait-runner",
    components: ["ait-runner"],
    targetOs: ["linux"],
    lifecycle: "standalone-runner",
    binding: null,
    service: "container",
  },
];

const RUNNER_BUNDLE_DISTRIBUTION_AUTHORITY = LEGACY_DISTRIBUTION_AUTHORITY.map((row) => {
  if (
    row.role === "product" &&
    ["homebrew", "apt", "winget"].includes(row.channel)
  ) {
    return { ...row, components: ["ait", "ait-server", "ait-runner"] };
  }
  return row;
});

const MATRIX_AUTHORITIES = {
  "distribution-target-32-2026-08-17.2": {
    distributions: LEGACY_DISTRIBUTION_AUTHORITY,
    rowCount: 32,
  },
  "distribution-target-runner-bundle-32-2026-08-26.1": {
    distributions: RUNNER_BUNDLE_DISTRIBUTION_AUTHORITY,
    rowCount: 32,
  },
};

function usage(message) {
  if (message) {
    process.stderr.write(`${message}\n`);
  }
  process.stderr.write(
    "usage:\n" +
      "  release_clean_host.mjs matrix --family <json> --platforms <json> --output <absolute-json>\n" +
      "  release_clean_host.mjs combine --matrix <json> --config <json> --status <json> --install <json> --upgrade <json> --output <absolute-json>\n" +
      "  release_clean_host.mjs aggregate --matrix <json> --config <json> --status <json> --evidence-root <absolute-dir> --output-root <absolute-dir>\n",
  );
  process.exit(64);
}

function parseOptions(argv, allowed) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined) {
      usage("clean-host option requires a value");
    }
    const name = key.slice(2);
    if (!allowed.includes(name) || Object.hasOwn(options, name)) {
      usage(`unsupported or repeated clean-host option: ${key}`);
    }
    options[name] = value;
  }
  for (const name of allowed) {
    if (!Object.hasOwn(options, name)) {
      usage(`missing required clean-host option: --${name}`);
    }
  }
  return options;
}

function fail(message, code = 65) {
  const error = new Error(message);
  error.exitCode = code;
  throw error;
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

function requireRealDirectory(directory, label) {
  if (!path.isAbsolute(directory)) {
    fail(`${label} must use an absolute path`, 64);
  }
  let stat;
  try {
    stat = lstatSync(directory);
  } catch {
    fail(`${label} is unavailable: ${directory}`, 66);
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail(`${label} must be a real non-symlink directory: ${directory}`, 66);
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

function requireNewFile(file, label) {
  if (!path.isAbsolute(file)) {
    fail(`${label} must use an absolute path`, 64);
  }
  try {
    lstatSync(file);
    fail(`${label} already exists: ${file}`, 73);
  } catch (error) {
    if (error.exitCode) {
      throw error;
    }
    if (error.code !== "ENOENT") {
      throw error;
    }
  }
  requireRealDirectory(path.dirname(file), `${label} parent`);
}

function requireNewDirectory(directory, label) {
  if (!path.isAbsolute(directory)) {
    fail(`${label} must use an absolute path`, 64);
  }
  try {
    lstatSync(directory);
    fail(`${label} already exists: ${directory}`, 73);
  } catch (error) {
    if (error.exitCode) {
      throw error;
    }
    if (error.code !== "ENOENT") {
      throw error;
    }
  }
  requireRealDirectory(path.dirname(directory), `${label} parent`);
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

function writeNewJson(file, value, label) {
  requireNewFile(file, label);
  writeFileSync(file, encoded(value), { encoding: "utf8", mode: 0o644 });
}

function sha256Bytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  requireRegularFile(file, "checksum input");
  return sha256Bytes(readFileSync(file));
}

function sameJson(left, right) {
  return JSON.stringify(sortedValue(left)) === JSON.stringify(sortedValue(right));
}

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (!sameJson(actual, wanted)) {
    fail(`${label} fields are not exact`);
  }
}

function validatePlatformManifest(platforms, version) {
  if (
    platforms.contract !== "ait-native-bootstrap-matrix/v1" ||
    platforms.schema_version !== 1 ||
    platforms.matrix_revision !== "six-target-2026-07-19.1" ||
    platforms.version !== version ||
    platforms.public_identity !== "ait" ||
    platforms.public_publish !== false ||
    !Array.isArray(platforms.targets)
  ) {
    fail("native platform manifest is not the frozen clean-host authority");
  }
  const projected = platforms.targets.map((row) => ({
    target: row.target,
    runner: row.runner,
    os: row.os,
    architecture: row.architecture,
    executable_suffix: row.executable_suffix,
  }));
  if (!sameJson(projected, PLATFORM_AUTHORITY)) {
    fail("native platform runner labels or target mapping drifted");
  }
}

function validateFamily(family) {
  if (
    family.schema !== "ait.release.family/v3" ||
    family.family?.name !== "ait-native" ||
    !["rc", "stable"].includes(family.family?.channel) ||
    family.family?.tag !== `v${family.family?.version}` ||
    family.public_source?.identity !== "weita2026/ait-native" ||
    !Array.isArray(family.distributions)
  ) {
    fail("release family is not a canonical ait-native v3 family");
  }
  const projected = family.distributions.map((row) => ({
    channel: row.channel,
    role: row.role,
    identity: row.identity,
    components: row.components,
    targets: row.targets,
  }));
  const matches = Object.entries(MATRIX_AUTHORITIES).filter(([, contract]) => {
    const expected = contract.distributions.map((authority) => ({
      channel: authority.channel,
      role: authority.role,
      identity: authority.identity,
      components: authority.components,
      targets: PLATFORM_AUTHORITY.filter((platform) =>
        authority.targetOs.includes(platform.os),
      ).map((platform) => platform.target),
    }));
    return sameJson(projected, expected);
  });
  if (matches.length !== 1) {
    fail("release family distributions differ from the exact clean-host inventory");
  }
  const [matrixRevision, contract] = matches[0];
  return {
    version: family.family.version,
    matrixRevision,
    distributions: contract.distributions,
    rowCount: contract.rowCount,
  };
}

function rowIdentity(authority, platform) {
  const component =
    authority.lifecycle === "standalone-server"
      ? "ait-server"
      : authority.lifecycle === "standalone-runner"
        ? "ait-runner"
        : null;
  const suffix = component ? `-${component}` : "";
  return `${authority.channel}-${authority.role}${suffix}-${platform.target}`;
}

function requiredChecks(row, phase) {
  const checks = [
    "runner_target",
    "package_manager",
    phase === "install" ? "candidate_install" : "prior_exact_install",
    "installed_origin",
    "command_version",
  ];
  if (row.lifecycle === "product") {
    if (phase === "install") {
      checks.push(
        "first_land",
        "generated_workflow_current",
        "uninstall",
        "user_data_retained",
      );
    } else {
      checks.push(
        "prior_baseline",
        "candidate_upgrade",
        "candidate_land",
        "generated_workflow_current",
      );
    }
  } else if (phase === "install") {
    checks.push("uninstall");
  } else {
    checks.push("candidate_upgrade");
  }
  if (row.binding === "python") {
    checks.push("python_direct_binding");
  }
  if (row.binding === "node") {
    checks.push("node_direct_addon", "node_in_process_command");
  }
  if (row.service !== null) {
    checks.push("component_inactive_default", "explicit_component_lifecycle");
  }
  if (row.channel === "oci") {
    checks.push("immutable_image_digest");
  }
  return checks.sort();
}

function buildMatrix(family, platforms) {
  const familyContract = validateFamily(family);
  const { version } = familyContract;
  validatePlatformManifest(platforms, version);
  const rows = [];
  for (const authority of familyContract.distributions) {
    for (const platform of PLATFORM_AUTHORITY.filter((candidate) =>
      authority.targetOs.includes(candidate.os),
    )) {
      rows.push({
        id: rowIdentity(authority, platform),
        channel: authority.channel,
        role: authority.role,
        identity: authority.identity,
        components: authority.components,
        target: platform.target,
        runner: platform.runner,
        os: platform.os,
        architecture: platform.architecture,
        executable_suffix: platform.executable_suffix,
        lifecycle: authority.lifecycle,
        binding: authority.binding,
        service: authority.service,
        required_checks: {
          install: requiredChecks(
            { ...authority, channel: authority.channel },
            "install",
          ),
          upgrade: requiredChecks(
            { ...authority, channel: authority.channel },
            "upgrade",
          ),
        },
      });
    }
  }
  const ids = rows.map((row) => row.id);
  if (
    rows.length !== familyContract.rowCount ||
    new Set(ids).size !== familyContract.rowCount
  ) {
    fail("clean-host row inventory does not match its exact unique-row contract");
  }
  const counts = Object.fromEntries(
    [...new Set(rows.map((row) => `${row.channel}:${row.role}`))].map((key) => [
      key,
      rows.filter((row) => `${row.channel}:${row.role}` === key).length,
    ]),
  );
  const expectedCounts = Object.fromEntries(
    [
      ...new Set(
        familyContract.distributions.map(
          (authority) => `${authority.channel}:${authority.role}`,
        ),
      ),
    ].map((key) => [
      key,
      familyContract.distributions
        .filter((authority) => `${authority.channel}:${authority.role}` === key)
        .reduce(
          (count, authority) =>
            count +
            PLATFORM_AUTHORITY.filter((platform) =>
              authority.targetOs.includes(platform.os),
            ).length,
          0,
        ),
    ]),
  );
  if (!sameJson(counts, expectedCounts)) {
    fail("clean-host distribution counts drifted");
  }
  return {
    contract: MATRIX_CONTRACT,
    schema_version: 1,
    matrix_revision: familyContract.matrixRevision,
    release: {
      version,
      channel: family.family.channel,
      tag: family.family.tag,
    },
    runner_authority: "ci/native_bootstrap_matrix.json",
    family_authority: "ait-release-family.json",
    row_count: rows.length,
    counts,
    rows,
  };
}

function validateConfigAndStatus(config, status) {
  if (
    config.contract !== "ait.release.family.endpoints/v1" ||
    !/^REL-FAM-[0-9A-F]{16}$/.test(config.release?.id ?? "") ||
    !/^[0-9a-f]{40}$/.test(config.release?.source_commit ?? "")
  ) {
    fail("endpoint configuration is not canonical");
  }
  const common =
    status.release?.id === config.release.id &&
    status.release?.version === config.release.version &&
    status.release?.tag === config.release.tag;
  const prepublish =
    status.contract === "ait.release.prepublish.candidate/v1" &&
    status.status === "frozen_candidate_pending_clean_host" &&
    status.release?.source_commit === config.release.source_commit &&
    /^[0-9a-f]{64}$/.test(status.candidate?.stage_receipt_sha256 ?? "");
  if (!common || !prepublish) {
    fail("status is not the exact frozen prepublish candidate");
  }
  return "prepublication";
}

function validateMatrix(matrix, config) {
  const contract = MATRIX_AUTHORITIES[matrix.matrix_revision];
  if (
    matrix.contract !== MATRIX_CONTRACT ||
    matrix.schema_version !== 1 ||
    !contract ||
    matrix.release?.version !== config.release.version ||
    matrix.release?.channel !== config.release.channel ||
    matrix.release?.tag !== config.release.tag ||
    matrix.row_count !== contract.rowCount ||
    !Array.isArray(matrix.rows) ||
    matrix.rows.length !== contract.rowCount ||
    new Set(matrix.rows.map((row) => row.id)).size !== contract.rowCount
  ) {
    fail("clean-host matrix does not bind the endpoint release");
  }
}

function releaseBinding(config, configFile, statusFile) {
  const status = readJson(statusFile, "clean-host status binding");
  const binding = {
    id: config.release.id,
    version: config.release.version,
    python_version: config.release.python_version,
    channel: config.release.channel,
    tag: config.release.tag,
    source_commit: config.release.source_commit,
    endpoint_config_sha256: sha256File(configFile),
    operator_status_sha256: sha256File(statusFile),
  };
  const artifactDigest = process.env.AIT_CLEAN_HOST_CANDIDATE_ARTIFACT_DIGEST ?? "";
  if (!/^sha256:[0-9a-f]{64}$/.test(artifactDigest)) {
    fail("prepublish aggregate lacks the immutable candidate artifact digest", 64);
  }
  binding.verification_stage = "prepublication";
  binding.candidate_stage_receipt_sha256 = status.candidate.stage_receipt_sha256;
  binding.candidate_artifact_digest = artifactDigest;
  return binding;
}

function validatePhase(phase, expectedPhase, row, release) {
  if (
    phase.contract !== PHASE_CONTRACT ||
    phase.status !== "pass" ||
    phase.phase !== expectedPhase ||
    !sameJson(phase.release, release) ||
    !sameJson(phase.row, row) ||
    phase.runner?.label !== row.runner ||
    phase.runner?.target_verified !== true ||
    phase.runner?.github_hosted !== true ||
    !/^[1-9][0-9]*$/.test(String(phase.runner?.run_id ?? "")) ||
    !/^[1-9][0-9]*$/.test(String(phase.runner?.run_attempt ?? "")) ||
    typeof phase.runner?.job !== "string" ||
    phase.runner.job.length === 0
  ) {
    fail(`${row.id} ${expectedPhase} phase evidence is not an exact passing hosted run`);
  }
  exactKeys(
    phase.checks,
    row.required_checks[expectedPhase],
    `${row.id} ${expectedPhase} checks`,
  );
  if (!Object.values(phase.checks).every((value) => value === true)) {
    fail(`${row.id} ${expectedPhase} checks are incomplete`);
  }
}

function validateFailedPhase(phase, expectedPhase, row, release) {
  if (
    phase.contract !== PHASE_CONTRACT ||
    phase.status !== "fail" ||
    phase.phase !== expectedPhase ||
    !sameJson(phase.release, release) ||
    !sameJson(phase.row, row) ||
    phase.runner?.label !== row.runner ||
    phase.runner?.target_verified !== true ||
    phase.runner?.github_hosted !== true ||
    !/^[1-9][0-9]*$/.test(String(phase.runner?.run_id ?? "")) ||
    !/^[1-9][0-9]*$/.test(String(phase.runner?.run_attempt ?? "")) ||
    typeof phase.runner?.job !== "string" ||
    phase.runner.job.length === 0 ||
    typeof phase.error?.message !== "string" ||
    phase.error.message.length === 0
  ) {
    fail(`${row.id} ${expectedPhase} failed phase evidence is not attributable`);
  }
  exactKeys(
    phase.checks,
    row.required_checks[expectedPhase],
    `${row.id} ${expectedPhase} failed checks`,
  );
  if (!Object.values(phase.checks).every((value) => typeof value === "boolean")) {
    fail(`${row.id} ${expectedPhase} failed checks are not boolean`);
  }
}

function combineEvidence(matrix, config, status, configFile, statusFile, install, upgrade) {
  validateConfigAndStatus(config, status);
  validateMatrix(matrix, config);
  const rowId = install.row?.id;
  const row = matrix.rows.find((candidate) => candidate.id === rowId);
  if (!row || upgrade.row?.id !== rowId) {
    fail("clean-host phase evidence does not select one declared row");
  }
  const release = releaseBinding(config, configFile, statusFile);
  if (install.status === "pass") {
    validatePhase(install, "install", row, release);
  } else {
    validateFailedPhase(install, "install", row, release);
  }
  if (upgrade.status === "pass") {
    validatePhase(upgrade, "upgrade", row, release);
  } else {
    validateFailedPhase(upgrade, "upgrade", row, release);
  }
  if (install.runner.job === upgrade.runner.job) {
    fail(`${row.id} upgrade evidence reused the install job instead of a fresh host`);
  }
  const passed = install.status === "pass" && upgrade.status === "pass";
  return {
    contract: ROW_CONTRACT,
    status: passed ? "pass" : "fail",
    release,
    row,
    phases: { install, upgrade },
    isolation: {
      install_and_upgrade_jobs_distinct: true,
      fresh_github_hosted_vm_per_phase: true,
    },
    failures: [install, upgrade]
      .filter((phase) => phase.status !== "pass")
      .map((phase) => ({ phase: phase.phase, error: phase.error })),
  };
}

function rowEvidenceFiles(evidenceRoot) {
  requireRealDirectory(evidenceRoot, "clean-host evidence root");
  const entries = readdirSync(evidenceRoot, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.isSymbolicLink() || !entry.isFile() || !entry.name.endsWith(".json")) {
      fail(`clean-host evidence root contains an unexpected entry: ${entry.name}`);
    }
    files.push(path.join(evidenceRoot, entry.name));
  }
  return files.sort();
}

function aggregateEvidence(matrix, config, status, configFile, statusFile, evidenceRoot) {
  validateConfigAndStatus(config, status);
  validateMatrix(matrix, config);
  const expectedRelease = releaseBinding(config, configFile, statusFile);
  const expected = new Map(matrix.rows.map((row) => [row.id, row]));
  const observed = new Set();
  const admitted = new Map();
  const failures = [];
  const inventory = [];
  for (const file of rowEvidenceFiles(evidenceRoot)) {
    const digest = sha256File(file);
    const evidence = readJson(file, "clean-host row evidence");
    const id = evidence.row?.id;
    inventory.push({ path: `rows/${path.basename(file)}`, sha256: digest });
    if (typeof id !== "string" || !expected.has(id)) {
      failures.push({ path: path.basename(file), reason: "unexpected_row" });
      continue;
    }
    if (observed.has(id)) {
      failures.push({ row_id: id, reason: "duplicate_row" });
      continue;
    }
    observed.add(id);
    if (path.basename(file) !== `${id}.json`) {
      failures.push({ row_id: id, reason: "filename_mismatch" });
      continue;
    }
    const row = expected.get(id);
    try {
      if (
        evidence.contract !== ROW_CONTRACT ||
        evidence.status !== "pass" ||
        !sameJson(evidence.release, expectedRelease) ||
        !sameJson(evidence.row, row) ||
        evidence.isolation?.install_and_upgrade_jobs_distinct !== true ||
        evidence.isolation?.fresh_github_hosted_vm_per_phase !== true
      ) {
        fail("row envelope mismatch");
      }
      validatePhase(evidence.phases?.install, "install", row, expectedRelease);
      validatePhase(evidence.phases?.upgrade, "upgrade", row, expectedRelease);
      if (evidence.phases.install.runner.job === evidence.phases.upgrade.runner.job) {
        fail("phase jobs are not distinct");
      }
      admitted.set(id, { file, digest });
    } catch (error) {
      failures.push({ row_id: id, reason: "invalid_or_failed_row", detail: error.message });
    }
  }
  for (const id of expected.keys()) {
    if (!observed.has(id)) {
      failures.push({ row_id: id, reason: "missing_row" });
    }
  }
  inventory.sort((left, right) => left.path.localeCompare(right.path, "en"));
  failures.sort((left, right) =>
    JSON.stringify(left).localeCompare(JSON.stringify(right), "en"),
  );
  const passed =
    failures.length === 0 &&
    admitted.size === matrix.row_count &&
    inventory.length === matrix.row_count;
  const promotion = {
    allowed: passed,
    retry_same_candidate: !passed,
    terminal_for_release: false,
  };
  return {
    contract: AGGREGATE_CONTRACT,
    status: passed ? "qualified" : "blocked",
    release: expectedRelease,
    matrix: {
      revision: matrix.matrix_revision,
      expected_rows: matrix.row_count,
      admitted_rows: admitted.size,
      evidence_files: inventory.length,
    },
    evidence_inventory: inventory,
    failures,
    promotion,
  };
}

function writeAggregate(outputRoot, aggregate, evidenceRoot) {
  requireNewDirectory(outputRoot, "clean-host aggregate output");
  mkdirSync(path.join(outputRoot, "rows"), { recursive: true, mode: 0o755 });
  for (const file of rowEvidenceFiles(evidenceRoot)) {
    const destination = path.join(outputRoot, "rows", path.basename(file));
    writeFileSync(destination, readFileSync(file), { mode: 0o644 });
  }
  const statusFile = path.join(outputRoot, "ait-release.clean-host-status.json");
  writeFileSync(statusFile, encoded(aggregate), { encoding: "utf8", mode: 0o644 });
  const checksumRows = [
    ...aggregate.evidence_inventory.map((row) => `${row.sha256}  ${row.path}`),
    `${sha256File(statusFile)}  ait-release.clean-host-status.json`,
  ].sort();
  writeFileSync(path.join(outputRoot, "SHA256SUMS"), `${checksumRows.join("\n")}\n`, {
    encoding: "utf8",
    mode: 0o644,
  });
}

function main() {
  const command = process.argv[2];
  if (!command) {
    usage();
  }
  if (command === "matrix") {
    const options = parseOptions(process.argv.slice(3), ["family", "platforms", "output"]);
    const family = readJson(options.family, "release family");
    const platforms = readJson(options.platforms, "native platform manifest");
    writeNewJson(options.output, buildMatrix(family, platforms), "clean-host matrix output");
    process.stdout.write(`${options.output}\n`);
    return;
  }
  if (command === "combine") {
    const options = parseOptions(process.argv.slice(3), [
      "matrix",
      "config",
      "status",
      "install",
      "upgrade",
      "output",
    ]);
    const matrix = readJson(options.matrix, "clean-host matrix");
    const config = readJson(options.config, "endpoint configuration");
    const status = readJson(options.status, "operator status");
    const install = readJson(options.install, "install phase evidence");
    const upgrade = readJson(options.upgrade, "upgrade phase evidence");
    const evidence = combineEvidence(
      matrix,
      config,
      status,
      options.config,
      options.status,
      install,
      upgrade,
    );
    writeNewJson(options.output, evidence, "clean-host row output");
    process.stdout.write(`${options.output}\n`);
    if (evidence.status !== "pass") {
      process.stderr.write(
        `clean-host row ${evidence.row.id} is blocked by ${evidence.failures.length} failed phase(s)\n`,
      );
      process.exitCode = 1;
    }
    return;
  }
  if (command === "aggregate") {
    const options = parseOptions(process.argv.slice(3), [
      "matrix",
      "config",
      "status",
      "evidence-root",
      "output-root",
    ]);
    const matrix = readJson(options.matrix, "clean-host matrix");
    const config = readJson(options.config, "endpoint configuration");
    const status = readJson(options.status, "operator status");
    const aggregate = aggregateEvidence(
      matrix,
      config,
      status,
      options.config,
      options.status,
      options["evidence-root"],
    );
    writeAggregate(options["output-root"], aggregate, options["evidence-root"]);
    process.stdout.write(`${path.join(options["output-root"], "ait-release.clean-host-status.json")}\n`);
    if (aggregate.status !== "qualified") {
      process.stderr.write(
        `clean-host aggregate is blocked by ${aggregate.failures.length} evidence failure(s)\n`,
      );
      process.exitCode = 1;
    }
    return;
  }
  usage(`unsupported clean-host command: ${command}`);
}

try {
  main();
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitCode ?? 70);
}
