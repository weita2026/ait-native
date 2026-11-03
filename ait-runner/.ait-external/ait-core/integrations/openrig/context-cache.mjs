import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';

export const hashContent = text => createHash('sha256').update(text).digest('hex');
export class DirectedContextCache {
  #entries = new Map();
  invalidate(reason) {
    if (!['compaction', 'handover', 'scope_changed', 'owner_changed', 'candidate_changed', 'runtime_changed'].includes(reason)) throw new Error('Unknown context invalidation reason');
    this.#entries.clear();
  }
  async read({ version, ref, contentHash, scope }, load) {
    if (!version || !ref || !/^[a-f0-9]{64}$/.test(contentHash) || !scope) throw new Error('Fresh version/ref/content hash/scope are required');
    const key = JSON.stringify([version, ref, contentHash, scope]);
    if (this.#entries.has(key)) return { text: this.#entries.get(key), cached: true };
    const text = await load(ref);
    if (hashContent(text) !== contentHash) throw new Error('Context changed during load; refresh the selected ref metadata');
    this.#entries.set(key, text);
    return { text, cached: false };
  }
  // Exact source paths come from the explicitly selected installed pack. A
  // fresh local digest validates cache use without rereading skill text into
  // the model. OpenRig metadata does not itself provide a trusted SHA.
  async readSelectedSource({ version, ref, sourcePaths, scope }, load) {
    if (!version || !ref || !scope || !sourcePaths?.length) throw new Error('Resolved version/ref/source paths/scope are required');
    const digest = async () => {
      const hashes = await Promise.all(sourcePaths.map(async path => [path, hashContent(await readFile(path))]));
      return hashContent(JSON.stringify(hashes));
    };
    const sourceHash = await digest();
    const key = JSON.stringify([version, ref, sourceHash, scope]);
    if (this.#entries.has(key)) return { text: this.#entries.get(key), cached: true, sourceHash };
    const text = await load(ref); // existing `rig context get <exact-ref>`
    if (await digest() !== sourceHash) throw new Error('Selected source changed during context load');
    this.#entries.set(key, text);
    return { text, cached: false, sourceHash, contentHash: hashContent(text) };
  }
}

// The caller resolves these exact refs once from installed metadata. No list
// JSON firehose, invented flags, or cloned AIT workflow guidance is produced.
export function bootstrapPlan({ role, queueId, guidancePath, sprintPath, contextRefs, seal }) {
  if (!['worker', 'reviewer'].includes(role) || !queueId || !guidancePath || !sprintPath) throw new Error('Explicit role, queue and source paths are required');
  if (role === 'reviewer' && !seal) throw new Error('Reviewer requires a sealed handoff');
  return { commands: [['rig', 'whoami', '--json'], ['rig', 'queue', 'claim', queueId, '--json'], ...contextRefs.map(ref => ['rig', 'context', 'get', ref])], readPaths: [guidancePath, sprintPath], seal: role === 'reviewer' ? seal : undefined };
}
