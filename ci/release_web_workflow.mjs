// Native's consumer checks for Personal's existing current workflow contracts.
// A verified receipt preserves phase evidence; it never grants release approval.
import assert from "node:assert/strict";
import { explicitAuthority, taskStartInspectLocation } from "./release_web_controls.mjs";
import { packageIdentity, packageAcceptance, packageReadiness, samePackage, repreparedPackageIdentity } from "./release_native_workflow.mjs";

const snapshot = value => /^SNP-[0-9A-F]{12}$/.test(value ?? "");
const ownedChange = (value, task) => typeof value === "string" && value.startsWith(`${task}/`) && /^C-(0[1-9]|[1-5][0-9]|6[0-4])$/.test(value.slice(task.length + 1));
const fields = {
  snapshot_create: [["change_ref", "message"], []], change_create: [["title", "fork_snapshot_id"], []],
  task_finish: [["confirmation"], ["message"]],
  workflow_ready: [["candidate_kind", "apply", "expected_identity"], ["summary"]],
  workflow_finish: [["candidate_kind", "apply", "expected_identity", "confirmation"], ["reviewer", "review"]],
};

function pin(value, kind, task) {
  if (kind === "package") return packageIdentity(value);
  assert.equal(kind, "patchset");
  assert.deepEqual(Object.keys(value ?? {}).sort(), ["repository_index", "patchset_id", "base_snapshot_id", "revision_snapshot_id"].sort());
  assert(Number.isSafeInteger(value.repository_index) && value.repository_index >= 0 && value.repository_index <= 0xffffffff);
  assert(snapshot(value.base_snapshot_id) && snapshot(value.revision_snapshot_id));
  assert(value.patchset_id.startsWith(`${task}/`) && new RegExp(`^${task}/(?:C-(?:0[1-9]|[1-5][0-9]|6[0-4])/)?P-(?:0[1-9]|[1-5][0-9]|6[0-4])$`).test(value.patchset_id));
  return value;
}

export function validateWorkflowRequest(request) {
  assert.equal(request?.contract, "ait.web.personal.workflow-mutation-request.v2");
  assert(/^[A-Z]+T-[0-9]+$/.test(request.task_id ?? ""));
  assert(Object.hasOwn(request, "remote"));
  const authority = explicitAuthority(request.scope, request.remote ?? undefined);
  assert.equal(request.remote, authority.remote);
  const spec = fields[request.operation]; assert(spec, "unsupported current workflow operation");
  const required = ["contract", "operation", "task_id", "scope", "remote", ...spec[0]];
  assert(required.every(key => Object.hasOwn(request, key)));
  assert(Object.keys(request).every(key => [...required, ...spec[1]].includes(key)), "unknown workflow input");
  for (const key of ["message", "title", "summary"]) if (Object.hasOwn(request, key))
    assert(typeof request[key] === "string" && request[key].trim() && [...request[key]].length <= 2000);
  if (request.operation === "snapshot_create") assert(request.change_ref === null || ownedChange(request.change_ref, request.task_id));
  if (request.operation === "change_create") assert(snapshot(request.fork_snapshot_id));
  if (request.operation === "task_finish") { assert.equal(authority.scope, "local"); assert.equal(request.confirmation, request.task_id); }
  if (request.operation.startsWith("workflow_")) {
    assert.equal(authority.scope, "remote"); assert.equal(typeof request.apply, "boolean");
    assert(["package", "patchset"].includes(request.candidate_kind));
    if (request.expected_identity !== null) pin(request.expected_identity, request.candidate_kind, request.task_id);
    if (request.operation === "workflow_finish") {
      assert(request.expected_identity, "finish requires the original exact pin");
      assert.equal(request.confirmation, request.candidate_kind === "package" ? request.expected_identity.package_hash : request.expected_identity.patchset_id);
    }
    if (request.reviewer !== undefined) assert(typeof request.reviewer === "string" && request.reviewer.trim() && Buffer.byteLength(request.reviewer) <= 255);
    if (request.review !== undefined) {
      const sections = ["reviewed_files", "findings", "risks", "tests", "recommendation"];
      assert.deepEqual(Object.keys(request.review).sort(), sections.sort());
      assert(Object.values(request.review).every(text => typeof text === "string" && text.trim() && [...text].length <= 4000));
    }
  }
  return authority;
}

export function currentWorkflowReceipt(http, request, repositoryIndex) {
  const selected = validateWorkflowRequest(request), body = http.body, native = body?.authority;
  assert([200, 201, 202].includes(http.status), "workflow failed or is ambiguous; do not retry");
  assert.equal(body?.contract, "ait.web.personal.workflow-mutation-receipt.v2");
  assert.equal(body.ok, true); assert.equal(body.operation, request.operation);
  assert.equal(body.scope, selected.scope); assert.equal(body.remote, selected.remote);
  assert.equal(body.target, request.operation === "snapshot_create" ? request.change_ref ?? request.task_id : request.task_id);
  assert.equal(native?.task_id, request.task_id);
  if (request.operation === "snapshot_create") {
    assert.equal(http.status, 201); assert.equal(native.kind, "snapshot"); assert(snapshot(native.snapshot_id));
    assert.equal(native.change_ref, request.change_ref); assert.equal(typeof native.line_name, "string");
    if (selected.scope === "local") assert.equal(native.capture_binding, null);
    else assert.deepEqual(native.capture_binding, { contract: "ait.snapshot_capture_binding.v2", repository_index: repositoryIndex,
      task_id: request.task_id, change_ref: request.change_ref, snapshot_id: native.snapshot_id, line_name: native.line_name });
  } else if (request.operation === "change_create") {
    assert.equal(http.status, 201); assert.equal(native.kind, "change"); assert(ownedChange(native.change_ref, request.task_id));
    assert.equal(native.fork_snapshot_id, request.fork_snapshot_id);
  } else if (request.operation === "task_finish") {
    assert.equal(http.status, 200); assert.equal(native.kind, "task_finish"); assert.equal(native.task_state, "completed");
    assert(snapshot(native.landed_snapshot_id)); assert.equal(native.patchset_ref, null);
  } else {
    assert.equal(native.kind, "workflow"); assert.equal(native.candidate_kind, request.candidate_kind);
    for (const key of ["expected_identity", "apply_status", "closeout_status", "next_action", "package_execution", "completion", "formal_finish"]) assert(Object.hasOwn(native, key));
    assert(typeof native.next_action?.code === "string" && native.next_action.code);
    assert(native.apply_status === null ? !request.apply : typeof native.apply_status === "string" && native.apply_status);
    if (native.expected_identity !== null) pin(native.expected_identity, request.candidate_kind, request.task_id);
    if (request.expected_identity !== null) {
      if (request.operation === "workflow_ready" && request.apply && request.candidate_kind === "package" &&
          !samePackage(native.expected_identity, request.expected_identity)) repreparedPackageIdentity(request.expected_identity, native.expected_identity);
      else assert.deepEqual(native.expected_identity, request.expected_identity);
    }
    if (native.package_execution !== null) assert.deepEqual(native.package_execution.expected_identity, native.expected_identity);
    if (request.candidate_kind === "package") {
      assert.equal(native.formal_finish, null);
      const acceptance = native.completion === null ? null : packageAcceptance(native, native.expected_identity, request.task_id);
      if (request.apply) assert.equal(http.status,
        (request.operation === "workflow_ready" ? packageReadiness(native) : acceptance?.complete === true) ? 200 : 202,
        "pending, ready and completed finish phases must retain their distinct HTTP status");
    } else {
      assert.equal(native.package_execution, null); assert.equal(native.completion, null);
      if (native.formal_finish !== null) {
        assert.equal(native.formal_finish.kind, "task_finish"); assert.equal(native.formal_finish.task_id, request.task_id);
        assert.equal(native.formal_finish.patchset_ref, native.expected_identity?.patchset_id);
        assert(snapshot(native.formal_finish.landed_snapshot_id));
      }
    }
    if (!request.apply) assert.equal(http.status, 200, "readonly observation is a distinct phase");
    assert.notEqual(http.status, 201, "ready/finish cannot be classified as Task creation");
  }
  // Package results inspect their completed Local source, while their receipt
  // authority and pin retain the selected Remote destination.
  const inspectAuthority = request.candidate_kind === "package" ? explicitAuthority("local") : selected;
  taskStartInspectLocation(body.inspect_path, request.task_id, inspectAuthority);
  return body;
}

export function currentReviewedForkRejection(http, request, reviewedSnapshot) {
  const selected = validateWorkflowRequest(request), body = http.body;
  assert.equal(request.operation, "change_create");
  assert(snapshot(reviewedSnapshot) && reviewedSnapshot.length === 16);
  assert.notEqual(request.fork_snapshot_id, reviewedSnapshot, "negative fork must differ from the actual reviewed Task head");
  assert.equal(http.status, 409); assert.equal(http.location ?? null, null);
  assert.equal(body?.contract, "ait.web.personal.workflow-mutation-error.v2");
  assert.equal(body.ok, false); assert.equal(body.operation, request.operation);
  assert.equal(body.target, request.task_id); assert.equal(body.task_id, request.task_id);
  assert.equal(body.scope, selected.scope); assert.equal(body.remote, selected.remote);
  assert.equal(body.code, "mutation_precondition_failed"); assert.equal(body.automatic_retry, false);
  assert.equal(body.reconciliation, null, "a possible mutation effect is not a rejected fork");
  assert.equal(body.expected_identity, null);
  assert(typeof body.message === "string" && body.message.includes(
    "Reviewed fork must equal the exact frozen Task head; no latest-Snapshot fallback"), "another precondition failure cannot prove the reviewed-fork guard");
  return body;
}
