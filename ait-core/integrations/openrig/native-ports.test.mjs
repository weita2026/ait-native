import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createNativePorts } from './native-ports.mjs';
import { classifyCI } from './workflow-adapter.mjs';
import { DirectedContextCache } from './context-cache.mjs';

test('native bridge uses named remote JSON/full and optional identity with raw receipts', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ait-openrig-native-'));
  try {
    const request = { remote: 'review', local_task_id: 'LT-1', identity: { repository_index: 7, patchset_id: 'RT-1/P-01', revision_snapshot_id: 'R', base_snapshot_id: 'B' }, review: { summary: 'Real review supplied by the assigned owner' } };
    const calls = [];
    const ports = createNativePorts({ request, cwd: dir, evidenceDir: dir, verifyDelivery: async () => ({}), execute: async (binary, args) => {
      calls.push({ binary, args });
      return { exit_code: 0, stderr: '', stdout: JSON.stringify({ identity: request.identity, apply_status: 'stopped', next_action: { code: 'record_review' } }) };
    } });
    const result = await ports.finish();
    assert.equal(result.apply_status, 'stopped');
    assert.deepEqual(calls[0].args.slice(0, 8), ['workflow', 'finish', 'LT-1', '--remote', 'review', '--json', '--full', '--apply']);
    assert.ok(calls[0].args.includes('--expected-identity'));
    assert.ok(!calls[0].args.includes('--reviewer'));
    assert.ok(!calls[0].args.includes('--run-ci-once'));
    assert.deepEqual(JSON.parse(await readFile(join(dir, 'expected-identity.json'), 'utf8')), request.identity);
    const receipt = JSON.parse(await readFile(join(dir, '0001-finish-process.json'), 'utf8'));
    assert.equal(receipt.exit_code, 0);
    assert.ok(receipt.stdout.includes('record_review'));
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('native bridge fails closed when an old CLI emits text despite exit zero', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ait-openrig-old-cli-'));
  try {
    const ports = createNativePorts({ request: { remote: 'review', local_task_id: 'LT-1', identity: {} }, cwd: dir, evidenceDir: dir, verifyDelivery: async () => ({}), execute: async () => ({ exit_code: 0, stdout: 'status: review', stderr: '' }) });
    await assert.rejects(ports.readIdentity(), { code: 'native_json_required' });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('native CI provenance is preserved and conflicting Repository or Patchset cannot be overwritten', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ait-openrig-ci-provenance-'));
  try {
    const identity = { repository_index: 7, patchset_id: 'RT-1/P-01', revision_snapshot_id: 'R', base_snapshot_id: 'B' };
    let ci = { available: true, repository_index: 7, patchset_id: 'RT-1/C-01/P-01', patchset_index: 3, ci_run_seq: 1, latest_job: { repository_index: 7, patchset_index: 3, state: 'running', worker_job_index: 8, patchset_ci: { ci_run_seq: 1, ci_worker_job_index: 8 } } };
    const ports = createNativePorts({ request: { remote: 'review', local_task_id: 'LT-1', identity }, cwd: dir, evidenceDir: dir, verifyDelivery: async () => ({}), execute: async (binary, args) => ({ exit_code: 0, stderr: '', stdout: JSON.stringify(args[0] === 'patchset' ? ci : { identity, patchset_ci_status: { ci_run_seq: 1 }, next_action: { code: 'waiting_for_ci' } }) }) });
    assert.equal((await ports.readCI()).patchset_index, 3);
    ci = { ...ci, repository_index: 8 };
    await assert.rejects(ports.readCI(), { code: 'candidate_changed' });
    ci = { ...ci, repository_index: 7, patchset_id: 'RT-2/P-01' };
    await assert.rejects(ports.readCI(), { code: 'candidate_changed' });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('captured live embedded pending reads the exact complete native CI job without submitting', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ait-openrig-live-ci-'));
  try {
    const state = JSON.parse(await readFile(new URL('./fixtures/core-embedded-pending.json', import.meta.url), 'utf8'));
    const captured = JSON.parse(await readFile(new URL('./fixtures/core-full-ci-running.json', import.meta.url), 'utf8'));
    let ci = structuredClone(captured);
    const commands = [];
    const ports = createNativePorts({ request: { remote: 'origin', local_task_id: 'LCT-1058', identity: state.identity }, cwd: dir, evidenceDir: dir, verifyDelivery: async () => ({}), execute: async (binary, args) => {
      commands.push(args);
      return { exit_code: 0, stderr: '', stdout: JSON.stringify(args[0] === 'patchset' ? ci : state) };
    } });
    assert.equal(state.patchset_ci_status.latest_job, null);
    assert.equal(classifyCI(await ports.readCI(), state.identity), 'pending');
    assert.deepEqual(commands[1], ['patchset', 'ci-status', state.identity.patchset_id, '--remote', 'origin', '--json']);
    assert.ok(commands.every(args => !args.includes('--apply') && !args.includes('rerun-ci')));
    ci = JSON.parse(await readFile(new URL('./fixtures/core-full-ci-pass.json', import.meta.url), 'utf8'));
    assert.equal(classifyCI(await ports.readCI(), state.identity), 'passed');
    ci.latest_job.state = 'running';
    assert.equal(classifyCI(await ports.readCI(), state.identity), 'pending');
    ci.latest_job.state = 'failed';
    assert.equal(classifyCI(await ports.readCI(), state.identity), 'failed');
    ci = structuredClone(captured); ci.latest_job.patchset_ci.ci_run_seq++;
    await assert.rejects(ports.readCI(), { code: 'ci_run_changed' });
    ci = structuredClone(captured); ci.ci_run_seq = 0;
    await assert.rejects(ports.readCI(), { code: 'ci_run_changed' });
    ci = structuredClone(captured); ci.available = false;
    await assert.rejects(ports.readCI(), { code: 'ci_run_changed' });
    ci = structuredClone(captured); ci.latest_job.patchset_ci.ci_worker_job_index++;
    await assert.rejects(ports.readCI(), { code: 'ci_run_changed' });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('native owner recovery uses compact whoami and rejects a partial managed identity', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ait-openrig-owner-'));
  try {
    let partial = false;
    const ports = createNativePorts({ request: { remote: 'review', local_task_id: 'LT-1', identity: {} }, cwd: dir, evidenceDir: dir, verifyDelivery: async () => ({}), execute: async (binary, args) => {
      assert.equal(binary, 'rig'); assert.deepEqual(args, ['whoami', '--json']);
      return { exit_code: 0, stderr: '', stdout: JSON.stringify({ partial, identity: { logicalId: partial ? null : 'review-team/reviewer' } }) };
    } });
    assert.equal(await ports.readOwner(), 'review-team/reviewer');
    partial = true;
    await assert.rejects(ports.readOwner(), { code: 'owner_unavailable' });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('fresh source digest avoids repeated context get and reloads after installed content changes', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ait-openrig-context-'));
  try {
    const source = join(dir, 'SKILL.md');
    await writeFile(source, '# Skill\n## Chosen\nfirst\n');
    const cache = new DirectedContextCache();
    const metadata = { version: '0.6.7', ref: 'skills/exact/SKILL.md#chosen', sourcePaths: [source], scope: 'reviewer:RT-1/P-01' };
    let reads = 0;
    const load = async ref => { assert.equal(ref, metadata.ref); reads++; return (await readFile(source, 'utf8')).split('## Chosen\n')[1]; };
    assert.equal((await cache.readSelectedSource(metadata, load)).text, 'first\n');
    assert.equal((await cache.readSelectedSource(metadata, load)).cached, true);
    assert.equal(reads, 1);
    await writeFile(source, '# Skill\n## Chosen\nsecond\n');
    assert.equal((await cache.readSelectedSource(metadata, load)).text, 'second\n');
    cache.invalidate('handover');
    await cache.readSelectedSource(metadata, load);
    assert.equal(reads, 3);
  } finally { await rm(dir, { recursive: true, force: true }); }
});
