import test from 'node:test';
import assert from 'node:assert/strict';
import { deliverSealedCandidate, classifyCI, HANDOFF_CONTRACT } from './workflow-adapter.mjs';
import { DirectedContextCache, hashContent, bootstrapPlan } from './context-cache.mjs';

const identity = { repository_index: 35, patchset_id: 'RT-0003/P-02', revision_snapshot_id: 'SNP-CANDIDATE', base_snapshot_id: 'SNP-BASE' };
const pass = () => ({ ...identity, ci_run_seq: 1, ci_completed_at_s: 10, latest_job: { repository_index: 35, patchset_index: 3, state: 'succeeded', worker_job_index: 8, patchset_ci: { ci_run_seq: 1, ci_worker_job_index: 8 } }, patchset_index: 3, overall_status: 'pass', tests_status: 'pass', has_runnable_evidence: true, blocking_failure_count: 0 });
const pending = () => ({ ...pass(), latest_job: { ...pass().latest_job, state: 'running' } });
const readyDone = () => ({ identity, apply_status: 'done', change: { status: 'review' }, next_action: { code: 'done' } });
const request = () => ({ contract: HANDOFF_CONTRACT, remote: 'owned-origin', owner: 'reviewer', authors: ['a', 'b', 'c'], local_task_id: 'LT-0003', remote_task_id: 'RT-0003', identity, review: { owner: 'reviewer', accepted: true, identity, summary: 'The real independently inspected exact-candidate review', evidence: 'local-review.json' } });
function fixture({ cis = [pass()], readies = [readyDone()], finish } = {}) {
  const events = []; let ciIndex = 0; let readyIndex = 0;
  const ports = {
    recordEvidence: async (phase, value) => { events.push(phase); return { phase, value }; },
    readIdentity: async () => identity,
    readOwner: async () => 'reviewer',
    readCI: async () => cis[Math.min(ciIndex++, cis.length - 1)],
    ready: async () => { events.push('ready-call'); return readies[Math.min(readyIndex++, readies.length - 1)]; },
    wait: async () => events.push('wait'),
    finish: async () => { events.push('finish-call'); return finish ?? { identity, apply_status: 'done', closeout: { status: 'execution_complete_plan_separate', task_status: 'completed' } }; },
    verifyDelivery: async () => ({ identity, remote_main: identity.revision_snapshot_id, ci: 'pass', policy: 'pass', review: 'pass', tasks_completed: true })
  };
  return { ports, events };
}

test('CI pass resumes the same ready to done before finish; no runner stop or enqueue port', async () => {
  const f = fixture({ cis: [pending(), pending(), pass(), pass()], readies: [{ identity, apply_status: 'waiting_for_ci', next_action: { code: 'run_patchset_ci' } }, readyDone()] });
  const result = await deliverSealedCandidate(request(), f.ports);
  assert.equal(result.completed, true);
  assert.equal(f.events.filter(x => x === 'ready-call').length, 2);
  assert.equal(f.events.filter(x => x === 'finish-call').length, 1);
  assert.ok(f.events.indexOf('wait') < f.events.indexOf('finish-call'));
  assert.deepEqual(result.identity, identity);
  assert.equal(result.remote, 'owned-origin');
});
test('ready done with change review is ready, not a failed change', async () => assert.equal((await deliverSealedCandidate(request(), fixture().ports)).completed, true));
test('exit-zero stopped never becomes success', async () => {
  const f = fixture({ finish: { identity, exit_code: 0, apply_status: 'stopped', next_action: { code: 'record_review' } } });
  const result = await deliverSealedCandidate(request(), f.ports);
  assert.equal(result.completed, false); assert.equal(result.native.next_action.code, 'record_review');
});
test('pending times out without finish, rerun or runner termination', async () => {
  const f = fixture({ cis: [pending()], readies: [{ identity, apply_status: 'waiting_for_ci' }] });
  const result = await deliverSealedCandidate(request(), f.ports, { maxPolls: 2 });
  assert.equal(result.phase, 'waiting_for_ci'); assert.ok(!f.events.includes('finish-call')); assert.equal(f.events.filter(x => x === 'ready-call').length, 0);
});
test('old pass with a newer running or retry-pending job remains waiting', () => {
  assert.equal(classifyCI(pending(), identity), 'pending');
  assert.equal(classifyCI({ ...pass(), latest_job: { ...pass().latest_job, retry_pending: true } }, identity), 'pending');
});
test('complete pending job outranks embedded ready and initial CI enqueue remains bounded', async () => {
  assert.equal(classifyCI({ ...pass(), native_next_action: 'waiting_for_ci' }, identity), 'pending');
  assert.equal(classifyCI({ ...pass(), latest_job: { ...pass().latest_job, state: 'queued' }, native_next_action: 'record_attestation' }, identity), 'pending');
  assert.equal(classifyCI({ ...pending(), native_next_action: 'done' }, identity), 'pending');
  assert.throws(() => classifyCI({ ...pass(), ci_run_seq: 2 }, identity), { code: 'ci_run_changed' });
  assert.equal(classifyCI({ ...identity, ci_run_seq: 1, ci_completed_at_s: 10, overall_status: 'pass', tests_status: 'pass', has_runnable_evidence: true, blocking_failure_count: 0, native_next_action: 'done' }, identity), 'passed');
  const f = fixture({ cis: [{ ...identity, ci_run_seq: 0, tests_status: 'unknown', native_next_action: 'run_patchset_ci' }, pending(), pass(), pass()], readies: [{ identity, apply_status: 'waiting_for_ci' }, readyDone()] });
  assert.equal((await deliverSealedCandidate(request(), f.ports)).completed, true);
  assert.equal(f.events.filter(x => x === 'ready-call').length, 2);
});
test('terminal CI fail stops before ready and cannot loop into automatic reruns', async () => {
  const f = fixture({ cis: [{ ...pass(), tests_status: 'fail' }] });
  await assert.rejects(deliverSealedCandidate(request(), f.ports), { code: 'ci_failed' }); assert.ok(!f.events.includes('ready-call'));
});
test('same names and Task/Patchset IDs in another numeric repo are rejected', async () => {
  const f = fixture(); f.ports.readIdentity = async () => ({ ...identity, repository_index: 36, repo_name: 'same-name' });
  await assert.rejects(deliverSealedCandidate(request(), f.ports), { code: 'candidate_changed' });
});
for (const field of ['dirty', 'invalidated', 'withdrawn']) test(`${field} sealed candidate cannot reuse review`, async () => {
  const f = fixture(); f.ports.readIdentity = async () => ({ ...identity, [field]: true });
  await assert.rejects(deliverSealedCandidate(request(), f.ports), { code: 'candidate_not_ready' });
});
test('stale or changed candidate invalidates the old review', async () => {
  const f = fixture(); f.ports.readIdentity = async () => ({ ...identity, base_is_fresh: false });
  await assert.rejects(deliverSealedCandidate(request(), f.ports), { code: 'candidate_not_ready' });
  const r = request(); r.review.identity = { ...identity, revision_snapshot_id: 'SNP-OTHER' };
  await assert.rejects(deliverSealedCandidate(r, fixture().ports), { code: 'candidate_changed' });
});
test('missing review and self review cannot create review pass or call finish', async () => {
  const f = fixture(); const r = request(); delete r.review;
  await assert.rejects(deliverSealedCandidate(r, f.ports), { code: 'review_required' }); assert.ok(!f.events.includes('finish-call'));
  const self = request(); self.authors.push('reviewer');
  await assert.rejects(deliverSealedCandidate(self, fixture().ports), { code: 'independent_review_required' });
});
test('a clean workspace pointing at a different candidate cannot reuse review', async () => {
  const f = fixture(); f.ports.readIdentity = async () => ({ ...identity, workspace_matches_patchset: false });
  await assert.rejects(deliverSealedCandidate(request(), f.ports), { code: 'candidate_not_ready' });
});
test('executing reviewer identity and three author identities cannot change or disappear', async () => {
  const f = fixture(); f.ports.readOwner = async () => 'another-reviewer';
  await assert.rejects(deliverSealedCandidate(request(), f.ports), { code: 'owner_changed' });
  assert.ok(!f.events.includes('ready-call'));
  const missing = request(); delete missing.authors;
  await assert.rejects(deliverSealedCandidate(missing, fixture().ports), { code: 'independent_review_required' });
});
test('required human approval and partial closeout retain native next action', async () => {
  const r = await deliverSealedCandidate(request(), fixture({ finish: { identity, apply_status: 'stopped', next_action: { code: 'record_review', detail: 'Required human approval' } } }).ports);
  assert.equal(r.completed, false);
  const partial = await deliverSealedCandidate(request(), fixture({ finish: { identity, apply_status: 'done', closeout: { status: 'partial', task_status: 'completed' }, next_action: { code: 'resume_task_land_closeout' } } }).ports);
  assert.equal(partial.completed, false); assert.equal(partial.native.next_action.code, 'resume_task_land_closeout');
});
test('context cache verifies content and reloads only on version/ref/hash/scope or explicit restore', async () => {
  const cache = new DirectedContextCache(); const text = 'bounded section'; let reads = 0;
  const meta = { version: '0.6.7', ref: 'skills/openrig-user/SKILL.md#coordination-primitives-when-to-use-which', contentHash: hashContent(text), scope: 'worker-a' };
  const load = async () => { reads++; return text; };
  assert.equal((await cache.read(meta, load)).cached, false); assert.equal((await cache.read(meta, load)).cached, true); assert.equal(reads, 1);
  await cache.read({ ...meta, version: '0.6.8' }, load); cache.invalidate('compaction'); await cache.read(meta, load); assert.equal(reads, 3);
  await assert.rejects(cache.read({ ...meta, scope: 'new' }, async () => 'changed'), /Context changed/);
});
test('bootstrap uses existing exact paths and queue claim, never broad context JSON', () => {
  const plan = bootstrapPlan({ role: 'worker', queueId: 'qitem-assigned', guidancePath: 'AGENTS.md', sprintPath: 'docs/sprints/own.md', contextRefs: ['skills/openrig-user/SKILL.md#coordination-primitives-when-to-use-which'] });
  assert.deepEqual(plan.commands[0], ['rig', 'whoami', '--json']); assert.ok(!JSON.stringify(plan).includes('context","list'));
  assert.throws(() => bootstrapPlan({ role: 'reviewer', queueId: 'q', guidancePath: 'a', sprintPath: 'b', contextRefs: [] }), /sealed/);
});
