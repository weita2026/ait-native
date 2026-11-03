#!/usr/bin/env node
// Private real-process boundary for the assigned Reviewer. The Reviewer owns
// the native CLI invocation and its actor/review input. This driver only
// interrupts that process after the actual Server exposes its formal Job.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { readFileSync, realpathSync, writeFileSync } from "node:fs";
import path from "node:path";

const separator = process.argv.indexOf("--");
assert(separator > 2, "supply driver options, then -- and the Reviewer-owned native finish arguments");
const pairs = process.argv.slice(2, separator);
assert(pairs.length % 2 === 0, "driver options must be key/value pairs");
const options = Object.fromEntries(Array.from({ length: pairs.length / 2 }, (_, i) => pairs.slice(i * 2, i * 2 + 2)));
for (const key of ["--ait", "--repository", "--server", "--identity", "--stdout", "--stderr", "--boundary", "--repository-index", "--worker-job-index", "--task"]) {
  assert(options[key], `missing ${key}`);
}
for (const key of ["--ait", "--repository", "--identity", "--stdout", "--stderr", "--boundary"]) {
  assert(path.isAbsolute(options[key]), `${key} must be absolute`);
}
const args = process.argv.slice(separator + 1);
assert.deepEqual(args.slice(0, 3), ["workflow", "finish", options["--task"]]);
assert(args.includes("--apply") && args.includes("--json") && args.includes("--remote") &&
  args[args.indexOf("--remote") + 1] === "origin" &&
  args.includes("--reviewer") && args[args.indexOf("--reviewer") + 1] &&
  args.includes("--review-message") && args[args.indexOf("--review-message") + 1] &&
  args.includes("--expected-identity") && args[args.indexOf("--expected-identity") + 1] === options["--identity"],
  "Reviewer must submit the selected native finish with its original pin");
const index = Number(options["--repository-index"]), jobIndex = Number(options["--worker-job-index"]);
assert(Number.isSafeInteger(index) && index >= 0 && Number.isSafeInteger(jobIndex) && jobIndex >= 0);
const remoteList = spawnSync(options["--ait"], ["remote", "list", "--json"], {
  cwd: realpathSync(options["--repository"]), env: process.env, encoding: "utf8",
});
assert.equal(remoteList.status, 0, "Reviewer fixture must resolve its configured native Remote");
assert.equal(JSON.parse(remoteList.stdout).find(remote => remote.name === "origin")?.url,
  options["--server"], "process boundary must target the fixture's configured origin");
const identity = JSON.parse(readFileSync(options["--identity"], "utf8"));
assert.equal(identity.repository_index, index);
assert.equal(identity.worker_job_index, jobIndex);
const jobUrl = `${options["--server"]}/v1/native/repository-authorities/${index}/worker-jobs/${jobIndex}`;
const waitMs = Number(options["--wait-ms"] ?? 120000);
assert(Number.isSafeInteger(waitMs) && waitMs > 0 && waitMs <= 3600000);
const deadline = Date.now() + waitMs;
async function job() {
  const response = await fetch(jobUrl, { cache: "no-store", signal: AbortSignal.timeout(3000) });
  assert.equal(response.status, 200, `native Job read returned HTTP ${response.status}`);
  const body = await response.json();
  return body.data ?? body;
}
const before = await job();
assert.equal(before.job.patchset_index, null, "formal finish already began before process boundary");
assert.deepEqual(before.package_execution?.expected_identity, identity);
assert.equal(before.package_execution?.ci_state, "passed");
let stdout = "", stderr = "", closed = false, closeCode = null, closeSignal = null;
const child = spawn(options["--ait"], args, { cwd: options["--repository"], env: process.env, stdio: ["ignore", "pipe", "pipe"] });
child.stdout.on("data", chunk => { stdout += chunk; });
child.stderr.on("data", chunk => { stderr += chunk; });
const completion = new Promise(resolve => child.once("close", (code, signal) => {
  closed = true; closeCode = code; closeSignal = signal; resolve();
}));
let acceptedJob = null, error = null, interrupted = false;
try {
  while (Date.now() < deadline && !closed) {
    const current = await job();
    if (current.job.patchset_index !== null) {
      acceptedJob = current;
      if (!closed && stdout.length === 0) {
        interrupted = child.kill("SIGKILL");
      }
      break;
    }
    await new Promise(resolve => setTimeout(resolve, 1));
  }
  if (!acceptedJob && !closed) child.kill("SIGKILL");
  await completion;
  assert(acceptedJob, "native formal Job was not observed before the finish process closed");
  assert(interrupted && closeSignal === "SIGKILL" && stdout.length === 0,
    "finish response was delivered or the native process closed before controlled interruption");
} catch (failure) {
  error = failure;
  if (!closed) child.kill("SIGKILL");
  await completion;
} finally {
  writeFileSync(options["--stdout"], stdout, { flag: "wx", mode: 0o600 });
  writeFileSync(options["--stderr"], stderr, { flag: "wx", mode: 0o600 });
  writeFileSync(options["--boundary"], JSON.stringify({
    status: error ? "unconfirmed" : "interrupted_after_formal_job",
    repository_index: index, worker_job_index: jobIndex,
    patchset_index: acceptedJob?.job.patchset_index ?? null,
    client_exit_code: closeCode, client_signal: closeSignal,
    stdout_bytes: Buffer.byteLength(stdout),
  }), { flag: "wx", mode: 0o600 });
}
if (error) throw error;
process.stdout.write("Reviewer-owned native finish process interrupted after formal Job observation; reconcile exact acceptance read-only.\n");
