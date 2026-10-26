#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const temporary = mkdtempSync(path.join(os.tmpdir(), "ait-release-owner-test."));
const fixture = path.join(temporary, "writer.mjs");
writeFileSync(fixture, `
import { enterOperation } from ${JSON.stringify(path.join(here, "release_operation_owner.mjs"))};
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
const [root, id, hold] = process.argv.slice(2);
// An immutable input may have been loaded before entry; mutable state must be
// reread under ownership, even after a previous invocation was killed.
const prior = existsSync(path.join(root, 'count')) ? readFileSync(path.join(root, 'count'), 'utf8') : '0';
await enterOperation(root);
const current = existsSync(path.join(root, 'count')) ? Number(readFileSync(path.join(root, 'count'), 'utf8')) : 0;
writeFileSync(path.join(root, 'count'), String(current + 1));
writeFileSync(path.join(root, id + '.entered'), JSON.stringify({pid:process.pid, prior, current}));
if (hold === 'hold') while (!existsSync(path.join(root, id + '.release'))) await new Promise(resolve => setTimeout(resolve, 10));
`);
const children = new Set();
function start(root, id, hold = "") {
  const child = spawn(process.execPath, [fixture, root, id, hold], { env: { ...process.env,
    AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" } });
  children.add(child);
  let stderr = "";
  child.stderr.on("data", data => { stderr += data; });
  child.stdout.resume();
  const finished = new Promise(resolve => child.once("exit", (code, signal) => { children.delete(child); resolve({ code, signal, stderr }); }));
  return { child, finished };
}
async function waitFor(predicate, label) {
  const deadline = Date.now() + 15000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`timeout: ${label}`);
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}
function owner(root) {
  const dir = path.join(root, ".release-operation.lock");
  return JSON.parse(readFileSync(path.join(dir, readdirSync(dir)[0]), "utf8"));
}
const groups = new Set();
try {
  const root = path.join(temporary, "operation");
  const first = start(root, "first", "hold");
  await waitFor(() => existsSync(path.join(root, "first.entered")), "first writer entered");
  groups.add(owner(root).group_pid);
  const second = await start(root, "second").finished;
  assert.equal(second.code, 75, second.stderr);
  assert.match(second.stderr, /already has a writer/);
  assert.equal(existsSync(path.join(root, "second.entered")), false);
  assert.equal(readFileSync(path.join(root, "count"), "utf8"), "1");
  writeFileSync(path.join(root, "first.release"), "go");
  assert.equal((await first.finished).code, 0);
  assert.equal((await start(root, "after").finished).code, 0);
  assert.equal(JSON.parse(readFileSync(path.join(root, "after.entered"))).current, 1);

  const killed = start(root, "killed", "hold");
  await waitFor(() => existsSync(path.join(root, "killed.entered")), "kill boundary");
  const lease = owner(root);
  groups.add(lease.group_pid);
  // Kill only the launcher. The command group still owns the operation.
  killed.child.kill("SIGKILL");
  await killed.finished;
  assert.equal((await start(root, "orphan-race").finished).code, 75);
  assert.equal(existsSync(path.join(root, "orphan-race.entered")), false);
  writeFileSync(path.join(root, "killed.release"), "go");
  await waitFor(() => { try { process.kill(-lease.group_pid, 0); return false; } catch (error) { return error.code === "ESRCH"; } }, "orphan group exit");

  // Two independent contenders race to reclaim one dead generation. Only one
  // may enter; the loser cannot remove the successor's unique owner file.
  const a = start(root, "recover-a", "hold");
  const b = start(root, "recover-b", "hold");
  await waitFor(() => existsSync(path.join(root, "recover-a.entered")) || existsSync(path.join(root, "recover-b.entered")), "recovery winner");
  const winner = existsSync(path.join(root, "recover-a.entered")) ? "recover-a" : "recover-b";
  groups.add(owner(root).group_pid);
  const loser = winner === "recover-a" ? b : a;
  assert.equal((await loser.finished).code, 75);
  assert.equal(readFileSync(path.join(root, "count"), "utf8"), "4");
  writeFileSync(path.join(root, winner + ".release"), "go");
  assert.equal((await (winner === "recover-a" ? a : b).finished).code, 0);
  assert.equal((await start(root, "final").finished).code, 0);
  assert.equal(JSON.parse(readFileSync(path.join(root, "final.entered"))).current, 4);
  assert.equal(existsSync(path.join(root, ".release-operation.lock")), false);
  const alias = path.join(temporary, "owner-alias.mjs");
  symlinkSync(path.join(here, "release_operation_owner.mjs"), alias);
  const aliased = spawn(process.execPath, [alias, "--shell", process.execPath, fixture, root, "alias"], {
    env: { ...process.env, AIT_RELEASE_OPERATION_ROOT: root, AIT_RELEASE_OPERATION_TOKEN: "" }, stdio: "ignore",
  });
  assert.equal(await new Promise(resolve => aliased.once("exit", resolve)), 0);
  assert.equal(JSON.parse(readFileSync(path.join(root, "alias.entered"))).current, 5);
  console.log("release operation ownership tests passed");
} finally {
  for (const group of groups) { try { process.kill(-group, "SIGKILL"); } catch {} }
  for (const child of children) child.kill("SIGKILL");
  rmSync(temporary, { recursive: true, force: true });
}
