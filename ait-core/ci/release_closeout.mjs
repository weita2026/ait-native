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
  if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) fail("invalid release closeout arguments", 64);
  options[key] = value;
}
const recordsRoot = options["--records-root"];
const version = options["--version"];
const output = options["--output"];
if (!path.isAbsolute(recordsRoot ?? "") || !path.isAbsolute(output ?? "") || !/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  fail("release closeout arguments are invalid", 64);
}
if (!output.startsWith(`${recordsRoot}${path.sep}`) || existsSync(output)) fail("release closeout output is invalid", 73);

function readRecord(relative, label) {
  const file = path.join(recordsRoot, relative);
  if (!existsSync(file) || !lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) fail(`${label} is missing`, 66);
  const bytes = readFileSync(file);
  let document;
  try { document = JSON.parse(bytes); } catch { fail(`${label} is invalid JSON`); }
  return { relative, file, bytes, document, sha256: createHash("sha256").update(bytes).digest("hex") };
}

const candidate = readRecord("candidate.json", "candidate record");
const web = readRecord("web-admission/web-admission.json", "Web admission");
const tag = readRecord("public-tag/receipt.json", "tag receipt");
const endpoints = readRecord("endpoints.json", "endpoint configuration");
const status = readRecord("operator-status.json", "endpoint status");
const winget = readRecord("winget-submission.json", "WinGet submission");
const latest = readRecord("latest-alias-cache/cache.json", "latest-alias cache");
const releaseId = candidate.document.release?.id;
if (
  candidate.document.status !== "ready_for_immutable_tag" || candidate.document.release?.version !== version ||
  web.document.status !== "admitted" || web.document.release_id !== releaseId ||
  tag.document.status !== "complete" || tag.document.kind !== "tag" ||
  endpoints.document.release?.id !== releaseId || endpoints.document.release?.version !== version ||
  status.document.status !== "published_readback_complete" || status.document.release?.id !== releaseId ||
  winget.document.contract !== "ait.release.winget-submission/v1" || winget.document.version !== version ||
  !new Set(["submitted", "merged"]).has(winget.document.status) ||
  latest.document.contract !== "ait.release.artifact-cache/v1" || latest.document.status !== "complete"
) fail("release closeout evidence is incomplete or inconsistent");

for (const relative of ["conductor-plan.json", "conductor-state.json"]) {
  const file = path.join(recordsRoot, relative);
  if (existsSync(file) && /[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/.test(readFileSync(file, "utf8"))) {
    fail(`release conductor exposes an internal Change reference: ${relative}`);
  }
}

const records = [candidate, web, tag, endpoints, status, winget, latest];
const closeout = {
  contract: "ait.release.closeout/v1",
  release: { id: releaseId, version, tag: `v${version}` },
  status: winget.document.status === "merged" ? "published" : "published_winget_review_pending",
  endpoints: "published_readback_complete",
  web_admission: "admitted",
  latest_aliases: "promoted",
  winget: {
    status: winget.document.status,
    pull_request: winget.document.pull_request,
  },
  public_cli_identity: { internal_change_references_in_plan_or_state: 0 },
  evidence: records.map((row) => ({ path: row.relative, sha256: row.sha256 })),
  completed_at: new Date().toISOString(),
};
writeFileSync(output, `${JSON.stringify(closeout, null, 2)}\n`, { mode: 0o600, flag: "wx" });
process.stdout.write(`${closeout.release.tag}: ${closeout.status}\n`);
