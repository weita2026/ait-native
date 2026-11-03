#!/usr/bin/env node
import assert from "node:assert/strict";
import { originalLocalSnapshot, packageIdentity, samePackage, repreparedPackageIdentity, workflowResult, packageReadiness, packageAcceptance, pendingPackageAcceptance } from "./release_native_workflow.mjs";

const identity = { repository_index: 23, package_hash: "ab".repeat(32), worker_job_index: 0,
  base_snapshot_id: "SNP-111111111111", revision_snapshot_id: "SNP-222222222222" };
const task = { task_id: "LWT-0001", status: "completed", local_finish: { task_id: "LWT-0001", status: "succeeded",
  target_line: "main", landed_snapshot_id: identity.revision_snapshot_id }, head_snapshot_id: "SNP-333333333333" };
assert.equal(originalLocalSnapshot(task, task.task_id), identity.revision_snapshot_id);
assert.throws(() => originalLocalSnapshot(task, "LWT-0002"));
assert.throws(() => originalLocalSnapshot({ ...task, local_finish: null }, task.task_id));
assert.throws(() => originalLocalSnapshot(task, task.task_id, task.head_snapshot_id));
assert.deepEqual(packageIdentity(identity), identity);
assert.throws(() => packageIdentity({ ...identity, author_mode: "ai_only_experimental" }));
assert.throws(() => packageIdentity({ ...identity, worker_job_index: -1 }));
assert.equal(samePackage(identity, { ...identity, worker_job_index: 1 }), false);
const ready = { contract: "ait-agent-action/v1", command: "workflow.ready", apply_status: "done", next_action: { code: "done" },
  expected_identity: identity, package_execution: { expected_identity: identity, ci_state: "passed", availability: "available", recoverable: true } };
assert.deepEqual(workflowResult(ready, "ready"), identity);
assert.equal(packageReadiness(ready), true);
assert.equal(packageAcceptance(ready, identity, task.task_id), null);
assert.equal(packageReadiness({ ...ready, package_execution: { ...ready.package_execution, ci_state: "queued" } }), false);
assert.throws(() => workflowResult({ ...ready, package_execution: { ...ready.package_execution, expected_identity: { ...identity, worker_job_index: 1 } } }, "ready"));
const nextIdentity = { ...identity, worker_job_index: 1 };
const nextReady = { ...ready, expected_identity: nextIdentity, package_execution: { ...ready.package_execution, expected_identity: nextIdentity } };
assert.deepEqual(repreparedPackageIdentity(identity, nextIdentity), nextIdentity);
assert.throws(() => repreparedPackageIdentity(identity, identity));
for (const key of ["repository_index", "package_hash", "base_snapshot_id", "revision_snapshot_id"]) {
  const foreign = { ...nextIdentity, [key]: key.endsWith("index") ? 24 : key.endsWith("hash") ? "cd".repeat(32) : "SNP-999999999999" };
  assert.throws(() => repreparedPackageIdentity(identity, foreign));
}
// Full native Ready omits command. It is accepted only with its actual
// invocation and existing selector path; this is schema coverage, not lineage
// evidence for the retained failed integration scene.
const invocation = { task: task.task_id, remote: "origin", pinPath: "/original/package-identity.json", expectedIdentity: identity };
const full = { ...nextReady, mode: "workflow_package", task_id: task.task_id, remote_name: "origin", expected_identity_path: invocation.pinPath };
delete full.command;
assert.throws(() => workflowResult(full, "ready"));
assert.deepEqual(workflowResult(full, "ready", null, { invocation }), nextIdentity);
const fullPending = { ...full, apply_status: "stopped", next_action: { code: "ci_pending" },
  package_execution: { ...full.package_execution, ci_state: "queued" } };
assert.deepEqual(workflowResult(fullPending, "ready", null, { invocation }), nextIdentity);
for (const wrong of [{ task_id: "LWT-9999" }, { remote_name: "other" }, { expected_identity_path: "/manually-selected/pin.json" },
  { expected_identity: { ...nextIdentity, package_hash: "cd".repeat(32) } }, { expected_identity: null }, { mode: "other" }]) {
  assert.throws(() => workflowResult({ ...full, ...wrong }, "ready", null, { invocation }));
}
assert.throws(() => workflowResult({ ...full, apply_status: "stopped", expected_identity: null, next_action: { code: "execution_unknown" } }, "ready", null, { invocation }));
const readonly = { ...ready }; delete readonly.apply_status;
assert.throws(() => workflowResult(readonly, "ready", identity));
assert.deepEqual(workflowResult(readonly, "ready", identity, { readonly: true }), identity);
assert.throws(() => workflowResult({ ...readonly, apply_status: {} }, "ready", identity, { readonly: true }));
const completion = { outcome: "confirmed", expected_identity: identity, patchset_id: "RWT-0008/P-01", submission_id: "RWT-0008/L-01",
  entries: [{ local_task_id: task.task_id, remote_task_id: "RWT-0008", changes: [{ local_change_ref: "LWT-0001/C-03", remote_change_ref: "RWT-0008/C-01" }] }] };
const finish = { ...ready, command: "workflow.finish", completion, closeout_status: "complete" };
const fullFinish = { ...finish, mode: "workflow_package", task_id: task.task_id,
  remote_name: "origin", expected_identity_path: invocation.pinPath };
delete fullFinish.command;
delete fullFinish.package_execution;
assert.throws(() => workflowResult(fullFinish, "finish", identity));
assert.deepEqual(workflowResult(fullFinish, "finish", identity, { invocation }), identity);
for (const wrong of [{ task_id: "LWT-0002" }, { remote_name: "other" },
  { expected_identity_path: "/other/pin.json" }, { mode: "other" },
  { command: "workflow.ready" }, { expected_identity: { ...identity, worker_job_index: 1 } }]) {
  assert.throws(() => workflowResult({ ...fullFinish, ...wrong }, "finish", identity, { invocation }));
}
const fullPartial = { ...fullFinish, apply_status: "partial", closeout_status: "partial",
  next_action: { code: "restore_publication_mappings" },
  completion: { ...completion, outcome: "incomplete", entries: [] } };
assert.deepEqual(workflowResult(fullPartial, "finish", identity, { invocation }), identity);
assert.equal(packageAcceptance(fullPartial, identity, task.task_id).complete, false);
const fullUnknown = { ...fullPartial, completion: { ...completion, outcome: "unknown",
  patchset_id: null, submission_id: null, entries: [] } };
assert.equal(packageAcceptance(fullUnknown, identity, task.task_id), null);
assert.equal(packageAcceptance(finish, identity, task.task_id).complete, true);
assert.equal(packageAcceptance({ ...finish, apply_status: "partial" }, identity, task.task_id).complete, false);
assert.equal(packageAcceptance({ ...finish, completion: { ...completion, outcome: "incomplete", entries: [] } }, identity, task.task_id).complete, false);
assert.equal(packageAcceptance({ ...finish, completion: { ...completion, outcome: "unknown", patchset_id: null, submission_id: null, entries: [] } }, identity, task.task_id), null);
assert.throws(() => packageAcceptance({ ...finish, remote_task_id: "RWT-9999" }, identity, task.task_id));
assert.throws(() => packageAcceptance({ ...finish, completion: { ...completion, submission_id: "RWT-0009/L-01" } }, identity, task.task_id));
assert.throws(() => packageAcceptance({ ...finish, completion: { ...completion, entries: [] } }, identity, task.task_id));
assert.throws(() => packageAcceptance(finish, { ...identity, package_hash: "cd".repeat(32) }, task.task_id));
const pending = { ...finish, task_id: task.task_id, apply_status: "partial", closeout_status: "partial",
  next_action: { code: "restore_publication_mappings" }, remote_task_id: "RWT-0008",
  patchset_id: completion.patchset_id, submission_id: completion.submission_id,
  completion: { ...completion, outcome: "incomplete", entries: [] } };
assert.deepEqual(pendingPackageAcceptance(pending, identity, task.task_id), pending.completion);
for (const wrong of [
  { ...pending, completion: { ...pending.completion, outcome: "unknown", patchset_id: null, submission_id: null } },
  { ...pending, completion: { ...pending.completion, entries: completion.entries } },
  { ...pending, completion: { ...pending.completion, expected_identity: { ...identity, worker_job_index: 9 } } },
  { ...pending, task_id: "LWT-0002" },
  { ...pending, remote_task_id: "RWT-0009" },
  { ...pending, closeout_status: "complete" },
  { ...pending, next_action: { code: "done" } },
]) assert.throws(() => pendingPackageAcceptance(wrong, identity, task.task_id));
console.log("release native workflow contract tests passed");
