#!/usr/bin/env node
import assert from "node:assert/strict";
import { validateWorkflowRequest, currentWorkflowReceipt, currentReviewedForkRejection } from "./release_web_workflow.mjs";
const clone = value => structuredClone(value);
const capture = { contract: "ait.web.personal.workflow-mutation-request.v2", operation: "snapshot_create", task_id: "RWTT-0001",
  scope: "remote", remote: "selected-B", change_ref: null, message: "Capture exact Task" };
validateWorkflowRequest(capture);
for (const invalid of [{ ...capture, path: "/foreign/worktree" }, { ...capture, remote: null },
  { ...capture, change_ref: "RWTT-0002/C-01" }, { ...capture, contract: "legacy" }]) assert.throws(() => validateWorkflowRequest(invalid));
const native = { kind: "snapshot", task_id: capture.task_id, change_ref: null, snapshot_id: "SNP-111111111111", line_name: "feature/rwtt-0001",
  capture_binding: { contract: "ait.snapshot_capture_binding.v2", repository_index: 23, task_id: capture.task_id,
    change_ref: null, snapshot_id: "SNP-111111111111", line_name: "feature/rwtt-0001" } };
const response = { status: 201, body: { contract: "ait.web.personal.workflow-mutation-receipt.v2", ok: true, operation: "snapshot_create",
  target: capture.task_id, scope: "remote", remote: "selected-B", authority: native, summary: "Native capture",
  inspect_path: "/tasks/RWTT-0001?scope=remote&remote=selected-B" } };
currentWorkflowReceipt(response, capture, 23);
for (const mutate of [value => { value.body.remote = "default-A"; }, value => { value.body.authority.capture_binding.repository_index = 24; },
  value => { value.body.authority.capture_binding = null; }, value => { value.body.inspect_path = "/tasks/RWTT-0001?scope=remote&remote=default-A"; }]) {
  const invalid = clone(response); mutate(invalid); assert.throws(() => currentWorkflowReceipt(invalid, capture, 23));
}
const identity = { repository_index: 23, package_hash: "a".repeat(64), worker_job_index: 0,
  base_snapshot_id: "SNP-111111111111", revision_snapshot_id: "SNP-222222222222" };
const ready = { contract: capture.contract, operation: "workflow_ready", task_id: "LWTT-0001", scope: "remote", remote: "selected-B",
  candidate_kind: "package", apply: false, expected_identity: null };
const preview = { status: 200, body: { contract: response.body.contract, ok: true, operation: ready.operation, target: ready.task_id,
  scope: ready.scope, remote: ready.remote, inspect_path: "/tasks/LWTT-0001?scope=local", summary: "Native preview",
  authority: { kind: "workflow", task_id: ready.task_id, candidate_kind: "package", expected_identity: null, apply_status: null,
    closeout_status: null, next_action: { code: "package_ready" }, package_execution: null, completion: null, formal_finish: null } } };
assert.equal(currentWorkflowReceipt(preview, ready, 23).authority.apply_status, null);
assert.throws(() => currentWorkflowReceipt(preview, { ...ready, apply: true }, 23));
const appliedRequest = { ...ready, apply: true, expected_identity: identity };
const applied = clone(preview);
Object.assign(applied.body.authority, { expected_identity: identity, apply_status: "done", next_action: { code: "done" },
  package_execution: { expected_identity: identity, ci_state: "passed", availability: "available", recoverable: true } });
currentWorkflowReceipt(applied, appliedRequest, 23); // Ready phase only; no finish/mapping is inferred.
const pending = clone(applied); pending.body.authority.apply_status = "waiting";
pending.body.authority.next_action.code = "ci_running"; pending.body.authority.package_execution.ci_state = "running";
assert.throws(() => currentWorkflowReceipt(pending, appliedRequest, 23));
pending.status = 202; currentWorkflowReceipt(pending, appliedRequest, 23);
const replacement = clone(applied); replacement.body.authority.expected_identity.worker_job_index = 1;
currentWorkflowReceipt(replacement, appliedRequest, 23);
assert.throws(() => currentWorkflowReceipt(replacement, { ...appliedRequest, apply: false }, 23));
const wrongFinish = clone(replacement); wrongFinish.body.operation = "workflow_finish";
assert.throws(() => currentWorkflowReceipt(wrongFinish, { ...appliedRequest, operation: "workflow_finish", confirmation: identity.package_hash }, 23));
const changedPin = clone(applied); changedPin.body.authority.expected_identity.base_snapshot_id = "SNP-999999999999";
assert.throws(() => currentWorkflowReceipt(changedPin, appliedRequest, 23));
const missingMappings = clone(applied);
missingMappings.body.authority.completion = { outcome: "confirmed", expected_identity: identity,
  patchset_id: "RWTT-0001/P-01", submission_id: "RWTT-0001/L-01", entries: [] };
assert.throws(() => currentWorkflowReceipt(missingMappings, appliedRequest, 23));
assert.throws(() => validateWorkflowRequest({ ...ready, operation: "workflow_finish", confirmation: identity.package_hash }));
assert.throws(() => validateWorkflowRequest({ ...ready, candidate_kind: "patchset", expected_identity: { ...identity } }));
const rejectedForkRequest = { contract: capture.contract, operation: "change_create", task_id: "LWTT-1234", scope: "local", remote: null,
  title: "Correct the reviewed implementation", fork_snapshot_id: "SNP-000000000000" };
const rejectedFork = { status: 409, location: undefined, body: { contract: "ait.web.personal.workflow-mutation-error.v2", ok: false,
  operation: "change_create", target: rejectedForkRequest.task_id, task_id: rejectedForkRequest.task_id, scope: "local", remote: null,
  code: "mutation_precondition_failed", message: "Reviewed fork must equal the exact frozen Task head; no latest-Snapshot fallback",
  automatic_retry: false, reconciliation: null, expected_identity: null } };
currentReviewedForkRejection(rejectedFork, rejectedForkRequest, "SNP-111111111111");
assert.throws(() => currentWorkflowReceipt(rejectedFork, rejectedForkRequest, 23));
for (const mutate of [
  value => { value.status = 200; }, value => { value.location = "/tasks/LWTT-1234?scope=local"; },
  value => { value.body.contract = "ait.web.personal.workflow-mutation-error.v1"; }, value => { value.body.ok = true; },
  value => { value.body.operation = "snapshot_create"; }, value => { value.body.target = "LWTT-9999"; },
  value => { value.body.task_id = "LWTT-9999"; }, value => { value.body.scope = "remote"; }, value => { value.body.remote = "origin"; },
  value => { value.body.code = "mutation_gate_unavailable"; }, value => { value.body.message = "Another precondition failed"; },
  value => { value.body.automatic_retry = true; }, value => { value.body.reconciliation = { possible_effect: true }; },
  value => { value.body.expected_identity = {}; }, value => { delete value.body.reconciliation; },
]) { const invalid = clone(rejectedFork); mutate(invalid); assert.throws(() => currentReviewedForkRejection(invalid, rejectedForkRequest, "SNP-111111111111")); }
for (const reviewed of [undefined, "SNP-000000000000", "SNP-INVALID", "SNP-111111111111\n"]) {
  assert.throws(() => currentReviewedForkRejection(rejectedFork, rejectedForkRequest, reviewed));
}
console.log("Native current workflow authority and phase boundary checks passed");
