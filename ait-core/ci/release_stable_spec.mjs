#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

const argv = process.argv.slice(2);
const options = {};
while (argv.length > 0) {
  const key = argv.shift();
  const value = argv.shift();
  if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) fail("invalid stable release spec arguments", 64);
  options[key] = value;
}
const inputPath = options["--input"];
const output = options["--output"];
if (!path.isAbsolute(inputPath ?? "") || !path.isAbsolute(output ?? "")) fail("stable release spec paths must be absolute", 64);
if (!existsSync(inputPath) || !lstatSync(inputPath).isFile() || lstatSync(inputPath).isSymbolicLink()) fail("stable release input must be a regular file", 66);
if (existsSync(output)) fail("stable release spec output already exists", 73);
const bytes = readFileSync(inputPath);
let input;
try { input = JSON.parse(bytes); } catch { fail("stable release input is invalid JSON"); }
if (input.contract !== "ait.release.accepted-input/v1" || input.release?.channel !== "stable") fail("stable release input contract is invalid");
const absolute = (value, label) => {
  if (typeof value !== "string" || !path.isAbsolute(value)) fail(`${label} must be absolute`);
  return path.resolve(value);
};
const stable = (value) => /^\d+\.\d+\.\d+$/.test(value ?? "");
const reviewable = (value) => typeof value === "string" &&
  /Reviewed files:\s+\S[\s\S]*Findings:\s+\S[\s\S]*Risks:\s+\S[\s\S]*Tests:\s+\S[\s\S]*Recommendation:\s+\S/.test(value);
if (!stable(input.release.version) || !stable(input.release.prior_version)) fail("stable release versions are invalid");
const recordsRoot = absolute(input.records_root, "records_root");
if (!output.startsWith(`${recordsRoot}${path.sep}`)) fail("stable release spec output must remain below records_root");

const paths = input.paths ?? {};
const requiredPaths = [
  "core_root", "public_source", "prior_family", "prior_coordinator", "final_family", "final_coordinator",
  "web_components", "web_test_root", "personal_root", "personal_bin", "community_bin",
  "community_cli_bin", "chrome",
];
for (const key of requiredPaths) paths[key] = absolute(paths[key], `paths.${key}`);
for (const key of requiredPaths) {
  if (!existsSync(paths[key])) fail(`stable release input path is missing: paths.${key}`, 66);
}
if (!/^http:\/\/(127\.0\.0\.1|localhost):[0-9]+\/$/.test(input.web?.server_url ?? "")) fail("stable release server URL is invalid");
if (!Number.isSafeInteger(input.web?.seed) || input.web.seed <= 0) fail("stable release Web seed is invalid");
if (!existsSync(input.web?.server_data ?? "") || !lstatSync(input.web.server_data).isDirectory()) fail("stable release server data root is missing", 66);
if (!/^[^/]+\/[^/]+$/.test(input.github_repository ?? "") || !/^[^/]+\/[^/]+$/.test(input.winget_fork ?? "")) {
  fail("stable release GitHub repository input is invalid");
}
if (!Array.isArray(input.component_closeouts) || input.component_closeouts.length !== 5) {
  fail("stable release input requires exactly five component closeouts");
}
const ids = input.component_closeouts.map((row) => row.id).sort();
if (JSON.stringify(ids) !== JSON.stringify(["core", "node", "python", "runner", "server"])) {
  fail("stable release component inventory is incomplete");
}
for (const row of input.component_closeouts) {
  if (
    !/^[a-z]+$/.test(row.id ?? "") || !/^[A-Z]+T-[0-9]{4}$/.test(row.task ?? "") ||
    !/^SNP-[0-9A-F]{12}$/.test(row.snapshot ?? "") ||
    (row.patchset !== null && !new RegExp(`^${row.task}/P-[0-9]{2}$`).test(row.patchset ?? "")) ||
    typeof row.finish_local_before_ready !== "boolean" ||
    typeof row.remote !== "string" || !row.remote
  ) fail(`stable release component closeout is invalid: ${row.id ?? "unknown"}`);
  if (!reviewable(row.review_message)) fail(`stable release component review is incomplete: ${row.id}`);
  row.repository_root = absolute(row.repository_root, `${row.id}.repository_root`);
  row.edit_root = absolute(row.edit_root, `${row.id}.edit_root`);
}

const spec = {
  contract: "ait.release.conductor-spec/v1",
  release: input.release,
  records_root: recordsRoot,
  values: {
    accepted_input_path: inputPath,
    accepted_input_sha256: createHash("sha256").update(bytes).digest("hex"),
    core_root: paths.core_root,
    public_source: paths.public_source,
    prior_family: paths.prior_family,
    prior_coordinator: paths.prior_coordinator,
    final_family: paths.final_family,
    final_coordinator: paths.final_coordinator,
    web_components: paths.web_components,
    web_test_root: paths.web_test_root,
    personal_root: paths.personal_root,
    personal_bin: paths.personal_bin,
    community_bin: paths.community_bin,
    community_cli_bin: paths.community_cli_bin,
    chrome: paths.chrome,
    server_url: input.web.server_url,
    server_data: absolute(input.web.server_data, "web.server_data"),
    web_seed: String(input.web.seed),
    github_repository: input.github_repository,
    winget_fork: input.winget_fork,
  },
  collections: { component_closeouts: input.component_closeouts },
};
writeFileSync(output, `${JSON.stringify(spec, null, 2)}\n`, { mode: 0o600, flag: "wx" });
process.stdout.write(`${output}\n`);
