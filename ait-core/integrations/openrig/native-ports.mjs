import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { WorkflowBlocked, assertIdentity, assertCIJobIdentity } from './workflow-adapter.mjs';

const runFile = promisify(execFile);
export async function executeNative(binary, args, cwd) {
  try {
    const result = await runFile(binary, args, { cwd, maxBuffer: 16 * 1024 * 1024 });
    return { exit_code: 0, stdout: result.stdout, stderr: result.stderr };
  } catch (error) {
    return { exit_code: error.code, stdout: error.stdout ?? '', stderr: error.stderr ?? error.message };
  }
}

// The existing reviewer supplies review and independent delivery verification.
// This bridge owns native command routing and raw receipts, never a verdict.
export function createNativePorts({ request, cwd, evidenceDir, ait = 'ait', rig = 'rig', execute = executeNative, verifyDelivery, pollMs = 1000 }) {
  if (!request.remote || !request.local_task_id || !evidenceDir || typeof verifyDelivery !== 'function') throw new Error('Named remote, local Task, evidence directory and native delivery verifier are required');
  const guard = Object.fromEntries(['repository_index', 'patchset_id', 'revision_snapshot_id', 'base_snapshot_id'].map(key => [key, request.identity[key]]));
  const guardPath = join(evidenceDir, 'expected-identity.json');
  let sequence = 0;
  let guardWritten = false;
  const recordEvidence = async (phase, value) => {
    await mkdir(evidenceDir, { recursive: true });
    const path = join(evidenceDir, `${String(++sequence).padStart(4, '0')}-${phase}.json`);
    await writeFile(path, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 });
    return { phase, path };
  };
  const invoke = async (phase, apply = false) => {
    const args = ['workflow', phase, request.local_task_id, '--remote', request.remote, '--json', '--full'];
    if (apply) {
      if (!guardWritten) { await mkdir(evidenceDir, { recursive: true }); await writeFile(guardPath, JSON.stringify(guard) + '\n', { mode: 0o600 }); guardWritten = true; }
      args.push('--apply', '--expected-identity', guardPath);
      if (phase === 'finish') args.push('--review-message', request.review.summary);
      if (phase === 'ready') {
        for (const key of ['summary', 'tests', 'lint', 'security', 'license', 'author-mode', 'model']) {
          if (request.readiness?.[key] !== undefined) args.push(`--${key}`, String(request.readiness[key]));
        }
      }
    }
    const process = await execute(ait, args, cwd);
    const evidence = await recordEvidence(`${phase}-process`, { binary: ait, args, ...process });
    if (process.exit_code !== 0) throw new WorkflowBlocked('native_command_failed', 'Native workflow command failed; inspect raw evidence', evidence);
    let payload;
    try { payload = JSON.parse(process.stdout); } catch { throw new WorkflowBlocked('native_json_required', 'Native workflow JSON/full is required; no text parsing fallback', evidence); }
    if (phase === 'finish' && payload.task_land_contract) {
      payload.closeout = { status: payload.closeout_status, task_status: payload.task_status, change_status: payload.change_status };
      // Reuse the existing authoritative recovery action after partial cleanup.
      if (payload.closeout_recovery) payload.next_action = payload.closeout_recovery;
    }
    return payload;
  };
  return {
    recordEvidence,
    readOwner: async () => {
      const args = ['whoami', '--json'];
      const process = await execute(rig, args, cwd);
      const evidence = await recordEvidence('owner-process', { binary: rig, args, ...process });
      if (process.exit_code !== 0) throw new WorkflowBlocked('owner_unavailable', 'Native OpenRig owner read failed', evidence);
      let identity;
      try { identity = JSON.parse(process.stdout); } catch { throw new WorkflowBlocked('owner_unavailable', 'Native OpenRig identity JSON required', evidence); }
      if (identity.partial || !identity.identity?.logicalId) throw new WorkflowBlocked('owner_unavailable', 'Full managed identity recovery is required', evidence);
      return identity.identity.logicalId;
    },
    readIdentity: async () => (await invoke('ready')).identity,
    readCI: async () => {
      const state = await invoke('ready');
      assertIdentity(request.identity, state.identity);
      const args = ['patchset', 'ci-status', state.identity.patchset_id, '--remote', request.remote, '--json'];
      const process = await execute(ait, args, cwd);
      const evidence = await recordEvidence('ci-status-process', { binary: ait, args, ...process });
      if (process.exit_code !== 0) throw new WorkflowBlocked('native_command_failed', 'Native CI status read failed; no enqueue fallback', evidence);
      let ci;
      try { ci = JSON.parse(process.stdout); } catch { throw new WorkflowBlocked('native_json_required', 'Complete native CI status JSON is required', evidence); }
      if (ci.available !== true || ci.repository_index === undefined || !ci.patchset_id || !Number.isSafeInteger(ci.ci_run_seq) || ci.ci_run_seq < (state.patchset_ci_status?.ci_run_seq ?? 0)) throw new WorkflowBlocked('ci_run_changed', 'Native CI status is unavailable, incomplete or older than readiness', evidence);
      assertIdentity(state.identity, { ...state.identity, ...ci });
      const result = { ...state.identity, ...ci, native_next_action: state.next_action?.code };
      assertCIJobIdentity(result, request.identity);
      return result;
    },
    ready: () => invoke('ready', true),
    finish: () => invoke('finish', true),
    wait: () => new Promise(resolve => setTimeout(resolve, pollMs)),
    verifyDelivery,
  };
}
