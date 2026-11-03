#!/usr/bin/env node
// Commit receipt hashes before exposing outputs. The worker survives a conductor
// interruption; recovery verifies the exact plan, inputs and bytes before adoption.
import { createHash, randomUUID } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const hash = value => createHash('sha256').update(value).digest('hex');
export function digest(target) {
  const h = createHash('sha256');
  function walk(file, relative) {
    const st = lstatSync(file);
    if (st.isSymbolicLink()) throw new Error(`receipt symlink rejected: ${file}`);
    if (st.isDirectory()) { h.update(`d\0${relative}\0`); for (const name of readdirSync(file).sort()) walk(path.join(file, name), path.join(relative, name)); }
    else if (st.isFile()) { h.update(`f\0${relative}\0${st.mode & 0o777}\0`); h.update(readFileSync(file)); }
    else throw new Error(`unsupported receipt: ${file}`);
  }
  walk(target, '.'); return h.digest('hex');
}
function atomic(file, value) {
  const temp = `${file}.${randomUUID()}.new`;
  writeFileSync(temp, JSON.stringify(value, null, 2) + '\n', { mode: 0o600, flag: 'wx' });
  renameSync(temp, file);
}
function identity(context) {
  // Scalar candidate/run bindings and explicit file inputs are part of identity.
  const outputs = context.action.outputs ?? context.action.bind?.outputs ?? [];
  const inputs = {};
  for (const arg of [...(context.action.argv ?? []), ...(context.action.bind?.argv ?? []), ...(context.action.probe?.argv ?? []), ...(context.action.adopt?.argv ?? [])]) {
    if (path.isAbsolute(arg) && !outputs.includes(arg) && existsSync(arg) && lstatSync(arg).isFile()) inputs[arg] = digest(arg);
  }
  for (const binding of Object.values(context.action.bindings ?? {})) inputs[binding.path] = digest(binding.path);
  return hash(JSON.stringify({ context, inputs }));
}
function receiptPath(root, id) { return path.join(root, 'conductor-receipts', `${id}.json`); }
export function receiptBindings(root, id) {
  const file = receiptPath(root, id);
  return existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')).bindings : null;
}
export function recoverReceiptTransaction(root, id, context) {
  const file = receiptPath(root, id);
  if (!existsSync(file)) return false;
  const receipt = JSON.parse(readFileSync(file, 'utf8'));
  if (receipt.contract !== 'ait.release.action-receipt/v1' || receipt.identity !== identity(context)) throw new Error(`${id}: receipt plan, candidate, executable or input drift`);
  for (const row of receipt.outputs) {
    const source = existsSync(row.path) ? row.path : row.staged;
    if (!source || digest(source) !== row.sha256) throw new Error(`${id}: receipt output drift: ${row.path}`);
    if (source !== row.path) renameSync(source, row.path);
  }
  return true;
}
function expand(argv, replacements) {
  return argv.map(arg => Object.entries(replacements).reduce((s, [key, value]) => s.replaceAll(key, String(value)), arg));
}
export function runReceiptTransaction(root, id, context, action, replacements, log) {
  if (recoverReceiptTransaction(root, id, context)) return { status: 0 };
  const requestRoot = path.join(root, 'conductor-transactions');
  mkdirSync(requestRoot, { recursive: true, mode: 0o700 });
  const attempt = mkdtempSync(path.join(requestRoot, `${id}-`));
  const request = path.join(attempt, 'request.json');
  atomic(request, { root, id, context, action, replacements, log, attempt, identity: identity(context) });
  const result = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--worker', request], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (result.status !== 0 && result.stderr) writeFileSync(log, result.stderr, { mode: 0o600 });
  return result;
}
function worker(request) {
  const { root, id, context, action, replacements, log, attempt, identity: expected } = request;
  const lock = path.join(root, 'conductor-transactions', id + '.lock');
  if (existsSync(lock)) {
    const owner = JSON.parse(readFileSync(path.join(lock, 'owner.json'), 'utf8'));
    try { process.kill(owner.pid, 0); process.exit(75); }
    catch (error) { if (error.code !== 'ESRCH') throw error; }
    renameSync(lock, lock + '.' + randomUUID() + '.stale');
  }
  mkdirSync(lock);
  atomic(path.join(lock, 'owner.json'), { pid: process.pid });
  process.on('exit', () => rmSync(lock, { recursive: true, force: true }));
  const args = expand(action.argv, replacements);
  const staged = action.outputs.map((output, i) => path.join(attempt, `output-${i}`));
  const adopting = action.outputs.some(output => existsSync(output)) && Boolean(action.adopt);
  const verification = adopting ? action.adopt : action.probe;
  const resumable = Boolean(verification);
  if (!resumable && action.outputs.some(output => existsSync(output))) throw new Error(`${id}: unbound pre-existing output requires independently validated receipt`);
  if (!resumable && !action.outputs.every(output => args.includes(output))) throw new Error(`${id}: output must be an exact argv path or have a validating resume probe`);
  const invocation = resumable ? args : args.map(arg => { const index = action.outputs.indexOf(arg); return index < 0 ? arg : staged[index]; });
  const probe = () => spawnSync(expand(verification.argv, replacements)[0], expand(verification.argv, replacements).slice(1), { cwd: verification.cwd, encoding: 'utf8' });
  let result;
  if (resumable && action.outputs.some(output => existsSync(output))) {
    result = probe(); // Fail closed on orphan evidence that does not validate.
  } else {
    result = spawnSync(invocation[0], invocation.slice(1), { cwd: action.cwd, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
    if (result.status === 0 && resumable) result = probe();
  }
  writeFileSync(log, `${result.stdout ?? ''}${result.stderr ?? ''}`, { mode: 0o600 });
  if (result.status !== 0) process.exit(result.status ?? 65);
  if (identity(context) !== expected) throw new Error(`${id}: input changed during execution`);
  const outputs = action.outputs.map((output, i) => ({ path: output, staged: resumable ? null : staged[i], sha256: digest(resumable ? output : staged[i]) }));
  mkdirSync(path.dirname(receiptPath(root, id)), { recursive: true, mode: 0o700 });
  atomic(receiptPath(root, id), { contract: 'ait.release.action-receipt/v1', identity: expected, bindings: context.replacements, plan_sha256: context.plan_sha256, release: context.release, outputs });
  recoverReceiptTransaction(root, id, context);
}
if (process.argv[2] === '--worker') {
  try { worker(JSON.parse(readFileSync(process.argv[3], 'utf8'))); }
  catch (error) { process.stderr.write(`${error.message}\n`); process.exit(65); }
}
