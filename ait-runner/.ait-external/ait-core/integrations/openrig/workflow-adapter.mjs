// Versioned integration only: AIT owns readiness, review, policy and closeout.
export const HANDOFF_CONTRACT = 'ait-openrig-handoff/v1';
const identityFields = ['repository_index', 'patchset_id', 'revision_snapshot_id', 'base_snapshot_id'];

export class WorkflowBlocked extends Error {
  constructor(code, detail, evidence) {
    super(detail); this.code = code; this.evidence = evidence;
  }
}

function canonicalPatchset(value) {
  // Public TASK/P-## and canonical TASK/C-01/P-## identify the same first Change.
  return String(value ?? '').replace(/\/C-01(?=\/P-)/, '');
}

export function assertIdentity(expected, actual) {
  if (!expected || !actual || !Number.isInteger(expected.repository_index) || expected.repository_index < 0 || expected.repository_index > 0xffffffff) {
    throw new WorkflowBlocked('invalid_identity', 'A complete numeric Repository candidate identity is required');
  }
  for (const key of identityFields) {
    const left = key === 'patchset_id' ? canonicalPatchset(expected[key]) : expected[key];
    const right = key === 'patchset_id' ? canonicalPatchset(actual[key]) : actual[key];
    if (left === undefined || left === null || left === '' || left !== right) {
      throw new WorkflowBlocked('candidate_changed', `Exact candidate ${key} differs`, actual);
    }
  }
  if (actual.invalidated || actual.withdrawn || actual.dirty || actual.workspace_matches_patchset === false || actual.base_is_fresh === false) {
    throw new WorkflowBlocked('candidate_not_ready', 'Dirty, stale, invalidated or withdrawn sealed candidate', actual);
  }
}

export function assertCIJobIdentity(ci, expected) {
  assertIdentity(expected, ci);
  const job = ci.latest_job;
  if (job && job.repository_index !== expected.repository_index) {
    throw new WorkflowBlocked('wrong_ci_job', 'CI job belongs to another Repository', ci);
  }
  if (job && (!Number.isInteger(ci.patchset_index) || job.patchset_index !== ci.patchset_index)) {
    throw new WorkflowBlocked('wrong_ci_job', 'CI job belongs to another Patchset', ci);
  }
  if (job && (!Number.isSafeInteger(ci.ci_run_seq) || ci.ci_run_seq <= 0 || job.patchset_ci?.ci_run_seq !== ci.ci_run_seq || job.patchset_ci?.ci_worker_job_index !== job.worker_job_index || !Number.isInteger(job.worker_job_index))) {
    throw new WorkflowBlocked('ci_run_changed', 'CI job/run sequence does not match the exact Patchset CI receipt', ci);
  }
}

export function classifyCI(ci, expected) {
  assertCIJobIdentity(ci, expected);
  const job = ci.latest_job;
  // Complete native job evidence outranks a cached embedded pass or next action.
  if (job?.retry_pending || ['queued', 'running'].includes(job?.state)) return 'pending';
  if (job?.state === 'failed' || ci.blocking_failure_count > 0 || ['fail', 'failed', 'hard_fail', 'soft_fail'].includes(ci.tests_status) || ci.overall_status === 'fail') return 'failed';
  if (ci.native_next_action === 'waiting_for_ci') return 'pending';
  const completed = job?.state === 'succeeded' || (ci.ci_run_seq > 0 && ci.ci_completed_at_s > 0);
  if (completed && ci.overall_status === 'pass' && ci.tests_status === 'pass' && ci.has_runnable_evidence === true && ci.blocking_failure_count === 0) return 'passed';
  if (ci.native_next_action === 'run_patchset_ci' && !job && !ci.ci_run_seq && (!ci.tests_status || ci.tests_status === 'unknown')) return 'needs_run';
  throw new WorkflowBlocked('ci_unknown', 'No complete exact-candidate runnable CI evidence', ci);
}

function outcome(payload) {
  if (payload.apply_status === 'done' && payload.next_action?.code === 'done') return 'done';
  if (payload.apply_status === 'waiting_for_ci' || payload.next_action?.code === 'waiting_for_ci') return 'waiting';
  return 'needs_action';
}

// Ports write raw evidence and use native commands. No model, runner lifecycle,
// review generation, approval changes, or alternate integration owner is here.
export async function deliverSealedCandidate(request, ports, { maxPolls = 120, maxReadyCalls = 3 } = {}) {
  if (request.contract !== HANDOFF_CONTRACT || !request.remote || !request.local_task_id || !request.owner) {
    throw new WorkflowBlocked('invalid_handoff', 'Versioned handoff, named remote, Task and owner are required');
  }
  const expected = request.identity;
  assertIdentity(expected, expected);
  const evidence = [];
  const record = async (phase, value) => { evidence.push(await ports.recordEvidence(phase, value)); return value; };
  const check = async () => assertIdentity(expected, await record('identity', await ports.readIdentity()));
  const checkOwner = async () => {
    if (typeof ports.readOwner !== 'function' || await record('owner', await ports.readOwner()) !== request.owner) throw new WorkflowBlocked('owner_changed', 'The executing OpenRig owner differs from the assigned reviewer');
  };
  await checkOwner();
  await check();
  // A ready invocation may finish, or return pending after enqueue. CI pass is
  // followed by another native ready invocation to record its Attestation.
  let readyDone = false;
  for (let calls = 0; calls < maxReadyCalls; calls++) {
    const initialCI = await record('ci-before-ready', await ports.readCI());
    const initialState = classifyCI(initialCI, expected);
    if (initialState === 'failed') throw new WorkflowBlocked('ci_failed', 'Terminal CI failure; no automatic rerun', initialCI);
    if (initialState === 'pending') {
      let passed = false;
      for (let poll = 0; poll < maxPolls; poll++) {
        await ports.wait();
        const ci = await record('ci', await ports.readCI());
        const state = classifyCI(ci, expected);
        if (state === 'failed') throw new WorkflowBlocked('ci_failed', 'Terminal CI failure; no automatic rerun', ci);
        if (state === 'passed') { passed = true; break; }
      }
      if (!passed) return { contract: HANDOFF_CONTRACT, completed: false, phase: 'waiting_for_ci', identity: expected, evidence };
    }
    await check();
    const ready = await record('ready', await ports.ready(request));
    assertIdentity(expected, ready.identity);
    if (outcome(ready) === 'done') { readyDone = true; break; }
    if (outcome(ready) !== 'waiting') return { contract: HANDOFF_CONTRACT, completed: false, phase: 'ready', native: ready, evidence };
    let passed = false;
    for (let poll = 0; poll < maxPolls; poll++) {
      const ci = await record('ci', await ports.readCI());
      const state = classifyCI(ci, expected);
      if (state === 'failed') throw new WorkflowBlocked('ci_failed', 'Terminal CI failure; keep native failure evidence', ci);
      if (state === 'passed') { passed = true; break; }
      await ports.wait();
    }
    if (!passed) return { contract: HANDOFF_CONTRACT, completed: false, phase: 'waiting_for_ci', identity: expected, evidence };
  }
  if (!readyDone) throw new WorkflowBlocked('ready_not_done', 'Bounded ready continuation did not finish');
  const review = request.review;
  if (!review || review.accepted !== true || review.owner !== request.owner || !review.summary?.trim() || !review.evidence) {
    throw new WorkflowBlocked('review_required', 'An assigned independent reviewer must supply the existing real verdict');
  }
  assertIdentity(expected, review.identity);
  if (request.authors?.length !== 3 || new Set(request.authors).size !== 3 || request.authors.includes(review.owner)) throw new WorkflowBlocked('independent_review_required', 'The three authors and assigned independent reviewer must remain distinct');
  await checkOwner();
  await check();
  const finish = await record('finish', await ports.finish(request));
  assertIdentity(expected, finish.identity);
  if (finish.apply_status !== 'done' || !['complete', 'complete_unbound', 'execution_complete_plan_separate'].includes(finish.closeout?.status) || finish.closeout?.task_status !== 'completed') {
    return { contract: HANDOFF_CONTRACT, completed: false, phase: 'finish', native: finish, evidence };
  }
  const proof = await record('verification', await ports.verifyDelivery(request, finish));
  assertIdentity(expected, proof.identity);
  if (proof.remote_main !== expected.revision_snapshot_id || proof.ci !== 'pass' || proof.policy !== 'pass' || proof.review !== 'pass' || proof.tasks_completed !== true) {
    throw new WorkflowBlocked('delivery_unverified', 'Native delivery evidence did not establish completion', proof);
  }
  return { contract: HANDOFF_CONTRACT, completed: true, phase: 'finished', identity: expected, remote: request.remote, task_id: request.remote_task_id, remote_main: proof.remote_main, closeout: finish.closeout, evidence };
}
