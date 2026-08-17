#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import path from "node:path";

function fail(message, code = 65) {
  const error = new Error(message);
  error.exitCode = code;
  throw error;
}

function usage(message = null) {
  if (message) {
    process.stderr.write(`${message}\n`);
  }
  process.stderr.write(
    "usage:\n" +
      "  release_prepublish_verify.mjs stage --root <absolute-dir> --config-sha256 <sha> --status-sha256 <sha>\n" +
      "  release_prepublish_verify.mjs qualify --root <absolute-dir> --candidate-root <absolute-dir> --candidate-artifact-digest <sha256:digest> --aggregate-sha256 <sha>\n",
  );
  process.exit(64);
}

function options(argv, names) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined) {
      usage("prepublish verifier option requires a value");
    }
    const name = key.slice(2);
    if (!names.includes(name) || Object.hasOwn(parsed, name)) {
      usage(`unsupported or repeated prepublish verifier option: ${key}`);
    }
    parsed[name] = value;
  }
  if (names.some((name) => !Object.hasOwn(parsed, name))) {
    usage("prepublish verifier is missing a required option");
  }
  return parsed;
}

function sha256Bytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function regularFile(file, label) {
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

function sha256File(file) {
  regularFile(file, "checksum input");
  return sha256Bytes(readFileSync(file));
}

function realDirectory(directory, label) {
  if (!path.isAbsolute(directory)) {
    fail(`${label} must be absolute`, 64);
  }
  let stat;
  try {
    stat = lstatSync(directory);
  } catch {
    fail(`${label} is unavailable: ${directory}`, 66);
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail(`${label} must be a real directory`, 66);
  }
  return realpathSync(directory);
}

function readJson(file, label) {
  regularFile(file, label);
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    fail(`${label} is invalid JSON: ${error.message}`);
  }
}

function inventory(root) {
  const files = [];
  const stack = [root];
  while (stack.length > 0) {
    const directory = stack.pop();
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const file = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) {
        fail(`prepublish artifact contains a symbolic link: ${file}`);
      }
      if (entry.isDirectory()) {
        stack.push(file);
      } else if (entry.isFile()) {
        files.push(path.relative(root, file).split(path.sep).join("/"));
      } else {
        fail(`prepublish artifact contains a special file: ${file}`);
      }
    }
  }
  return files.sort();
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    same(Object.keys(value).sort(), [...expected].sort())
  );
}

function verifyChecksumInventory(root, name) {
  const checksumFile = path.join(root, name);
  regularFile(checksumFile, "prepublish checksum inventory");
  const declared = new Map();
  for (const line of readFileSync(checksumFile, "utf8").split(/\r?\n/)) {
    if (!line) {
      continue;
    }
    const match = line.match(/^([0-9a-f]{64})  ([A-Za-z0-9][A-Za-z0-9._@+\/-]*)$/);
    if (
      !match ||
      match[2].includes("..") ||
      match[2].includes("//") ||
      path.posix.isAbsolute(match[2]) ||
      declared.has(match[2])
    ) {
      fail(`prepublish checksum inventory contains an unsafe or duplicate row: ${line}`);
    }
    declared.set(match[2], match[1]);
  }
  const actual = inventory(root).filter((relative) => relative !== name);
  if (!same(actual, [...declared.keys()].sort())) {
    fail("prepublish checksum inventory does not cover the exact file set");
  }
  for (const [relative, expected] of declared) {
    if (sha256File(path.join(root, ...relative.split("/"))) !== expected) {
      fail(`prepublish staged file digest drifted: ${relative}`);
    }
  }
  return declared;
}

function verifyStage(root, configSha, statusSha) {
  if (!/^[0-9a-f]{64}$/.test(configSha) || !/^[0-9a-f]{64}$/.test(statusSha)) {
    fail("prepublish stage expected digest is invalid", 64);
  }
  verifyChecksumInventory(root, "PREPUBLISH_SHA256SUMS");

  const configFile = path.join(root, "ait-release.endpoints.authority.json");
  const statusFile = path.join(root, "ait-release.prepublish-candidate.json");
  const receiptFile = path.join(root, "ait-release.prepublish-stage.json");
  if (sha256File(configFile) !== configSha || sha256File(statusFile) !== statusSha) {
    fail("prepublish stage config or candidate status digest differs");
  }
  const config = readJson(configFile, "prepublish endpoint config");
  const status = readJson(statusFile, "prepublish candidate status");
  const receipt = readJson(receiptFile, "prepublish stage receipt");
  const expectedMutation = {
    artifact_rebuild: false,
    component_rebuild: false,
    endpoint_write: false,
    github_release_write: false,
    registry_write: false,
    service_start: false,
    tag_write: false,
  };
  if (
    receipt.contract !== "ait.release.prepublish.stage/v1" ||
    receipt.status !== "frozen_candidate_staged" ||
    receipt.authority?.endpoint_config_sha256 !== configSha ||
    receipt.authority?.endpoint_stage_receipt_sha256 !==
      sha256File(path.join(root, "ait-release.endpoint-publication.json")) ||
    receipt.authority?.assets_checksum_sha256 !==
      sha256File(path.join(root, "assets", "SHA256SUMS")) ||
    status.contract !== "ait.release.prepublish.candidate/v1" ||
    status.status !== "frozen_candidate_pending_clean_host" ||
    status.candidate?.stage_receipt_sha256 !== sha256File(receiptFile) ||
    !same(status.release, receipt.release) ||
    status.release?.id !== config.release?.id ||
    status.release?.version !== config.release?.version ||
    status.release?.tag !== config.release?.tag ||
    status.release?.source_commit !== config.release?.source_commit ||
    status.public_endpoint_writes !== false ||
    !same(receipt.mutation, expectedMutation) ||
    !same(status.candidate?.oci, receipt.oci)
  ) {
    fail("prepublish stage receipt, status, and endpoint config are not one frozen candidate");
  }
  const components = ["ait-server", "ait-runner"];
  const architectures = ["amd64", "arm64"];
  if (!exactKeys(receipt.oci, components)) {
    fail("prepublish OCI component inventory is not exact");
  }
  for (const component of components) {
    if (!exactKeys(receipt.oci[component], architectures)) {
      fail(`prepublish OCI architecture inventory is not exact: ${component}`);
    }
    for (const architecture of architectures) {
      const row = receipt.oci?.[component]?.[architecture];
      if (
        !row ||
        !exactKeys(row, ["archive", "sha256", "reference", "image_id"]) ||
        !/^[A-Za-z0-9][A-Za-z0-9._-]*\.docker\.tar$/.test(row.archive ?? "") ||
        !/^[0-9a-f]{64}$/.test(row.sha256 ?? "") ||
        !/^sha256:[0-9a-f]{64}$/.test(row.image_id ?? "") ||
        row.reference !==
          `ait-prepublish/${component}:${status.release.version}-${architecture}` ||
        sha256File(path.join(root, "oci-archives", row.archive)) !== row.sha256
      ) {
        fail(`prepublish OCI authority is invalid: ${component}/${architecture}`);
      }
    }
  }
  return { config, status, receipt };
}

function verifyQualification(root, candidateRoot, artifactDigest, aggregateSha) {
  if (
    !/^sha256:[0-9a-f]{64}$/.test(artifactDigest) ||
    !/^[0-9a-f]{64}$/.test(aggregateSha)
  ) {
    fail("prepublish qualification expected digest is invalid", 64);
  }
  const aggregateFile = path.join(root, "ait-release.clean-host-status.json");
  const copiedStatus = path.join(root, "ait-release.prepublish-candidate.json");
  const candidateStatus = path.join(candidateRoot, "ait-release.prepublish-candidate.json");
  if (sha256File(aggregateFile) !== aggregateSha) {
    fail("prepublish aggregate status digest differs");
  }
  verifyChecksumInventory(root, "SHA256SUMS");
  if (sha256File(copiedStatus) !== sha256File(candidateStatus)) {
    fail("prepublish aggregate does not retain the exact candidate status");
  }
  const aggregate = readJson(aggregateFile, "prepublish aggregate status");
  const status = readJson(candidateStatus, "prepublish candidate status");
  if (
    aggregate.contract !== "ait.release.clean-host.aggregate/v1" ||
    aggregate.status !== "qualified" ||
    aggregate.release?.verification_stage !== "prepublication" ||
    aggregate.release?.candidate_artifact_digest !== artifactDigest ||
    aggregate.release?.candidate_stage_receipt_sha256 !==
      status.candidate?.stage_receipt_sha256 ||
    aggregate.release?.operator_status_sha256 !== sha256File(candidateStatus) ||
    aggregate.matrix?.expected_rows !== 32 ||
    aggregate.matrix?.admitted_rows !== 32 ||
    aggregate.matrix?.evidence_files !== 32 ||
    !Array.isArray(aggregate.failures) ||
    aggregate.failures.length !== 0 ||
    !same(aggregate.promotion, {
      allowed: true,
      retry_same_candidate: false,
      terminal_for_release: false,
    })
  ) {
    fail("prepublish aggregate is not an exact complete qualification");
  }
  const rows = inventory(path.join(root, "rows"));
  if (rows.length !== 32 || rows.some((name) => !name.endsWith(".json"))) {
    fail("prepublish qualification does not retain exactly 32 row records");
  }
  return aggregate;
}

const command = process.argv[2];
try {
  if (command === "stage") {
    const parsed = options(process.argv.slice(3), ["root", "config-sha256", "status-sha256"]);
    const root = realDirectory(parsed.root, "prepublish stage root");
    verifyStage(root, parsed["config-sha256"], parsed["status-sha256"]);
    process.stdout.write(`${root}\n`);
  } else if (command === "qualify") {
    const parsed = options(process.argv.slice(3), [
      "root",
      "candidate-root",
      "candidate-artifact-digest",
      "aggregate-sha256",
    ]);
    const root = realDirectory(parsed.root, "prepublish aggregate root");
    const candidate = realDirectory(parsed["candidate-root"], "prepublish candidate root");
    verifyQualification(
      root,
      candidate,
      parsed["candidate-artifact-digest"],
      parsed["aggregate-sha256"],
    );
    process.stdout.write(`${root}\n`);
  } else {
    usage(command ? `unsupported prepublish verifier command: ${command}` : null);
  }
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitCode ?? 70);
}
