// Consume native workflow results. These checks add no workflow authority.
const SNAPSHOT = /^SNP-[0-9A-F]{12}$/;
const TASK = /^[A-Z]+T-[0-9]+$/;
const IDENTITY_KEYS = ["repository_index", "package_hash", "worker_job_index", "base_snapshot_id", "revision_snapshot_id"];

export function originalLocalSnapshot(task, taskId, expected = null) {
  const receipt = task?.local_finish;
  if (task?.task_id !== taskId || task.status !== "completed" ||
      receipt?.task_id !== taskId || receipt.status !== "succeeded" ||
      typeof receipt.target_line !== "string" || !receipt.target_line ||
      !SNAPSHOT.test(receipt.landed_snapshot_id ?? "")) {
    throw new Error(`${taskId}: original successful Local finish receipt is unavailable`);
  }
  if (expected !== null && expected !== receipt.landed_snapshot_id) {
    throw new Error(`${taskId}: original Local finish conflicts with the selected Snapshot`);
  }
  return receipt.landed_snapshot_id;
}

export function packageIdentity(value) {
  const index = n => Number.isSafeInteger(n) && n >= 0 && n <= 0xffffffff;
  if (!value || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...IDENTITY_KEYS].sort()) ||
      !index(value.repository_index) || !index(value.worker_job_index) ||
      !/^[0-9a-f]{64}$/.test(value.package_hash ?? "") ||
      !SNAPSHOT.test(value.base_snapshot_id ?? "") || !SNAPSHOT.test(value.revision_snapshot_id ?? "")) {
    throw new Error("Native package identity must contain the exact Repository/hash/Job/base/revision tuple");
  }
  return Object.fromEntries(IDENTITY_KEYS.map(key => [key, value[key]]));
}

export function samePackage(actual, expected) {
  return JSON.stringify(packageIdentity(actual)) === JSON.stringify(packageIdentity(expected));
}

export function repreparedPackageIdentity(previous, nextIdentity) {
  const before = packageIdentity(previous), after = packageIdentity(nextIdentity);
  for (const key of IDENTITY_KEYS.filter(key => key !== "worker_job_index")) {
    if (before[key] !== after[key]) throw new Error("Reprepare changed the original sealed package/Repository/base/revision");
  }
  // The verified archive hash includes its sealed manifest and author mode.
  if (before.worker_job_index >= after.worker_job_index) throw new Error("Reprepare must select a new native Job");
  return after;
}

export function workflowResult(value, command, expected = null, { readonly = false, invocation = null } = {}) {
  const compact = value?.command === `workflow.${command}`;
  const full = (command === "ready" || command === "finish") && value?.mode === "workflow_package" &&
    !Object.hasOwn(value, "command") && invocation;
  if (value?.contract !== "ait-agent-action/v1" || !(compact || full) ||
      typeof value.next_action?.code !== "string" || !value.next_action.code ||
      !(typeof value.apply_status === "string" && value.apply_status ||
        readonly && (!Object.hasOwn(value, "apply_status") || value.apply_status === null))) {
    throw new Error(`Invalid native workflow.${command} result`);
  }
  const identity = packageIdentity(value.expected_identity);
  if (invocation) {
    if (value.task_id !== invocation.task || value.remote_name !== invocation.remote ||
        full && value.expected_identity_path !== invocation.pinPath ||
        Object.hasOwn(value, "expected_identity_path") && value.expected_identity_path !== invocation.pinPath) {
      throw new Error("Native workflow capture belongs to another invocation or selector path");
    }
    const original = packageIdentity(invocation.expectedIdentity);
    if (!samePackage(identity, original)) {
      if (command !== "ready") throw new Error("Native finish changed the original package/Job identity");
      repreparedPackageIdentity(original, identity);
    }
  }
  if (expected && !samePackage(identity, expected)) throw new Error("Native workflow changed the selected package identity");
  if (value.package_execution && !samePackage(value.package_execution.expected_identity, identity)) {
    throw new Error("Native execution belongs to another package/Job");
  }
  return identity;
}

export function packageReadiness(value) {
  const execution = value.package_execution;
  return value.apply_status === "done" && value.next_action.code === "done" &&
    execution?.ci_state === "passed" && execution.availability === "available" && execution.recoverable === true;
}

export function packageAcceptance(value, identity, sourceTask) {
  const completion = value.completion;
  if (!completion) return null;
  if (!samePackage(completion.expected_identity, identity) ||
      !["confirmed", "incomplete", "unknown"].includes(completion.outcome) || !Array.isArray(completion.entries)) {
    throw new Error("Native completion does not match the selected package");
  }
  const paired = typeof completion.patchset_id === "string" && typeof completion.submission_id === "string";
  if (!paired) {
    if (completion.patchset_id !== null || completion.submission_id !== null ||
        completion.outcome === "confirmed" || completion.entries.length !== 0) {
      throw new Error("Native completion has inconsistent formal acceptance evidence");
    }
    return null;
  }
  const patchset = /^([A-Z]+T-[0-9]+)\/P-(0[1-9]|[1-5][0-9]|6[0-4])$/.exec(completion.patchset_id);
  const submission = /^([A-Z]+T-[0-9]+)\/L-(0[1-9]|[1-5][0-9]|6[0-4])$/.exec(completion.submission_id);
  if (completion.outcome === "unknown" || !patchset || !submission || patchset[1] !== submission[1]) {
    throw new Error("Native completion formal references have conflicting owners");
  }
  if (completion.outcome === "incomplete") {
    if (completion.entries.length !== 0) throw new Error("Incomplete native completion cannot supply inferred mappings");
    return { completion, complete: false };
  }
  const local = new Set(), remote = new Set();
  for (const row of completion.entries) {
    if (!TASK.test(row.local_task_id ?? "") || !TASK.test(row.remote_task_id ?? "") ||
        local.has(row.local_task_id) || remote.has(row.remote_task_id) || !Array.isArray(row.changes)) {
      throw new Error("Native completion has invalid or duplicate Task mappings");
    }
    local.add(row.local_task_id); remote.add(row.remote_task_id);
    const changes = new Set(), remoteChanges = new Set();
    for (const change of row.changes) {
      const owned = (ref, owner) => typeof ref === "string" && ref.startsWith(`${owner}/`) &&
        /^\/C-(0[1-9]|[1-5][0-9]|6[0-4])$/.test(ref.slice(owner.length));
      if (!owned(change.local_change_ref, row.local_task_id) || !owned(change.remote_change_ref, row.remote_task_id) ||
          changes.has(change.local_change_ref) || remoteChanges.has(change.remote_change_ref)) {
        throw new Error("Native completion has conflicting Change mappings");
      }
      changes.add(change.local_change_ref); remoteChanges.add(change.remote_change_ref);
    }
  }
  const final = completion.entries.at(-1);
  if (final?.local_task_id !== sourceTask || final.remote_task_id !== patchset[1] ||
      (value.remote_task_id !== undefined && value.remote_task_id !== final.remote_task_id) ||
      (value.patchset_id !== undefined && value.patchset_id !== completion.patchset_id) ||
      (value.submission_id !== undefined && value.submission_id !== completion.submission_id)) {
    throw new Error("Native acceptance does not map the original final source Task");
  }
  return { completion, remote_task_id: final.remote_task_id,
    complete: value.apply_status === "done" && value.next_action.code === "done" && value.closeout_status === "complete" };
}

export function pendingPackageAcceptance(value, identity, sourceTask) {
  const accepted = packageAcceptance(value, identity, sourceTask);
  if (value.task_id !== sourceTask || !accepted || accepted.complete || accepted.completion.outcome !== "incomplete" ||
      accepted.completion.entries.length !== 0 || value.apply_status !== "partial" ||
      value.closeout_status !== "partial" || value.next_action?.code !== "restore_publication_mappings") {
    throw new Error("Native accepted retry must retain honest pending mappings and cleanup");
  }
  const completion = accepted.completion;
  const owner = completion.patchset_id.split("/")[0];
  if (value.remote_task_id !== undefined && value.remote_task_id !== owner ||
      value.patchset_id !== undefined && value.patchset_id !== completion.patchset_id ||
      value.submission_id !== undefined && value.submission_id !== completion.submission_id) {
    throw new Error("Native accepted retry changed the original formal references");
  }
  return completion;
}
