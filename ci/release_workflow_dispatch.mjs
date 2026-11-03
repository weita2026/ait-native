#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail("usage: release_workflow_dispatch.mjs --record <absolute-json> --selector <dot-path|endpoint_publication|latest_alias> [--status-record <absolute-json>]", 64);
}

const argv = process.argv.slice(2);
const options = {};
while (argv.length > 0) {
  const key = argv.shift();
  const value = argv.shift();
  if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) usage();
  options[key] = value;
}
const recordPath = options["--record"];
const selector = options["--selector"];
const statusPath = options["--status-record"];
if (
  !path.isAbsolute(recordPath ?? "") ||
  !/^[a-zA-Z0-9_-]+(?:\.[a-zA-Z0-9_-]+)*$/.test(selector ?? "") ||
  (statusPath !== undefined && !path.isAbsolute(statusPath))
) usage();
if (!existsSync(recordPath) || !lstatSync(recordPath).isFile() || lstatSync(recordPath).isSymbolicLink()) {
  fail("release workflow dispatch record must be a regular non-symlink file", 66);
}
let record;
try { record = JSON.parse(readFileSync(recordPath, "utf8")); } catch { fail("release workflow dispatch record is invalid JSON"); }
const hash = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");
const encoded = (file) => readFileSync(file).toString("base64");
let dispatch;
let repository = record?.release?.repository;
if (selector === "endpoint_publication") {
  repository = record?.publisher?.repository;
  dispatch = {
    workflow: record?.publisher?.workflow,
    ref: "main",
    inputs: {
      release_id: record?.release?.id,
      protected_run_id: record?.protected_authorization?.workflow_run_id,
      endpoint_config_sha256: hash(recordPath),
      endpoint_config_b64: encoded(recordPath),
      publish_exact_frozen_bytes: true,
    },
  };
} else if (selector === "latest_alias") {
  if (!statusPath || !existsSync(statusPath) || !lstatSync(statusPath).isFile() || lstatSync(statusPath).isSymbolicLink()) {
    fail("latest-alias status record must be a regular non-symlink file", 66);
  }
  let status;
  try { status = JSON.parse(readFileSync(statusPath, "utf8")); } catch { fail("latest-alias status record is invalid JSON"); }
  if (status?.status !== "published_readback_complete" || status?.release?.id !== record?.release?.id) {
    fail("latest-alias status does not admit the endpoint configuration");
  }
  repository = record?.publisher?.repository;
  dispatch = {
    workflow: "ait-release-latest-alias.yml",
    ref: "main",
    inputs: {
      release_id: record?.release?.id,
      endpoint_config_sha256: hash(recordPath),
      endpoint_config_b64: encoded(recordPath),
      operator_status_sha256: hash(statusPath),
      operator_status_b64: encoded(statusPath),
      promote_exact_release: true,
    },
  };
} else {
  dispatch = selector.split(".").reduce((current, part) => current?.[part], record);
}
if (
  !/^[^/]+\/[^/]+$/.test(repository ?? "") ||
  typeof dispatch?.workflow !== "string" || dispatch.workflow.length === 0 ||
  typeof dispatch?.ref !== "string" || dispatch.ref.length === 0 ||
  dispatch.inputs === null || typeof dispatch.inputs !== "object" || Array.isArray(dispatch.inputs)
) fail("release workflow dispatch contract is invalid");
const command = ["workflow", "run", dispatch.workflow, "--repo", repository, "--ref", dispatch.ref];
for (const key of Object.keys(dispatch.inputs).sort()) {
  if (!/^[a-zA-Z0-9_-]+$/.test(key)) fail(`release workflow input name is invalid: ${key}`);
  const value = dispatch.inputs[key];
  if (!["string", "number", "boolean"].includes(typeof value)) fail(`release workflow input is not scalar: ${key}`);
  command.push("-f", `${key}=${String(value)}`);
}
const result = spawnSync("gh", command, { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
if (result.error) fail(`release workflow dispatch could not start gh: ${result.error.message}`, 69);
if (result.status !== 0) {
  if (result.stdout) process.stderr.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  process.exit(result.status ?? 1);
}
process.stdout.write(`${dispatch.workflow}: dispatched\n`);
