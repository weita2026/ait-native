#!/usr/bin/env node
// One local writer per release operation. Lock metadata is process exclusion,
// never publication evidence. A supervised process group retains ownership if
// its launcher dies while a command or receipt worker is still running.
import { randomUUID } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync,
  renameSync, rmdirSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SELF = fileURLToPath(import.meta.url);
const LOCK = ".release-operation.lock";
const ROOT_ENV = "AIT_RELEASE_OPERATION_ROOT";
const TOKEN_ENV = "AIT_RELEASE_OPERATION_TOKEN";
const ownerName = token => `owner-${token}.json`;
const live = pid => {
  try { process.kill(pid, 0); return true; }
  catch (error) { if (error.code === "ESRCH") return false; if (error.code === "EPERM") return true; throw error; }
};
const groupLive = pid => pid !== null && live(-pid);
function canonical(root) {
  if (typeof root !== "string" || !path.isAbsolute(root)) throw new Error("release operation root must be absolute");
  mkdirSync(root, { recursive: true, mode: 0o700 });
  return realpathSync(root);
}
function readOwner(root) {
  const dir = path.join(root, LOCK);
  if (!lstatSync(dir).isDirectory() || lstatSync(dir).isSymbolicLink()) throw new Error("invalid release operation lock");
  const names = readdirSync(dir);
  if (names.length !== 1 || !/^owner-[0-9a-f-]{36}\.json$/.test(names[0])) throw new Error("unverifiable release operation owner; retain the lock");
  const file = path.join(dir, names[0]);
  if (!lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) throw new Error("invalid release operation owner file");
  const value = JSON.parse(readFileSync(file, "utf8"));
  if (value.contract !== "ait.release.operation-owner/v1" || value.root !== root || names[0] !== ownerName(value.token) ||
      !Number.isSafeInteger(value.supervisor_pid) || value.supervisor_pid <= 0 ||
      !(value.group_pid === null || (Number.isSafeInteger(value.group_pid) && value.group_pid > 0))) {
    throw new Error("unverifiable release operation owner; retain the lock");
  }
  return { file, value };
}
function release(root, token) {
  // Unlink only this generation's unique filename. A stale reaper can never
  // unlink a successor's owner; rmdir also cannot remove a nonempty successor.
  try { unlinkSync(path.join(root, LOCK, ownerName(token))); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  try { rmdirSync(path.join(root, LOCK)); }
  catch (error) { if (!["ENOENT", "ENOTEMPTY", "EEXIST"].includes(error.code)) throw error; }
}
function acquire(root) {
  const token = randomUUID();
  const value = { contract: "ait.release.operation-owner/v1", root, token, supervisor_pid: process.pid, group_pid: null };
  const staging = mkdtempSync(path.join(root, ".release-owner-"));
  writeFileSync(path.join(staging, ownerName(token)), JSON.stringify(value), { mode: 0o600, flag: "wx" });
  try {
    for (let attempt = 0; attempt < 16; attempt += 1) {
      try { renameSync(staging, path.join(root, LOCK)); return value; }
      catch (error) { if (!["ENOTEMPTY", "EEXIST"].includes(error.code)) throw error; }
      let prior;
      try { prior = readOwner(root); }
      catch (error) { if (error.code === "ENOENT") continue; throw error; }
      if (live(prior.value.supervisor_pid) || groupLive(prior.value.group_pid)) {
        throw new Error(`release operation already has a writer: ${root}`);
      }
      release(root, prior.value.token);
    }
    throw new Error("release ownership changed repeatedly; retry after the active writer exits");
  } finally {
    if (existsSync(staging)) rmSync(staging, { recursive: true });
  }
}
function updateOwner(owner) {
  const temporary = path.join(owner.root, `.release-owner-update-${owner.token}`);
  writeFileSync(temporary, JSON.stringify(owner), { mode: 0o600, flag: "wx" });
  renameSync(temporary, path.join(owner.root, LOCK, ownerName(owner.token)));
}
function inherited(root) {
  if (!process.env[TOKEN_ENV]) return false;
  if (process.env[ROOT_ENV] !== root) throw new Error("nested release operation root changed");
  const { value } = readOwner(root);
  const group = spawnSync("ps", ["-o", "pgid=", "-p", String(process.pid)], { encoding: "utf8" });
  if (value.token !== process.env[TOKEN_ENV] || Number(group.stdout?.trim()) !== value.group_pid || !groupLive(value.group_pid)) {
    throw new Error("nested release operation lease is invalid");
  }
  return true;
}
async function supervise(root, argv) {
  if (process.platform === "win32") throw new Error("release control ownership requires the existing POSIX shell orchestration host");
  const owner = acquire(root);
  let worker;
  try {
    worker = spawn(process.execPath, [SELF, "--worker", ...argv], {
      detached: true, stdio: ["inherit", "inherit", "inherit", "ipc"],
      env: { ...process.env, [ROOT_ENV]: root, [TOKEN_ENV]: owner.token },
    });
    await new Promise((resolve, reject) => { worker.once("spawn", resolve); worker.once("error", reject); });
    owner.group_pid = worker.pid;
    updateOwner(owner);
    const signal = name => { try { process.kill(-worker.pid, name); } catch (error) { if (error.code !== "ESRCH") throw error; } };
    for (const name of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(name, () => signal(name));
    const closed = new Promise(resolve => worker.once("exit", (code, reason) => resolve(code ?? (reason ? 128 + ({ SIGINT: 2, SIGTERM: 15, SIGHUP: 1 }[reason] ?? 1) : 1))));
    worker.send("go");
    const code = await closed;
    while (groupLive(worker.pid)) await new Promise(resolve => setTimeout(resolve, 50));
    return code;
  } finally {
    if (!worker || !groupLive(worker.pid)) release(root, owner.token);
  }
}

export async function enterOperation(requestedRoot) {
  try {
    const root = canonical(requestedRoot);
    if (inherited(root)) return;
    const code = await supervise(root, [process.execPath, ...process.argv.slice(1)]);
    process.exit(code);
  } catch (error) { process.stderr.write(error.message + "\n"); process.exit(75); }
}
export async function enterDerivedOperation() { await enterOperation(derivedRoot(process.argv.slice(2))); }

function derivedRoot(argv, script = process.argv[1]) {
  if (process.env[ROOT_ENV]) return canonical(process.env[ROOT_ENV]);
  for (const key of ["--request", "--input", "--spec", "--plan"]) {
    const i = argv.indexOf(key);
    if (i >= 0 && argv[i + 1]) {
      const data = JSON.parse(readFileSync(argv[i + 1], "utf8"));
      if (data.records_root) return canonical(data.records_root);
    }
  }
  const output = argv.indexOf("--output");
  const publicSource = ["publish-qualification", "probe-qualification", "publish-final", "probe-final", "publish-tag", "probe-tag"].includes(argv[0]);
  const positional = ["release_candidate_promote.sh", "release_endpoint_publication.sh", "release_endpoint_remote.sh", "release_latest_alias.sh", "release_protected_promotion.sh"].includes(path.basename(script));
  const i = output >= 0 ? output : publicSource || positional ? argv.length - 2 : argv.findIndex(arg => ["--status", "--config", "--prepare", "--dossier-root", "--output-dir"].includes(arg));
  if (i < 0 || !path.isAbsolute(argv[i + 1] ?? "")) throw new Error("cannot identify release operation root; supply the existing operation through AIT_RELEASE_OPERATION_ROOT");
  const value = argv[i + 1];
  const base = ["--dossier-root", "--output-dir"].includes(argv[i]) ? value : path.dirname(value);
  for (let dir = base; dir !== path.dirname(dir); dir = path.dirname(dir)) {
    if ([LOCK, "conductor-plan.json", "preparation-state.json", "release-spec.json", "accepted-input.json"].some(name => existsSync(path.join(dir, name)))) return canonical(dir);
  }
  // Existing output-parent guards remain authoritative. Ownership must not
  // make an otherwise invalid missing parent become a valid destination.
  return realpathSync(base);
}

if (process.argv[1] && realpathSync(process.argv[1]) === SELF) {
  const [mode, ...argv] = process.argv.slice(2);
  try {
    if (mode === "--worker") {
      // No command starts until the group identity is published in the lock.
      let started = false;
      process.on("disconnect", () => { if (!started) process.exit(75); });
      process.once("message", message => {
        if (message !== "go") process.exit(75);
        started = true;
        const child = spawn(argv[0], argv.slice(1), { stdio: "inherit", env: process.env });
        child.once("error", error => { process.stderr.write(error.message + "\n"); process.exit(1); });
        child.once("exit", (code, signal) => process.exit(code ?? (signal ? 128 + ({ SIGINT: 2, SIGTERM: 15, SIGHUP: 1 }[signal] ?? 1) : 1)));
      });
    } else if (mode === "--shell") {
      process.exit(await supervise(derivedRoot(argv.slice(1), argv[0]), argv));
    } else if (mode === "--check") {
      if (!inherited(realpathSync(argv[0]))) throw new Error("release operation lease is missing");
    } else throw new Error("invalid internal release ownership invocation");
  } catch (error) { process.stderr.write(error.message + "\n"); process.exit(75); }
}
