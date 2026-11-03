#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { digest } from "./release_receipt_transaction.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const temporary = mkdtempSync(path.join(os.tmpdir(), "ait-product-gate-test."));
try {
  const root = path.join(temporary, "records"); mkdirSync(root);
  const evidence = path.join(root, "legacy-32.json");
  writeFileSync(evidence, JSON.stringify({ status: "pass", clean_host_rows: 32 }));
  const marker = path.join(root, "tag-created");
  const names = ["source_preflight", "component_release", "candidate_freeze", "public_qualification", "candidate_admission", "component_receipts", "clean_host_qualification", "web_admission", "tag", "protected_promotion", "endpoint_publication", "winget_submission", "latest_alias", "closeout"];
  const plan = { contract: "ait.release.conductor-plan/v1", release: { version: "1.2.0", prior_version: "1.1.4", channel: "stable", tag: "v1.2.0" }, records_root: root,
    phases: names.map(id => ({ id, actions: id === "tag" ? [{ id: "tag", type: "command", cwd: root, mutation: false,
      argv: [process.execPath, "-e", `require('node:fs').writeFileSync(${JSON.stringify(marker)},'wrong')`], outputs: [marker] }] :
      [{ id: "legacy-" + id, type: "evidence_gate", path: evidence, message: "legacy supporting evidence" }] })) };
  for (const phase of plan.phases) {
    if (phase.id === "component_release") phase.actions = [{ id: "component", type: "ait_closeout", task: "LXT-0001", snapshot: "SNP-111111111111",
      repository_root: root, edit_root: root, review_message: "Reviewed files: fixture. Findings: pending. Risks: fixture. Tests: source. Recommendation: await Reviewer." }];
    if (["public_qualification", "component_receipts", "clean_host_qualification", "protected_promotion", "endpoint_publication", "latest_alias"].includes(phase.id)) {
      phase.actions = [{ id: "workflow-" + phase.id, type: "github_workflow", repository: "fixture/fixture", workflow: phase.id,
        head_sha: "a".repeat(40), cwd: root, dispatch_argv: [process.execPath, "-e", "process.exit(99)"] }];
    }
    if (["tag", "winget_submission"].includes(phase.id)) phase.actions = [{ id: "mutate-" + phase.id, type: "command", cwd: root, mutation: true,
      argv: [process.execPath, "-e", `require('node:fs').writeFileSync(${JSON.stringify(marker)},'wrong')`], outputs: [marker],
      probe: { cwd: root, argv: [process.execPath, "-e", "process.exit(99)"] } }];
  }
  const planFile = path.join(root, "conductor-plan.json"); writeFileSync(planFile, JSON.stringify(plan));
  const identity = { repository_index: 1, package_hash: "ab".repeat(32), worker_job_index: 0,
    base_snapshot_id: "SNP-222222222222", revision_snapshot_id: "SNP-111111111111" };
  const completion = { outcome: "confirmed", expected_identity: identity, patchset_id: "RXT-0001/P-01", submission_id: "RXT-0001/L-01",
    entries: [{ local_task_id: "LXT-0001", remote_task_id: "RXT-0001", changes: [] }] };
  mkdirSync(path.join(root, "conductor-logs"));
  const finishLog = path.join(root, "conductor-logs/component.ait.finish.log");
  writeFileSync(finishLog, JSON.stringify({ contract: "ait-agent-action/v1", command: "workflow.finish", apply_status: "done",
    next_action: { code: "done" }, expected_identity: identity, closeout_status: "complete", completion }));
  // A legacy completed control state is supporting evidence, never A120 pass.
  writeFileSync(path.join(root, "state.json"), JSON.stringify({ contract: "ait.release.conductor-state/v1", release: plan.release,
    plan_sha256: createHash("sha256").update(readFileSync(planFile)).digest("hex"), status: "complete",
    phases: plan.phases.map(phase => ({ id: phase.id, status: "complete", actions: phase.actions.map(action => ({ id: action.id, status: "complete",
      ...(action.type === "ait_closeout" ? { expected_identity: identity, completion, closeout_status: "complete",
        outputs: { [finishLog]: digest(finishLog) } } : {}) })) })) }));
  for (let i = 0; i < 2; i++) {
    const result = spawnSync(process.execPath, [path.join(here, "release_conductor.mjs"), "run", "--plan", planFile, "--state", path.join(root, "state.json")], { encoding: "utf8" });
    assert.equal(result.status, 75, result.stderr);
    assert.match(result.stderr, /Pre-tag operator HOLD/);
    for (const id of ["A120-01", "A120-09", "A120-10"]) assert(result.stderr.includes(id));
    assert.equal(existsSync(marker), false);
  }
  const state = JSON.parse(readFileSync(path.join(root, "state.json")));
  assert.equal(state.phases.find(row => row.id === "tag").status, "action_required");
  assert(state.phases.find(row => row.id === "tag").actions.every(row => row.status === "action_required"));
  assert.equal(state.phases.find(row => row.id === "clean_host_qualification").status, "complete");
  console.log("release product pre-tag blocking checks passed");
} finally { rmSync(temporary, { recursive: true, force: true }); }
