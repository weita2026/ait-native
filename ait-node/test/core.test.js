import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { once } from "node:events";
import { Worker } from "node:worker_threads";
import { chmod, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  NativeRuntime, WorkspaceCommandLock, PlanRemoteTransportConfig,
  PlanHttpClientManager, TaskWorkflowHttpClientManager, HttpTaskRemote, HttpWorkflowCloseoutRemote,
  DEFAULT_REQUEST_TIMEOUT_SECONDS, LONG_RUNNING_REQUEST_TIMEOUT_SECONDS,
} from "../src/index.js";

const ENTRYPOINT = fileURLToPath(new URL("../bin/ait.mjs", import.meta.url));

async function repository(context) {
  const temporary = await realpath(await mkdtemp(path.join(os.tmpdir(), "ait-node-direct-core-")));
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const root = path.join(temporary, "repository with spaces");
  await mkdir(root);
  const runtime = new NativeRuntime();
  // Seed the isolated authority through the retained domain API. Disable
  // Teams before invoking bootstrap-aware CLI operations in this fixture.
  runtime.taskWorkflowInit(root, { name: "node-direct-core-fixture" });
  runtime.taskWorkflowConfigSet(root, { agent_teams: "disabled", sprint: "off" });
  return { root, runtime };
}

function nativeFailure(error) {
  assert.ok(error instanceof Error);
  assert.equal(error.code, "GenericFailure");
  assert.ok(error.message.length > 0);
  return true;
}

function cli(root, ...args) {
  return spawnSync(process.execPath, [ENTRYPOINT, ...args], {
    cwd: root, encoding: "utf8", windowsHide: true,
  });
}

test("real named APIs persist configuration and read isolated repository state", async (context) => {
  const { root, runtime } = await repository(context);
  const initialized = runtime.initializeRepository(root, { repairExisting: true });
  assert.equal(runtime.configurationError(initialized), null);
  const configured = runtime.configureRepository(root, {
    plan_language: "ja-JP", plan_style: "detailed", agent_teams: "disabled",
  });
  assert.equal(runtime.configurationError(configured), null);
  const shown = runtime.taskWorkflowConfigShow(root);
  const encoded = runtime.encodeJson(shown);
  assert.match(encoded, /ja-JP/);
  assert.match(encoded, /detailed/);
  assert.deepEqual(runtime.taskWorkflowTaskList(root, { local: true, all: true }), []);
  assert.equal(typeof runtime.taskWorkflowDoctorPlanAuthority(), "object");
  const status = runtime.taskWorkflowStatus(root);
  assert.ok(runtime.formatStatus(status).length > 0);
  assert.ok(runtime.encodeJson(runtime.projectStatus(status, true)).length > 0);
});

test("real APIs preserve validation, missing identity and explicit remote errors", async (context) => {
  const { root, runtime } = await repository(context);
  assert.throws(() => runtime.taskWorkflowConfigSet(root, { unsupported_field: true }), nativeFailure);
  assert.throws(() => runtime.taskWorkflowTaskShow(root, "LNT-9999", { local: true }), nativeFailure);
  assert.throws(() => runtime.taskWorkflowTaskList(root, { remoteName: "missing-remote" }), nativeFailure);
  assert.throws(() => runtime.taskFinish(root, "LNT-9999", { local: true }), nativeFailure);
  assert.throws(() => runtime.snapshotCapture(root, "LNT-9999"), nativeFailure);
});

test("native JSON codec retains full-width integers and original parse failures", () => {
  const runtime = new NativeRuntime();
  const text = '{"negative":-9007199254740993,"positive":9007199254740993}';
  assert.equal(runtime.encodeJson(runtime.decodeJson(text)), text);
  assert.equal(runtime.encodeJson({ maximum: 18446744073709551615n }), '{"maximum":18446744073709551615}');
  assert.equal(runtime.encodeJson({ minimum: -9223372036854775808n }), '{"minimum":-9223372036854775808}');
  for (const value of [9007199254740992, 18446744073709551616n, -9223372036854775809n]) {
    assert.throws(() => runtime.encodeJson({ value }), Error);
  }
  const cycle = {};
  cycle.self = cycle;
  assert.throws(() => runtime.encodeJson(cycle), Error);
  assert.throws(() => runtime.decodeJson("{"), nativeFailure);
});

test("real Task worktree supports capture, binary blobs, Stash and Tag selectors", async (context) => {
  const { root, runtime } = await repository(context);
  const editRoot = path.join(path.dirname(root), "native task worktree");
  let taskId;
  try {
    const started = runtime.taskStart(root, {
      title: "Node direct Task fixture", intent: "Exercise native lifecycle",
      local: true, editRoot,
    });
    assert.ok(runtime.formatTaskStart(started).length > 0);
    const tasks = runtime.taskWorkflowTaskList(root, { local: true, all: true });
    assert.equal(tasks.length, 1);
    taskId = tasks[0].task_id;
    assert.equal(typeof taskId, "string");
    assert.equal(runtime.taskWorkflowTaskShow(root, taskId, { local: true }).status, "active");
    const worktree = runtime.taskWorkflowWorktreeGet(editRoot, { refreshStatus: false });
    assert.match(runtime.encodeJson(worktree), new RegExp(taskId));
    const worktreeStatus = runtime.taskWorkflowWorktreeStatus(editRoot);
    assert.ok(runtime.encodeJson(worktreeStatus).length > 0);
    const usage = runtime.taskWorkflowWorktreeTouchUsage(editRoot);
    assert.ok(runtime.encodeJson(usage).length > 0);
    const worktreeShow = runtime.worktreeShow(editRoot);
    assert.ok(runtime.formatWorktreeShow(worktreeShow).length > 0);
    const worktreePath = runtime.worktreePath(editRoot);
    assert.ok(runtime.encodeJson(runtime.projectWorktreePath(worktreePath)).length > 0);
    for (const [command, shell] of [["path", false], ["open", true]]) {
      const pathCli = cli(editRoot, "worktree", command, ...(shell ? ["--shell"] : []));
      assert.equal(pathCli.status, 0, pathCli.stderr);
      assert.equal(pathCli.stdout, `${runtime.formatWorktreePath(worktreePath, shell)}\n`);
    }
    const audited = runtime.taskAudit(root, taskId, { local: true });
    assert.ok(runtime.formatTaskAudit(audited, taskId).length > 0);
    const bytes = Buffer.from([0, 255, 128, 10, 13, 65]);
    const blobId = runtime.taskWorkflowBlobEnsureBytes(editRoot, bytes, "fixture.bin");
    assert.equal(typeof blobId, "string");
    const restoredBytes = runtime.taskWorkflowBlobReadBytes(editRoot, blobId);
    assert.equal(Buffer.isBuffer(restoredBytes), true);
    assert.deepEqual(restoredBytes, bytes);
    await writeFile(path.join(editRoot, "fixture.txt"), "native capture fixture\n");
    const captured = runtime.snapshotCapture(editRoot, taskId, { message: "Native fixture capture" });
    assert.ok(runtime.formatSnapshotCapture(captured).length > 0);
    assert.ok(runtime.projectSnapshotCapture(captured, false));
    const firstTag = runtime.tagCreate(editRoot, "node-first-tag", "First native capture");
    const firstSnapshot = runtime.resolveSnapshotRef(editRoot, "node-first-tag");
    assert.equal(firstTag.snapshot_id, firstSnapshot);
    assert.equal(firstTag.source_line, runtime.lineInventory(editRoot).current_line);
    assert.ok(runtime.formatTagCreate(firstTag).length > 0);
    const explicitTag = runtime.tagCreate(editRoot, "node-explicit-tag", "Exact native capture", firstSnapshot);
    assert.equal(explicitTag.snapshot_id, firstSnapshot);
    assert.equal(runtime.resolveSnapshotRef(editRoot, firstSnapshot), firstSnapshot);
    assert.throws(() => runtime.tagCreate(editRoot, "node-first-tag", "Duplicate", firstSnapshot), nativeFailure);
    const fixturePath = path.join(editRoot, "fixture.txt");
    await writeFile(fixturePath, "stashed fixture contents\n");
    const dirtyDiff = runtime.workspaceDiff(editRoot, ["fixture.txt"]);
    assert.match(runtime.encodeJson(dirtyDiff), /fixture\.txt/);
    for (const [flags, render] of [
      [[], () => runtime.renderWorkspaceDiffText(dirtyDiff)],
      [["--stat"], () => runtime.renderWorkspaceDiffStat(dirtyDiff)],
      [["--name-only"], () => runtime.renderWorkspaceDiffNames(dirtyDiff)],
    ]) {
      const diffCli = cli(editRoot, "diff", ...flags, "fixture.txt");
      assert.equal(diffCli.status, 0, diffCli.stderr);
      assert.equal(diffCli.stdout, render());
    }
    const saved = runtime.stashSave(editRoot, { message: "Native Stash fixture" });
    assert.equal(typeof saved.stash_id, "string");
    assert.ok(runtime.formatStashSave(saved).length > 0);
    assert.equal(await readFile(fixturePath, "utf8"), "native capture fixture\n");
    assert.equal(runtime.taskWorkflowStashList(editRoot).length, 1);
    const shown = runtime.taskWorkflowStashShow(editRoot, saved.stash_id);
    assert.equal(shown.stash_id, saved.stash_id);
    assert.ok(runtime.formatStashShow(shown).length > 0);
    const applied = runtime.stashApply(editRoot, saved.stash_id);
    assert.ok(runtime.formatStashApply(applied).length > 0);
    assert.equal(await readFile(fixturePath, "utf8"), "stashed fixture contents\n");
    const conflict = cli(editRoot, "stash", "pop", saved.stash_id, "--json");
    assert.equal(conflict.status, runtime.domainErrorExitCode());
    assert.equal(conflict.stdout, "");
    assert.match(conflict.stderr, /--force/);
    const popped = runtime.stashPop(editRoot, saved.stash_id, true);
    assert.ok(runtime.formatStashPop(popped).length > 0);
    assert.deepEqual(runtime.taskWorkflowStashList(editRoot), []);
    const kept = runtime.stashSave(editRoot, { keepWorkspace: true });
    assert.equal(await readFile(fixturePath, "utf8"), "stashed fixture contents\n");
    const dropped = runtime.stashDrop(editRoot, kept.stash_id);
    assert.ok(runtime.formatStashDrop(dropped).length > 0);
    assert.deepEqual(runtime.taskWorkflowStashList(editRoot), []);
    runtime.snapshotCapture(editRoot, taskId, { message: "Second native fixture capture" });
    const secondTag = runtime.tagCreate(editRoot, "node-second-tag", "Second native capture");
    const secondSnapshot = runtime.resolveSnapshotRef(editRoot, "node-second-tag");
    assert.equal(secondTag.snapshot_id, secondSnapshot);
    assert.notEqual(secondSnapshot, firstSnapshot);
    const cleanDiff = runtime.workspaceDiff(editRoot);
    assert.equal(runtime.renderWorkspaceDiffText(cleanDiff), "");
    assert.equal(runtime.renderWorkspaceDiffStat(cleanDiff), "");
    assert.equal(runtime.renderWorkspaceDiffNames(cleanDiff), "");
    for (const flags of [[], ["--stat"], ["--name-only"]]) {
      const cleanCli = cli(editRoot, "diff", ...flags);
      assert.equal(cleanCli.status, 0, cleanCli.stderr);
      assert.equal(cleanCli.stdout, "");
    }
    const blamed = runtime.taskWorkflowBlame(editRoot, "fixture.txt", { snapshotId: secondSnapshot, line: 1 });
    assert.match(runtime.encodeJson(blamed), /fixture\.txt/);
    const blameText = runtime.formatBlame(blamed);
    assert.ok(blameText.length > 0);
    const blameCli = cli(editRoot, "blame", "fixture.txt", "--snapshot", secondSnapshot, "--line", "1");
    assert.equal(blameCli.status, 0, blameCli.stderr);
    assert.equal(blameCli.stdout, `${blameText}\n`);
    assert.equal(runtime.inspectSnapshot(editRoot, "node-second-tag", false).payload.snapshot_id, secondSnapshot);
    const snapshotDiff = runtime.snapshotDiff(editRoot, "node-first-tag", "node-second-tag", { includeText: true, maxBytes: 1000 });
    assert.match(runtime.encodeJson(snapshotDiff), /fixture\.txt/);
    assert.ok(runtime.formatSnapshotDiff(snapshotDiff).length > 0);
    assert.ok(runtime.encodeJson(runtime.taskWorkflowSnapshotChain(editRoot, secondSnapshot)).includes(firstSnapshot));
    assert.ok(runtime.encodeJson(runtime.snapshotAncestry(editRoot, "node-second-tag")).includes(firstSnapshot));
    assert.equal(runtime.snapshotIsAncestor(editRoot, "node-first-tag", "node-second-tag").exitCode, 0);
    const mergeBase = runtime.snapshotMergeBase(editRoot, "node-first-tag", "node-second-tag");
    assert.equal(mergeBase.exitCode, 0);
    assert.ok(runtime.encodeJson(mergeBase.payload).includes(firstSnapshot));
    const shownTag = cli(editRoot, "tag", "show", "node-second-tag", "--json");
    assert.equal(shownTag.status, 0, shownTag.stderr);
    assert.deepEqual(runtime.decodeJson(shownTag.stdout), runtime.tagShow(editRoot, "node-second-tag"));
    assert.equal(runtime.tagList(editRoot).length, 3);
    for (const name of ["node-first-tag", "node-explicit-tag", "node-second-tag"]) {
      const deleted = runtime.tagDelete(editRoot, name);
      assert.equal(deleted.deleted, true);
      assert.ok(runtime.formatTagDelete(deleted).length > 0);
    }
    assert.deepEqual(runtime.tagList(editRoot), []);
    assert.equal(runtime.taskWorkflowSnapshotExists(editRoot, firstSnapshot), true);
    assert.equal(runtime.taskWorkflowSnapshotExists(editRoot, secondSnapshot), true);
    assert.throws(() => runtime.resolveSnapshotRef(editRoot, "node-first-tag"), nativeFailure);
    assert.throws(() => runtime.taskAbandon(root, taskId, { local: true }), (error) => {
      nativeFailure(error);
      assert.match(error.message, /is abandoned, but owned worktree cleanup is incomplete/);
      assert.match(error.message, /supplied explicitly; automatic deletion is not authorized/);
      return true;
    });
    assert.equal(runtime.taskWorkflowTaskShow(root, taskId, { local: true }).status, "abandoned");
    assert.ok((await readdir(editRoot)).length > 0);
    taskId = undefined;
  } finally {
    if (taskId) {
      try { runtime.taskAbandon(root, taskId, { local: true }); } catch { /* Explicit fixture worktree is retained. */ }
    }
  }
});

test("real JavaScript CLI reads state and renders native errors without another ait", async (context) => {
  const { root, runtime } = await repository(context);
  runtime.configureRepository(root, { plan_language: "ja-JP", agent_teams: "disabled" });
  const config = cli(root, "config", "show", "--json");
  assert.equal(config.status, 0, config.stderr);
  assert.equal(config.stderr, "");
  assert.deepEqual(runtime.decodeJson(config.stdout), runtime.taskWorkflowConfigShow(root));
  const status = cli(root, "status", "--json", "--full");
  assert.equal(status.status, 0, status.stderr);
  assert.equal(status.stderr, "");
  assert.equal(typeof runtime.decodeJson(status.stdout), "object");
  const absent = cli(root, "task", "show", "LNT-9999", "--local");
  assert.equal(absent.status, runtime.domainErrorExitCode());
  assert.equal(absent.stdout, "");
  assert.match(absent.stderr, /^Error: /);
});

test("repository-aware Plan APIs sync and query native authority without frontend facts", async (context) => {
  const { root, runtime } = await repository(context);
  await writeFile(path.join(root, "work.md"), [
    "# Node fixture Plan [plan-ref: node-fixture/work]",
    "",
    "- [ ] Preserve native Plan behavior. [ref: fixture/native-plan]",
    "",
  ].join("\n"));
  const synced = runtime.planSync(root, "work.md", { local: true });
  assert.equal(runtime.planSyncError(synced), null);
  assert.ok(Array.isArray(runtime.planList(root, { local: true })));
  const shown = runtime.planShow(root, "artifact:work.md", { local: true });
  assert.match(runtime.encodeJson(shown), /work\.md/);
  assert.ok(Array.isArray(runtime.planRevisions(root, "artifact:work.md", { local: true })));
  const items = runtime.planItems(root, "artifact:work.md", { local: true });
  assert.match(runtime.encodeJson(items), /fixture\/native-plan/);
  assert.ok(runtime.planInspect(root, "artifact:work.md", { local: true }));
  const candidates = runtime.planCandidates(root, { local: true, contains: "node-fixture/work" });
  assert.ok(candidates);
  assert.throws(() => runtime.planShow(root, "artifact:missing.md", { local: true }), nativeFailure);
  const result = cli(root, "plan", "show", "artifact:work.md", "--local", "--json");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(runtime.decodeJson(result.stdout), shown);
});

test("real Line APIs preserve native identity, local lifecycle and remote errors", async (context) => {
  const { root, runtime } = await repository(context);
  const inventory = runtime.lineInventory(root, { includeAll: true });
  const original = inventory.current_line;
  assert.equal(typeof original, "string");
  assert.deepEqual(inventory.payload, runtime.taskWorkflowLineList(root, { includeAll: true }));
  assert.deepEqual(runtime.lineShow(root), runtime.taskWorkflowLineShow(root));
  assert.ok(runtime.formatLineList(inventory, { includeAll: true }).length > 0);

  const created = runtime.taskWorkflowLineCreate(root, "node-fixture-line", { switch: true });
  assert.equal(created.line_name, "node-fixture-line");
  assert.ok(runtime.formatLineCreate(created).length > 0);
  assert.equal(runtime.lineInventory(root).current_line, "node-fixture-line");
  assert.throws(() => runtime.taskWorkflowLineArchive(root, "node-fixture-line"), nativeFailure);
  runtime.taskWorkflowLineSwitch(root, original);
  const renamed = runtime.taskWorkflowLineRename(root, "node-fixture-line", "node-fixture-renamed");
  assert.equal(renamed.new_line_name, "node-fixture-renamed");
  assert.ok(runtime.formatLineRename(renamed).length > 0);
  assert.equal(runtime.lineShow(root, "node-fixture-renamed").line_id, created.line_id);
  runtime.taskWorkflowLineSetHead(root, "node-fixture-renamed");
  assert.equal(runtime.lineShow(root, "node-fixture-renamed").head_snapshot_id, null);
  assert.throws(() => runtime.taskWorkflowLineSetHead(root, "node-fixture-renamed", "SNP-MISSING"), nativeFailure);
  const archived = runtime.taskWorkflowLineArchive(root, "node-fixture-renamed");
  assert.equal(archived.status, "archived");
  assert.ok(runtime.formatLineArchive(archived).length > 0);
  const deleted = runtime.taskWorkflowLineDelete(root, "node-fixture-renamed", { confirm: true });
  assert.equal(deleted.line_id, created.line_id);
  assert.ok(runtime.formatLineDelete(deleted).length > 0);
  assert.throws(() => runtime.lineShow(root, "node-fixture-renamed"), nativeFailure);
  assert.throws(() => runtime.lineShow(root, undefined, "missing-remote"), nativeFailure);
  assert.throws(() => runtime.lineMerge(root, { abortMerge: true }), nativeFailure);

  const cleanup = runtime.taskWorkflowLineCleanup(root, { idleFor: "7d" });
  assert.ok(runtime.formatLineCleanup(cleanup, "7d").length > 0);
  const shown = cli(root, "branch", "show", "--json");
  assert.equal(shown.status, 0, shown.stderr);
  assert.deepEqual(runtime.decodeJson(shown.stdout), runtime.lineShow(root));
});

test("real concrete repository APIs inspect remote state and preserve missing-remote errors", async (context) => {
  const { root, runtime } = await repository(context);
  assert.throws(() => runtime.repositoryShow(root), nativeFailure);
  const remoteRepository = { repository: { repository_index: 7, repository_name: path.basename(root), tombstoned: false } };
  const server = await planHttpFixture(context, {
    "GET /v1/handshake": { body: "{}" },
    "GET /v1/native/repository-authorities/7": { body: runtime.encodeJson(remoteRepository) },
  });
  const configPath = path.join(root, ".ait", "config.json");
  const config = runtime.decodeJson(await readFile(configPath, "utf8"));
  await writeFile(configPath, runtime.encodeJson({ ...config, repository_index: 7, default_remote: "origin",
    remotes: { origin: { url: server.baseUrl, repo_name: path.basename(root) } } }));
  const shown = runtime.repositoryShow(root);
  assert.deepEqual(shown, remoteRepository);
  assert.ok(runtime.formatRepositoryShow(shown).length > 0);
  for (const invoke of [
    () => runtime.repositoryShow(root, "missing-remote"),
    () => runtime.repositoryJobs(root, { remoteName: "missing-remote", state: "queued", limit: 1 }),
    () => runtime.repositoryCiCapabilities(root, "missing-remote"),
    () => runtime.repositoryRetire(root, "missing-remote", true),
    () => runtime.repositoryRestore(root, "missing-remote"),
  ]) assert.throws(invoke, nativeFailure);
  const result = cli(root, "repo", "show", "--json");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(runtime.decodeJson(result.stdout), shown);
  const missing = cli(root, "repo", "jobs", "--remote", "missing-remote", "--state", "queued", "--limit", "1", "--json");
  assert.equal(missing.status, runtime.domainErrorExitCode());
  assert.equal(missing.stdout, "");
  assert.ok(missing.stderr.length > 0);
});

test("real governance SDK counterparts retain template defaults, local queries and domain errors", async (context) => {
  const { root, runtime } = await repository(context);
  const template = runtime.taskWorkflowReviewCodeTemplate();
  assert.deepEqual(template, runtime.taskWorkflowReviewCodeTemplate({}));
  assert.ok(runtime.encodeJson(template).length > 0);
  assert.throws(() => runtime.taskWorkflowReviewCodeTemplate({ style: "unknown-template-style" }), nativeFailure);
  assert.deepEqual(runtime.taskWorkflowChangeList(root, { local: true }), []);
  for (const invoke of [
    () => runtime.taskWorkflowChangeShow(root, "LNT-9999/C-01", { local: true }),
    () => runtime.taskWorkflowPatchsetShow(root, "LNT-9999/P-01", { remoteName: "missing-remote" }),
    () => runtime.taskWorkflowReviewRequest(root, "LNT-9999/C-01", ["code_review"], { remoteName: "missing-remote" }),
    () => runtime.taskWorkflowAttestShow(root, "LNT-9999/P-01", { remoteName: "missing-remote" }),
    () => runtime.taskWorkflowPolicyEval(root, "LNT-9999/P-01", { remoteName: "missing-remote" }),
  ]) assert.throws(invoke, nativeFailure);
});

test("real Change CLI services preserve empty inventory and missing-work guards", async (context) => {
  const { root, runtime } = await repository(context);
  const raw = runtime.taskWorkflowChangeList(root, { local: true });
  assert.ok(runtime.projectChangeList(raw, true));
  assert.equal(typeof runtime.formatChangeList(raw, { local: true }), "string");
  for (const invoke of [
    () => runtime.changeCreate(root, "LNT-9999", "Missing Task revision", { local: true }),
    () => runtime.changeShow(root, "LNT-9999", { local: true }),
    () => runtime.changeRevert(root, "LNT-9999", { local: true, dryRun: true }),
    () => runtime.changeReplay(root, "LNT-9999", undefined, { local: true, dryRun: true }),
    () => runtime.changeClose(root, "LNT-9999", { local: true }),
    () => runtime.changePublish(root, "LNT-9999", "missing-remote"),
    () => runtime.patchsetPublish(root, "LNT-9999", "Missing Task fixture", { remoteName: "missing-remote" }),
    () => runtime.patchsetList(root, "LNT-9999", "missing-remote"),
  ]) assert.throws(invoke, nativeFailure);
  const listed = cli(root, "change", "list", "--local", "--all", "--json");
  assert.equal(listed.status, 0, listed.stderr);
  assert.deepEqual(runtime.decodeJson(listed.stdout), runtime.projectChangeList(raw, true));
  const missing = cli(root, "change", "show", "LNT-9999", "--local");
  assert.equal(missing.status, runtime.domainErrorExitCode());
  assert.equal(missing.stdout, "");
  assert.ok(missing.stderr.length > 0);
  const syntax = cli(root, "patchset", "show", "123", "--json");
  assert.equal(syntax.status, 2);
  assert.equal(syntax.stdout, "");
});

test("real workspace lock handles retain metadata and release idempotently", async (context) => {
  const { root, runtime } = await repository(context);
  assert.equal(path.isAbsolute(runtime.workspaceCommandLockPath(root)), true);
  const lock = new WorkspaceCommandLock(root, "Node native lock fixture", runtime);
  const metadata = lock.metadata();
  try {
    assert.equal(lock.isClosed(), false);
    assert.ok(runtime.encodeJson(metadata).length > 0);
    lock.close();
    lock.close();
    lock.release();
    assert.equal(lock.isClosed(), true);
    assert.deepEqual(lock.metadata(), metadata);
  } finally {
    lock.close();
  }
  const reacquired = new WorkspaceCommandLock(root, "Node reacquired lock fixture", runtime);
  try {
    assert.equal(reacquired.isClosed(), false);
  } finally {
    reacquired.release();
  }
  assert.equal(reacquired.isClosed(), true);
});

test("real Snapshot operations retain missing-selector and workspace guards", async (context) => {
  const { root, runtime } = await repository(context);
  assert.equal(runtime.taskWorkflowSnapshotExists(root, "SNP-000000000000"), false);
  for (const invoke of [
    () => runtime.taskWorkflowSnapshotChain(root, "SNP-000000000000"),
    () => runtime.snapshotDiff(root, "missing-old-tag", "missing-new-tag"),
    () => runtime.snapshotRevert(root, "missing-tag", { dryRun: true }),
    () => runtime.snapshotReplay(root, "missing-tag", undefined, { dryRun: true }),
    () => runtime.taskWorkflowSnapshotRestoreLines(root, "SNP-000000000000", "missing.txt", { line: 1 }),
  ]) assert.throws(invoke, nativeFailure);
  const diff = cli(root, "snapshot", "diff", "missing-old-tag", "missing-new-tag", "--json");
  assert.equal(diff.status, runtime.domainErrorExitCode());
  assert.equal(diff.stdout, "");
  assert.ok(diff.stderr.length > 0);
});

test("real worktree SDK reads native inventory and retains missing-work guards", async (context) => {
  const { root, runtime } = await repository(context);
  const inventory = runtime.taskWorkflowWorktreeList(root, { refreshStatus: false });
  assert.ok(runtime.encodeJson(inventory).length > 0);
  const doctor = runtime.taskWorkflowWorktreeDoctor(root, { refreshStatus: false });
  assert.ok(runtime.encodeJson(doctor).length > 0);
  for (const invoke of [
    () => runtime.taskWorkflowWorktreeGet(root, { name: "missing-worktree", refreshStatus: false }),
    () => runtime.taskWorkflowWorktreeBindExisting(root, "missing-worktree", { taskId: "LNT-9999" }),
    () => runtime.taskWorkflowWorktreePreviewRebase(root, { name: "missing-worktree", ontoLineName: "main" }),
    () => runtime.taskWorkflowWorktreeRestore(root, { name: "missing-worktree", paths: ["../outside.txt"], dryRun: true }),
  ]) assert.throws(invoke, nativeFailure);
});

test("real Auth APIs retain native local identity fallback and remote errors", async (context) => {
  const { root, runtime } = await repository(context);
  const identity = runtime.taskWorkflowAuthWhoami(root);
  assert.ok(runtime.formatAuthWhoami(identity).length > 0);
  assert.throws(() => runtime.taskWorkflowAuthBindings(root, { remoteName: "missing-remote" }), nativeFailure);
  assert.throws(() => runtime.taskWorkflowAuthGrant(root, "fixture@example.test", ["code_review"], {
    remoteName: "missing-remote",
  }), nativeFailure);
  const whoami = cli(root, "auth", "whoami", "--json");
  assert.equal(whoami.status, 0, whoami.stderr);
  assert.equal(typeof runtime.decodeJson(whoami.stdout), "object");
});

test("real Stash inventory and missing identity errors retain native behavior", async (context) => {
  const { root, runtime } = await repository(context);
  assert.deepEqual(runtime.taskWorkflowStashList(root), []);
  assert.throws(() => runtime.taskWorkflowStashShow(root, "stash-missing"), nativeFailure);
  assert.throws(() => runtime.taskWorkflowStashDrop(root, "stash-missing"), nativeFailure);
  const listed = cli(root, "stash", "list", "--json");
  assert.equal(listed.status, 0, listed.stderr);
  assert.deepEqual(runtime.decodeJson(listed.stdout), []);
});

test("real Tag inventory and missing-reference errors retain native behavior", async (context) => {
  const { root, runtime } = await repository(context);
  assert.deepEqual(runtime.tagList(root), []);
  assert.throws(() => runtime.tagShow(root, "missing-tag"), nativeFailure);
  assert.throws(() => runtime.tagDelete(root, "missing-tag"), nativeFailure);
  assert.throws(() => runtime.resolveSnapshotRef(root, "missing-tag"), nativeFailure);
  const listed = cli(root, "tag", "list", "--json");
  assert.equal(listed.status, 0, listed.stderr);
  assert.deepEqual(runtime.decodeJson(listed.stdout), []);
});

test("real Review validators preserve scalar normalization and leave identity I/O to operations", async (context) => {
  const { root, runtime } = await repository(context);
  assert.equal(runtime.validatePublicPatchsetArgument("  LNT-7/P-01  "), "LNT-7/P-01");
  assert.equal(runtime.validateReviewPatchsetSelector(" LNT-7/P-01 "), "LNT-7/P-01");
  const selector = `@${path.join(root, "nonexistent identity.json")}`;
  assert.equal(runtime.validateReviewPatchsetSelector(selector), selector);
  for (const value of ["", "123", " 123 "]) {
    assert.throws(() => runtime.validatePublicPatchsetArgument(value), nativeFailure);
  }
  for (const value of ["@", "@relative.json", ` ${selector}`, `${selector} `]) {
    assert.throws(() => runtime.validateReviewPatchsetSelector(value), nativeFailure);
  }
  const invalidSelector = cli(root, "review", "task", "comment", "LNT-9999", "--patchset", "@relative.json", "--json");
  assert.equal(invalidSelector.status, 2);
  assert.equal(invalidSelector.stdout, "");
  const missingIdentity = cli(root, "review", "task", "comment", "LNT-9999", "--patchset", selector, "--json");
  assert.equal(missingIdentity.status, runtime.domainErrorExitCode());
  assert.equal(missingIdentity.stdout, "");
  assert.ok(missingIdentity.stderr.length > 0);
});

test("real Review CLI services retain native templates and candidate/Team guards", async (context) => {
  const { root, runtime } = await repository(context);
  const template = runtime.taskWorkflowReviewCodeTemplate({ style: "inline" });
  const rendered = runtime.formatReviewCodeTemplate(template);
  assert.ok(rendered.length > 0);
  const cliTemplate = cli(root, "review", "code", "template", "--style", "inline");
  assert.equal(cliTemplate.status, 0, cliTemplate.stderr);
  assert.equal(cliTemplate.stdout, `${rendered}\n`);
  for (const invoke of [
    () => runtime.reviewShow(root, "LNT-9999", "missing-remote"),
    () => runtime.reviewTeamRequest(root, "LNT-9999", ["code_review"], { remoteName: "missing-remote" }),
    () => runtime.reviewTeamApprove(root, "LNT-9999", { remoteName: "missing-remote" }),
    () => runtime.reviewTaskApprove(root, "LNT-9999", "LNT-9999/P-01", "Missing candidate", "missing-remote"),
    () => runtime.reviewCodeSubmit(root, "LNT-9999", "LNT-9999/P-01", "Missing candidate", { remoteName: "missing-remote" }),
  ]) assert.throws(invoke, nativeFailure);
});

test("real Attestation and Policy CLI calls retain native candidate and remote guards", async (context) => {
  const { root, runtime } = await repository(context);
  assert.throws(() => runtime.taskWorkflowAttestPut(root, {
    patchsetId: "LNT-9999/P-01", tests: "pass", remoteName: "missing-remote",
  }), nativeFailure);
  for (const args of [
    ["attest", "show", "LNT-9999/P-01"],
    ["policy", "eval", "LNT-9999/P-01"],
    ["policy", "show", "LNT-9999/P-01"],
    ["policy", "waive", "LNT-9999/P-01", "--rule", "native-rule", "--reason", "Missing candidate fixture"],
  ]) {
    const result = cli(root, ...args, "--remote", "missing-remote", "--json");
    assert.equal(result.status, runtime.domainErrorExitCode());
    assert.equal(result.stdout, "");
    assert.ok(result.stderr.length > 0);
  }
});

test("real remote sync operations retain missing-remote errors and raw SDK guards", async (context) => {
  const { root, runtime } = await repository(context);
  for (const invoke of [
    () => runtime.pullRepository(root, { remoteName: "missing-remote" }),
    () => runtime.pushRepository(root, { remoteName: "missing-remote" }),
    () => runtime.taskWorkflowPull(root, { remoteName: "missing-remote", merge: true, restore: false }),
    () => runtime.taskWorkflowPush(root, { remoteName: "missing-remote" }),
    () => runtime.taskWorkflowUploadSnapshotChain(root, "SNP-000000000000", { remoteName: "missing-remote", reason: "Missing remote fixture" }),
  ]) assert.throws(invoke, nativeFailure);
  for (const command of ["pull", "push"]) {
    const result = cli(root, command, "--remote", "missing-remote", "--json");
    assert.equal(result.status, runtime.domainErrorExitCode());
    assert.equal(result.stdout, "");
    assert.ok(result.stderr.length > 0);
  }
});

test("real Worktree CLI services retain inventory, confirmations and recovery guards", async (context) => {
  const { root, runtime } = await repository(context);
  const listed = cli(root, "worktree", "list", "--json");
  assert.equal(listed.status, 0, listed.stderr);
  assert.deepEqual(runtime.decodeJson(listed.stdout), runtime.taskWorkflowWorktreeList(root, { refreshStatus: false }));
  for (const invoke of [
    () => runtime.worktreeCleanup(root, { olderThan: "7d", dryRun: false, yes: false }),
    () => runtime.worktreePruneStale(root, { dryRun: false, yes: false }),
    () => runtime.worktreeRemove(root, [], { dryRun: false, yes: false }),
    () => runtime.worktreeRecoverTask(root, "LNT-9999", { dryRun: true }),
    () => runtime.worktreeRebase(root, { continueRebase: true, abortRebase: true }),
  ]) assert.throws(invoke, nativeFailure);
  assert.equal(runtime.worktreeSyncExitCode({ ok: false }, true), 2);
  assert.equal(runtime.worktreeSyncExitCode({ ok: false }, false), 0);
  assert.equal(runtime.worktreeSyncExitCode({ ok: true }, true), 0);
});

test("real Git import dry-run reads a genuine local Git repository without importing Snapshots", async (context) => {
  const { root, runtime } = await repository(context);
  const parent = path.dirname(root);
  const source = path.join(parent, "Git source with spaces");
  const emptyConfig = path.join(parent, "empty-git-config");
  const hooksRoot = path.join(parent, "empty-git-hooks");
  await mkdir(source);
  await mkdir(hooksRoot);
  await writeFile(emptyConfig, "");
  function git(...args) {
    const result = spawnSync("git", [
      "-c", `core.hooksPath=${hooksRoot}`, "-c", "core.autocrlf=false",
      "-c", "user.name=Node fixture", "-c", "user.email=fixture@example.test",
      "-c", "commit.gpgSign=false", ...args,
    ], {
      cwd: source, encoding: "utf8", windowsHide: true,
      env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: emptyConfig },
    });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
  }
  git("init", "-q");
  await writeFile(path.join(source, "sample.txt"), "Native Git interoperability fixture\n");
  git("add", "--", "sample.txt");
  git("commit", "-q", "-m", "Native Git fixture");
  const before = runtime.taskWorkflowSnapshotList(root);
  const preview = runtime.gitImport(root, source, { dryRun: true });
  assert.ok(runtime.formatGitImport(preview).length > 0);
  const imported = cli(root, "git", "import", source, "--dry-run", "--json");
  assert.equal(imported.status, 0, imported.stderr);
  assert.equal(typeof runtime.decodeJson(imported.stdout), "object");
  assert.deepEqual(runtime.taskWorkflowSnapshotList(root), before);
});

test("real content maintenance reports use native validation status and preview operations", async (context) => {
  const { root, runtime } = await repository(context);
  const stats = runtime.gcStats(root);
  assert.ok(runtime.formatGcStats(stats).length > 0);
  const validation = runtime.gcValidate(root);
  assert.equal(typeof validation.needs_attention, "boolean");
  const validated = cli(root, "gc", "validate", "--json");
  assert.equal(validated.status, runtime.gcValidateExitCode(validation), validated.stderr);
  assert.equal(runtime.decodeJson(validated.stdout).needs_attention, validation.needs_attention);
  const preview = runtime.gcPrune(root);
  assert.ok(runtime.formatGcPrune(preview).length > 0);
  const pruned = cli(root, "gc", "prune", "--json");
  assert.equal(pruned.status, 0, pruned.stderr);
  assert.equal(typeof runtime.decodeJson(pruned.stdout), "object");
  assert.equal(runtime.gcValidateExitCode({ needs_attention: true }), 1);
  assert.equal(runtime.gcValidateExitCode({ needs_attention: false }), 0);
  assert.throws(() => runtime.gcValidateExitCode({}), nativeFailure);
  assert.throws(() => runtime.gcValidateExitCode({ needs_attention: "invalid" }), nativeFailure);
});

test("real retained policy helpers preserve YAML roundtrips and native provenance validation", () => {
  const runtime = new NativeRuntime();
  for (const name of runtime.policyProfileNames()) {
    const policy = runtime.policyProfile(name);
    assert.deepEqual(runtime.parsePolicyYaml(runtime.policyToYaml(policy)), runtime.normalizePolicy(policy));
    assert.equal(typeof runtime.resolveEffectivePolicy(policy), "object");
  }
  for (const mode of runtime.authorModeValues()) {
    assert.equal(runtime.normalizeAuthorMode(mode), mode);
    assert.equal(runtime.buildMinimumProvenance(mode, "Node fixture").length, 2);
  }
  assert.equal(runtime.renderCodeReviewSummaryTemplate(), runtime.renderCodeReviewSummaryTemplate("inline"));
  assert.ok(runtime.missingCodeReviewSummarySections(null).length > 0);
  assert.equal(runtime.isStructuredCodeReviewSummary(null), false);
  assert.throws(() => runtime.normalizeAuthorMode("invented-author-mode"), nativeFailure);
  assert.throws(() => runtime.policyProfile("invented-profile"), nativeFailure);
  assert.throws(() => runtime.parsePolicyYaml("["), nativeFailure);
});

test("real workflow identity helpers retain native status and checked numeric semantics", () => {
  const runtime = new NativeRuntime();
  const id = runtime.generateNamespacedSequenceId("T", 17n, { namespacePrefix: "R", width: 4n });
  assert.equal(id, "RT-0017");
  assert.equal(runtime.workflowIdMatches("T", id, { namespacePrefix: "R", includeLegacy: false }), true);
  assert.equal(runtime.workflowIdMatchesAnyNamespacePrefix("T", id, { namespacePrefix: "R" }), true);
  assert.equal(runtime.workflowIdNamespacePrefixForValue("T", id, { namespacePrefix: "R" }), "R");
  assert.equal(runtime.normalizeOptionalText("  retained text\n"), "retained text");
  assert.equal(runtime.normalizeOptionalText(" \t"), null);
  assert.equal(runtime.normalizeTaskStatus("completed"), "completed");
  assert.equal(runtime.taskStatusDetails("completed").closed, true);
  assert.equal(runtime.workflowSuccessEnvelope("fixture", "native value").value, "native value");
  assert.equal(runtime.workflowErrorEnvelope("fixture", "FIXTURE", "native message", "native detail").error.detail, "native detail");
  assert.match(runtime.utcNow(), /^\d{4}-\d{2}-\d{2}T/);
  for (const value of [9223372036854775808n, -9223372036854775809n, 9007199254740992]) {
    assert.throws(() => runtime.generateNamespacedSequenceId("T", value), Error);
  }
  assert.throws(() => runtime.generateNamespacedSequenceId("T", 1, { width: 18446744073709551616n }), Error);
});

test("real Plan SDK parses sections and preserves checked item line numbers", () => {
  const runtime = new NativeRuntime();
  const markdown = "# Native fixture [plan-ref: native/root]\n\n- [ ] First item. [ref: native/first]\n- [x] Done item. [ref: native/done]\n";
  const parsed = runtime.parsePlanMarkdown(markdown);
  assert.equal(parsed.plan_ref_count, 1);
  assert.equal(parsed.item_count, 2);
  assert.deepEqual(runtime.extractPlanRefs(parsed).plan_refs, runtime.listPlanSectionRefs(markdown));
  assert.deepEqual(runtime.extractPlanItems(markdown), parsed.items);
  const section = runtime.extractPlanSection(markdown, "native/root");
  assert.equal(section.section_markdown, markdown.trim());
  assert.deepEqual(section.items, parsed.items);
  assert.equal(runtime.findPlanItem(markdown, "native/first").checkbox_state, "open");
  assert.equal(runtime.findPlanItem(markdown, "native/missing"), null);
  const normalized = runtime.normalizePlanItems([{ plan_item_ref: "native/wide", line_number: 9007199254740993n }]);
  assert.equal(normalized[0].line_number, 9007199254740993n);
  assert.deepEqual(runtime.findPlanItemInItems(normalized, "native/wide"), normalized[0]);
  assert.deepEqual(runtime.normalizePlanItems(null), []);
  assert.throws(() => runtime.normalizePlanItems([{ plan_item_ref: "native/bad", line_number: 9223372036854775808n }]), Error);
  assert.throws(() => runtime.extractPlanRefs({ items: [{ plan_item_ref: "native/bad", heading_path: [false] }] }), Error);
});

test("real Plan dispatch keeps composite link identities and native taskability decisions", () => {
  const runtime = new NativeRuntime();
  const plan = {
    plan_id: "PL-1", title: "Native fixture", status: "draft", repo_name: "fixture",
    publication_state: "published", published_plan_id: "PL-1", published_head_revision_id: "PR-1", head_revision_id: "PR-2",
    head_revision: { plan_revision_id: "PR-2", revision_number: 2, artifact_path: "docs/sprints/native.md",
      artifact_selector: "native/root", publication_state: "local_draft", items: [
        { plan_item_ref: "native/linked", text: "Linked", checkbox_state: "open", line_number: 3 },
        { plan_item_ref: "native/free", text: "Free", checkbox_state: "open", line_number: 4 },
        { plan_item_ref: "native/done", text: "Done", checkbox_state: "done", line_number: 5 },
      ] },
  };
  const tasks = [
    { task_id: "RT-1", plan_id: "PL-1", plan_item_ref: "native/linked", status: "active", planning_state: "planned" },
    { task_id: "RT-2", plan_id: "PL-2", plan_item_ref: "native/linked", status: "active", planning_state: "planned" },
  ];
  const indexes = runtime.planTaskLinkIndexes(tasks);
  assert.equal(indexes.by_item.length, 2);
  assert.deepEqual(indexes.by_item.map((row) => [row.plan_id, row.plan_item_ref, row.tasks[0].task_id]),
    [["PL-1", "native/linked", "RT-1"], ["PL-2", "native/linked", "RT-2"]]);
  assert.equal(indexes.by_plan["PL-2"][0].task_id, "RT-2");
  assert.equal(runtime.planItemsPayload(plan).dispatch_validation_required, true);
  assert.equal(runtime.localPlanPublishShadow(plan).unpublished_head, true);
  assert.equal(runtime.planDispatchSummary(plan, { tasks }).taskable_item_count, 1);
  assert.deepEqual(runtime.computeTaskableItems(plan, { tasks }).map((row) => row.plan_item_ref), ["native/free"]);
  const linked = runtime.validateDispatchLegality(plan, "native/linked", { tasks });
  assert.equal(linked.taskable, false);
  assert.equal(linked.taskable_blocker, "linked_task_exists");
  assert.equal(runtime.validateDispatchLegality(plan, "native/free", { tasks }).taskable, true);
  assert.equal(typeof runtime.planCandidatesPayload([runtime.planDispatchSummary(plan, { tasks })], { includeAll: true }), "object");
  assert.deepEqual(runtime.planTaskLinkIndexes(null), { by_item: [], by_plan: {} });
  assert.throws(() => runtime.planDispatchSummary({ head_revision: { revision_number: 9223372036854775808n } }), nativeFailure);
});

test("real manifest SDK retains byte diffs, mode changes and binary classification", () => {
  const runtime = new NativeRuntime();
  const oldFiles = { "a.txt": { path: "a.txt", blob_id: "OLD", size_bytes: 10, mode: "0o644" },
    "mode.txt": { path: "mode.txt", blob_id: "MODE", size_bytes: 4, mode: "0o644" } };
  const newFiles = { "a.txt": { path: "a.txt", blob_id: "NEW", size_bytes: 10, mode: "0o755" },
    "mode.txt": { path: "mode.txt", blob_id: "MODE", size_bytes: 4, mode: "0o755" } };
  const summary = runtime.diffSnapshotManifests(oldFiles, newFiles, { oldSnapshotId: "S1", newSnapshotId: "S2" });
  assert.deepEqual(summary.modified, ["a.txt"]);
  assert.deepEqual(summary.mode_changed, ["mode.txt"]);
  const options = { includeText: true, maxBytes: 1000n,
    blobBytesById: { OLD: Buffer.from("hello\nfoo\n"), NEW: Buffer.from("hello\nbar\n"), MODE: Buffer.from("mode") } };
  const text = runtime.snapshotDiffFromManifests(oldFiles, newFiles, options);
  assert.equal(text.files[0].diff.status, "text");
  assert.match(text.files[0].diff.text, /\+bar/);
  const binary = runtime.snapshotDiffFromManifests(oldFiles, newFiles, {
    ...options, blobBytesById: { OLD: Buffer.from("hello\nfoo\n"), NEW: Buffer.from([0, 255, 1]), MODE: Buffer.from("mode") },
  });
  assert.equal(binary.files[0].diff.status, "binary");
  assert.throws(() => runtime.snapshotDiffFromManifests(oldFiles, newFiles, { maxBytes: 18446744073709551616n }), Error);
});

test("real HTTP normalizers retain native JSON validation and opaque object fields", () => {
  const runtime = new NativeRuntime();
  for (const [method, payload] of [["normalizeTaskWorkflowHttpCompatibility", { compatible: true, custom: { retained: true } }],
    ["normalizeTaskWorkflowHttpReadiness", { ready: true, custom: { retained: true } }]]) {
    assert.deepEqual(runtime[method](runtime.encodeJson(payload)), payload);
    assert.throws(() => runtime[method]("{"), nativeFailure);
    assert.throws(() => runtime[method]("[]"), nativeFailure);
  }
});

test("real source cache contract uses explicit roots and missing-artifact freshness reports", async (context) => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "ait-node-cache-"));
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const runtime = new NativeRuntime();
  const coreRoot = path.join(temporary, "Core source");
  await mkdir(coreRoot);
  const request = { namespaceRoot: path.join(temporary, "cache"), coreRepoRoot: coreRoot,
    extSuffix: ".node", coreSourceFingerprint: "native-fixture" };
  const contract = runtime.currentSourceNativeCacheContract(request);
  assert.ok(contract.cache_root.startsWith(contract.namespace_root + path.sep));
  assert.equal(contract.core_source_fingerprint, "native-fixture");
  const nativeCli = cli(temporary, "current-source-cache", "contract", "--namespace-root", request.namespaceRoot,
    "--core-repo-root", coreRoot, "--ext-suffix", ".node", "--core-source-fingerprint", "native-fixture", "--json");
  assert.equal(nativeCli.status, 0, nativeCli.stderr);
  assert.deepEqual(runtime.decodeJson(nativeCli.stdout), contract);
  const missing = { metadataPath: path.join(temporary, "missing.json"), extensionPath: path.join(temporary, "missing.node"),
    sourceMtimeNs: 18446744073709551615n, sourceFingerprint: "native-fixture" };
  assert.equal(runtime.currentSourceCacheExtensionFresh(missing).fresh, false);
  assert.throws(() => runtime.currentSourceCacheExtensionFresh({ ...missing, sourceMtimeNs: 18446744073709551616n }), Error);
  assert.throws(() => runtime.currentSourceCacheMarkBuilding(request, 1, "not JSON"), nativeFailure);
  const invalid = cli(temporary, "current-source-cache", "mark-building", "--namespace-root", request.namespaceRoot,
    "--core-repo-root", coreRoot, "--ext-suffix", ".node", "--source-mtime-ns", "1", "--extra-json", "not JSON", "--json");
  assert.equal(invalid.status, runtime.domainErrorExitCode());
  assert.equal(invalid.stdout, "");
  assert.ok(invalid.stderr.length > 0);
});


test("real filesystem helpers preserve binary and ZIP bytes, JSON integers and path guards", async (context) => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "ait-node-filesystem-"));
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const runtime = new NativeRuntime();
  const bytes = Buffer.from([0, 255, 128, 10, 13]);
  const binaryPath = path.join(temporary, "fixture.bin");
  const textPath = path.join(temporary, "fixture.txt");
  const jsonPath = path.join(temporary, "fixture.json");
  const zipPath = path.join(temporary, "fixture.zip");
  await writeFile(binaryPath, bytes);
  await writeFile(textPath, "one\r\ntwo\rthree\n");
  await writeFile(jsonPath, '{"wide":18446744073709551615}');
  await writeFile(zipPath, Buffer.from("UEsDBBQAAAAAAPkNWl1bfkx9BQAAAAUAAAALAAAAZml4dHVyZS5iaW4A/4AKDVBLAQIUAxQAAAAAAPkNWl1bfkx9BQAAAAUAAAALAAAAAAAAAAAAAACAAQAAAABmaXh0dXJlLmJpblBLBQYAAAAAAQABADkAAAAuAAAAAAA=", "base64"));
  const binary = runtime.planFilesystemReadBinaryFile(binaryPath);
  assert.ok(Buffer.isBuffer(binary));
  assert.deepEqual(binary, bytes);
  assert.equal(runtime.planFilesystemReadUtf8TextFile(textPath), "one\ntwo\nthree\n");
  assert.equal(runtime.planFilesystemReadJsonFile(jsonPath).wide, 18446744073709551615n);
  assert.equal(runtime.planFilesystemZipArchiveHasMember(zipPath, "fixture.bin"), true);
  assert.equal(runtime.planFilesystemZipArchiveHasMember(zipPath, "absent.bin"), false);
  assert.deepEqual(runtime.planFilesystemReadZipArchiveMember(zipPath, "fixture.bin"), bytes);
  assert.throws(() => runtime.planFilesystemReadZipArchiveMember(zipPath, "absent.bin"), nativeFailure);
  assert.throws(() => runtime.planFilesystemReadUtf8TextFile(binaryPath), nativeFailure);
  assert.throws(() => runtime.planFilesystemResolveRepoArtifactPath(temporary, "../outside.txt", true), nativeFailure);
  assert.throws(() => runtime.planFilesystemResolveRepoArtifactPath(temporary, ".ait/config.json", true), nativeFailure);
  assert.equal(runtime.planFilesystemResolveRepoArtifactPath(temporary, "fixture.bin").artifact_path, "fixture.bin");
  assert.equal(runtime.planFilesystemNormalizeMarkdownArtifactPath(String.raw`docs\plan.md`), "docs/plan.md");
  assert.equal(runtime.planFilesystemIsMarkdownArtifactPath("docs/PLAN.MD"), true);
  assert.equal(runtime.planFilesystemIsLineageOnlyMarkdownArtifactPath("docs/plan.md"), true);
  assert.equal(runtime.planFilesystemWorkspacePathIsIgnored(temporary, "fixture.bin", "*.bin\n"), true);
  const visible = runtime.planFilesystemListVisibleWorkspacePaths(temporary, { ignoreRulesText: "*.bin\n" });
  assert.ok(visible.includes("fixture.txt"));
  assert.ok(!visible.includes("fixture.bin"));
  for (const suffix of ["RepoConfigJson", "WorktreeConfigJson", "WorktreeMetadataJson"]) {
    const statePath = path.join(temporary, `${suffix}.json`);
    const payload = { minimum: -9223372036854775808n, maximum: 18446744073709551615n, bytes_are_separate: true };
    assert.equal(runtime[`repoStateWrite${suffix}`](statePath, payload), undefined);
    assert.deepEqual(runtime[`repoStateRead${suffix}`](statePath), payload);
    await writeFile(statePath, "{");
    assert.throws(() => runtime[`repoStateRead${suffix}`](statePath), nativeFailure);
  }
  for (const name of ["path/with spaces", "中文/branch", "percent%sign"]) {
    assert.equal(runtime.decodeRefName(runtime.encodeRefName(name)), name);
  }
  assert.equal(runtime.resolveRuntimeDataRootWithSource(temporary)[0], runtime.resolveRuntimeDataRoot(temporary));
  assert.equal(runtime.resolveServerRuntimeRootWithSource(temporary)[0], runtime.resolveServerRuntimeRoot(temporary));
});

test("real artifact matching retains native content identity and publication semantics", () => {
  const runtime = new NativeRuntime();
  const body = "# Demo\n";
  assert.equal(runtime.artifactBlobId(body), "BLB-31ca6c61ca3fcc54029a");
  const plan = { title: "Demo", published_head_revision_id: "PR-REMOTE-1", publication_state: "published",
    head_revision: { publication_state: "published", artifact_path: "docs/sprints/demo.md", artifact_selector: "demo/root",
      artifact_heading: "Demo", artifact_blob_id: runtime.artifactBlobId(body), items: [{ plan_item_ref: "demo/item" }] } };
  const artifact = { artifact_path: "docs/sprints/demo.md", artifact_selector: "demo/root", artifact_heading: "Demo",
    artifact_body: body, items: [{ plan_item_ref: "demo/item" }] };
  assert.equal(runtime.localPlanFullyPublished(plan), true);
  assert.equal(runtime.planHeadsEquivalent(plan, plan), true);
  assert.equal(runtime.planMatchesSyncArtifact(plan, artifact), true);
  assert.equal(runtime.planMatchesSyncArtifact({ ...plan, title: "Other title" }, artifact), false);
  assert.equal(runtime.planMatchesSyncArtifact({ ...plan, title: "Other title" }, artifact, false), true);
  assert.deepEqual(runtime.artifactCandidatesOpen([{ plan_id: "PL-1", status: "draft" }, { plan_id: "PL-2", status: "archived" }]),
    [{ plan_id: "PL-1", status: "draft" }]);
  const plans = [{ ...plan, plan_id: "PL-1" }];
  assert.equal(runtime.indexPlansByArtifactPath(plans)["docs/sprints/demo.md"][0].plan_id, "PL-1");
  assert.equal(runtime.indexPlansByArtifactIdentity(plans).length, 1);
  assert.equal(runtime.openPlansMatchingSelector(plans, "demo/root").length, 1);
  assert.throws(() => runtime.indexPlansByArtifactIdentity({ invalid: "array required" }), nativeFailure);
});

test("real pack SDK round-trips binary deltas and persists object and tree pack indexes", async (context) => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "ait-node-pack-"));
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const runtime = new NativeRuntime();
  const base = Buffer.from(Array.from({ length: 1024 }, (_, index) => index % 256));
  const target = Buffer.concat([base.subarray(0, 300), Buffer.from([255, 0, 128, 3]), base.subarray(304), Buffer.from("tail")]);
  const delta = runtime.planPackSubstrateBuildGitBinaryDelta(base, target);
  assert.ok(Buffer.isBuffer(delta));
  assert.deepEqual(runtime.planPackSubstrateApplyGitBinaryDelta(base, delta), target);
  assert.deepEqual(runtime.planPackSubstrateApplyPackDelta(base, delta, "git-binary-v1"), target);
  assert.throws(() => runtime.planPackSubstrateApplyGitBinaryDelta(base, Buffer.from([255])), nativeFailure);
  assert.throws(() => runtime.planPackSubstrateApplyPackDelta(base, delta, "unknown-algorithm"), nativeFailure);
  const member = runtime.planPackSubstrateBuildGitBinaryDeltaMember("blobs/BLB-TARGET", "BLB-TARGET", "BLB-BASE", base, target, 1n);
  assert.deepEqual(member.logical_data, [...target]);
  assert.equal(member.base_blob_id, "BLB-BASE");
  const members = runtime.planPackSubstrateBuildPackMembers([
    { entry_name: "blobs/BLB-BASE", blob_id: "BLB-BASE", data: [...base], path_hint: "fixture.bin" },
    { entry_name: "blobs/BLB-TARGET", blob_id: "BLB-TARGET", data: [...target], path_hint: "fixture.bin" },
  ]);
  const objectPath = path.join(temporary, "objects.zstpack");
  runtime.planPackSubstrateWritePackArchive(objectPath, "PCK-NODE", "2026-10-26T00:00:00+00:00", members);
  assert.equal(runtime.planPackSubstrateReadPackIndex(objectPath).pack_id, "PCK-NODE");
  assert.equal(runtime.planPackSubstratePackHasEntry(objectPath, "blobs/BLB-BASE"), true);
  assert.equal(runtime.planPackSubstratePackHasEntry(objectPath, "blobs/BLB-MISSING"), false);
  const entries = [
    { path: "a.txt", blob_id: "BLB-A", size_bytes: 1, mode: "0o644" },
    { path: "nested/b.bin", blob_id: "BLB-B", size_bytes: 4, mode: "0o755" },
  ];
  const records = runtime.planPackSubstrateBuildTreeRecords(entries);
  // Producer golden captured for the retained Python storage identity contract.
  assert.equal(records.root_tree_id, "TRE-59B20845E41B99DD6AF0");
  assert.equal(records.tree_entry_rows.length, 3);
  assert.deepEqual(runtime.planPackSubstrateBuildSnapshotId({
    repoName: "fixture", lineName: "main", rootTreeId: records.root_tree_id, message: "SDK",
  }), { snapshot_id: "SNP-2E95AF56F25D", revision_hash: "2e95af56f25d3a1dbc7c62ace62f6bf7fe5bf3f590d6097eae0904a1d85a62e3" });
  const treePath = path.join(temporary, "trees.zstpack");
  const treeMembers = runtime.planPackSubstrateBuildTreePackMembers(records.tree_rows, records.tree_entry_rows);
  runtime.planPackSubstrateWriteTreePackArchive(treePath, "TPK-NODE", "2026-10-26T00:00:00+00:00", treeMembers);
  const index = runtime.planPackSubstrateReadTreePackIndex(treePath);
  assert.equal(index.trees.length, records.tree_rows.length);
  const rows = runtime.planPackSubstrateReadTreePackTree(treePath, records.root_tree_id);
  assert.deepEqual(rows.map((row) => row.entry_name), ["a.txt", "nested"]);
  for (let ordinal = 0; ordinal < index.trees.length; ordinal += 1) {
    const physical = runtime.planPackSubstrateReadTreePackTreeByOrdinal(treePath, BigInt(ordinal));
    assert.equal(physical.entry_ordinal, ordinal);
    assert.deepEqual(physical.rows, runtime.planPackSubstrateReadTreePackTree(treePath, physical.tree_id));
  }
  assert.throws(() => runtime.planPackSubstrateReadTreePackTreeByOrdinal(treePath, 18446744073709551616n), Error);
  assert.throws(() => runtime.planPackSubstrateBuildTreeRecords([
    { path: "a", blob_id: "BLB-X", mode: "0o644" }, { path: "a/b", blob_id: "BLB-Y", mode: "0o644" },
  ]), nativeFailure);
  assert.throws(() => runtime.planPackSubstrateBuildPackMembers([], { maxDeltaChainDepth: -1n }), Error);
  const validation = runtime.planPackSubstrateBuildStorageValidationSummary({
    packedBlobCount: 4n, packedFullBlobCount: 4n, packedDeltaBlobCount: 0n,
    packCount: 1n, packIndexErrorCount: 0n, storageSavingsRatio: 0.5, unreferencedTreeCount: 1n,
  });
  assert.equal(validation.state, "packed_full_only");
  assert.equal(validation.recommended_action, "none");
});


test("real Plan HTTP builders preserve retained SDK request objects and native validation", () => {
  const runtime = new NativeRuntime();
  const base = "http://127.0.0.1:1/base/";
  const items = [{ plan_item_ref: "native/wide", line_number: 9007199254740993n }];
  assert.equal(runtime.joinPath(base, "/child"), "http://127.0.0.1:1/base/child");
  assert.equal(runtime.normalizeRemoteText("  native\n"), "native");
  assert.equal(runtime.normalizeRemoteText("  "), null);
  assert.equal(runtime.buildQueryString({}), null);
  assert.equal(runtime.buildQueryString({ z: "中文", a: "space + slash/" }), "a=space+%2B+slash%2F&z=%E4%B8%AD%E6%96%87");
  assert.deepEqual(runtime.buildListPlansRequest(base, "fixture", "docs/a b.md"), {
    method: "GET", url: "http://127.0.0.1:1/base/v1/native/repositories/fixture/sprints?artifact_path=docs%2Fa+b.md", body: null,
  });
  assert.deepEqual(runtime.buildCreatePlanRequest(base, {
    repoName: "fixture", title: "Native", artifactPath: "docs/plan.md", artifactHeading: "Native", items,
  }), { method: "POST", url: "http://127.0.0.1:1/base/v1/native/repositories/fixture/sprints", body: {
    title: "Native", artifact_path: "docs/plan.md", artifact_selector: null, artifact_heading: "Native",
    items, status: "draft", source_kind: "manual_edit",
  } });
  assert.deepEqual(runtime.buildRevisePlanRequest(base, {
    planId: "PL-1", artifactPath: "docs/plan.md", artifactHeading: "Native", items, expectedHeadRevisionId: "PR-1",
  }), { method: "POST", url: "http://127.0.0.1:1/base/v1/native/sprints/PL-1/revisions", body: {
    artifact_path: "docs/plan.md", artifact_selector: null, artifact_heading: "Native", items,
    source_kind: "manual_edit", expected_head_revision_id: "PR-1",
  } });
  assert.deepEqual(runtime.buildUpdatePlanStatusRequest(base, "PL-1", "archived"), {
    method: "PATCH", url: "http://127.0.0.1:1/base/v1/native/sprints/PL-1", body: { status: "archived" },
  });
  assert.deepEqual(runtime.buildPutPlanRevisionArtifactsRequest(base, "PL-1", "PR-1", items), {
    method: "PUT", url: "http://127.0.0.1:1/base/v1/native/sprints/PL-1/revisions/PR-1/artifacts", body: { artifacts: items },
  });
  assert.throws(() => runtime.buildGetPlanRequest(base, ""), nativeFailure);
  assert.throws(() => runtime.buildQueryString({ numeric: 1 }), nativeFailure);
  assert.throws(() => runtime.buildCreatePlanRequest(base, {
    repoName: "fixture", title: "Native", artifactPath: "docs/plan.md", artifactHeading: "Native", items: {},
  }), nativeFailure);
});

async function planHttpFixture(context, routes = {}) {
  // A separate worker serves loopback requests while the synchronous Rust
  // client blocks this thread. The product runtime never uses this server.
  const worker = new Worker(`
    const { parentPort, workerData } = require("node:worker_threads");
    const http = require("node:http");
    const requests = [];
    const server = http.createServer((req, res) => {
      let body = "";
      req.setEncoding("utf8");
      req.on("data", (chunk) => { body += chunk; });
      req.on("end", () => {
        requests.push({ method: req.method, url: req.url, headers: req.headers, body });
        const fixture = workerData[req.method + " " + req.url.split("?")[0]];
        res.setHeader("content-type", "application/json");
        if (fixture) {
          res.statusCode = fixture.status || 200;
          for (const [name, value] of Object.entries(fixture.headers || {})) res.setHeader(name, value);
          res.end(fixture.bytes ? Buffer.from(fixture.bytes) : fixture.body);
        } else if (req.url.includes("missing")) {
          res.statusCode = 404;
          res.end('{"detail":"native fixture missing"}');
        } else if (req.method === "GET" && (req.url.includes("/events?") || ["/sprints", "/revisions", "/planning-sessions"].some((suffix) => req.url.split("?")[0].endsWith(suffix)))) {
          res.end('[{"plan_id":"PL-1","sequence":9007199254740993}]');
        } else {
          res.end('{"plan_id":"PL-1","planning_session_id":"PS-1","wide":9007199254740993}');
        }
      });
    });
    server.listen(0, "127.0.0.1", () => parentPort.postMessage(server.address().port));
    parentPort.on("message", () => parentPort.postMessage(requests));
  `, { eval: true, workerData: routes });
  context.after(() => worker.terminate());
  const [port] = await once(worker, "message");
  return {
    baseUrl: `http://127.0.0.1:${port}`,
    async requests() {
      const received = once(worker, "message");
      worker.postMessage("requests");
      const [requests] = await received;
      return requests;
    },
  };
}

test("real native Plan HTTP operations retain authority, exact wire integers and HTTP errors", { timeout: 15000 }, async (context) => {
  const runtime = new NativeRuntime();
  const server = await planHttpFixture(context);
  const options = { repositoryIndex: 7n, headersJson: '{"x-native-fixture":"retained"}', timeoutMs: 2000n };
  assert.equal(runtime.listPlans(server.baseUrl, "fixture", "docs/a b.md", options)[0].sequence, 9007199254740993n);
  assert.equal(runtime.getPlan(server.baseUrl, "PL-1", options).wide, 9007199254740993n);
  assert.equal(runtime.appendPlanningSessionEvent(server.baseUrl, "PS-1", "native_fixture", {
    wide: 9007199254740993n, negative: -9007199254740993n,
  }, options).wide, 9007199254740993n);
  assert.equal(runtime.listPlanningSessionEvents(server.baseUrl, "PS-1", {
    afterSequence: 9007199254740993n, limit: 17n,
  }, options)[0].sequence, 9007199254740993n);
  assert.throws(() => runtime.getPlan(server.baseUrl, "missing", options), (error) => {
    nativeFailure(error);
    assert.match(error.message, /404/);
    assert.match(error.message, /native fixture missing/);
    return true;
  });
  const requests = await server.requests();
  const listUrl = new URL(requests[0].url, server.baseUrl);
  assert.equal(listUrl.pathname, "/v1/native/repository-authorities/7/sprints");
  assert.equal(listUrl.searchParams.get("artifact_path"), "docs/a b.md");
  assert.equal(requests[0].headers["x-native-fixture"], "retained");
  const event = requests.find((request) => request.method === "POST" && request.url.endsWith("/events"));
  assert.match(event.body, /"wide":9007199254740993/);
  assert.match(event.body, /"negative":-9007199254740993/);
  const eventList = requests.find((request) => request.method === "GET" && request.url.includes("/events?"));
  assert.equal(new URL(eventList.url, server.baseUrl).searchParams.get("after_sequence"), "9007199254740993");
});

test("real standalone Plan HTTP APIs preserve native repository and Actor admission guards", () => {
  const runtime = new NativeRuntime();
  const base = "http://127.0.0.1:1";
  const create = { repoName: "fixture", title: "Native", artifactPath: "docs/plan.md", artifactHeading: "Native", items: [] };
  const revise = { planId: "PL-1", artifactPath: "docs/plan.md", artifactHeading: "Native", items: [] };
  const options = { repositoryIndex: 7, timeoutMs: 50 };
  assert.throws(() => runtime.listPlans(base, "fixture"), (error) => {
    nativeFailure(error); assert.match(error.message, /repository_index is required/); return true;
  });
  for (const invoke of [
    () => runtime.createPlan(base, create, options),
    () => runtime.revisePlan(base, revise, options),
    () => runtime.updatePlanStatus(base, "PL-1", "archived", options),
    () => runtime.putPlanRevisionArtifacts(base, "PL-1", "PR-1", [], options),
  ]) assert.throws(invoke, (error) => { nativeFailure(error); assert.match(error.message, /Actor registration/); return true; });
  for (const invalid of [
    { headersJson: "{" }, { headersJson: "[]" }, { headersJson: '{"numeric":1}' },
    { repositoryIndex: 4294967296n }, { timeoutMs: 18446744073709551616n },
    { retryAttempts: -1n }, { poolMaxIdlePerHost: 1.5 }, { timeoutMs: null },
  ]) assert.throws(() => runtime.listPlans(base, "fixture", undefined, { ...options, ...invalid }), Error);
  assert.throws(() => runtime.listPlanningSessionEvents(base, "PS-1", { afterSequence: 9223372036854775808n }), nativeFailure);
});


test("real HTTP manager classes preserve native timeout validation and lifecycle statistics", { timeout: 30000 }, async (context) => {
  const runtime = new NativeRuntime();
  const addon = runtime.loadAddon();
  assert.equal(addon.DEFAULT_REQUEST_TIMEOUT_SECONDS, DEFAULT_REQUEST_TIMEOUT_SECONDS);
  assert.equal(addon.LONG_RUNNING_REQUEST_TIMEOUT_SECONDS, LONG_RUNNING_REQUEST_TIMEOUT_SECONDS);
  const defaults = new PlanRemoteTransportConfig(undefined, undefined, runtime);
  assert.equal(defaults.requestTimeoutSeconds(false), 30);
  assert.equal(defaults.requestTimeoutSeconds(true), 1800);
  const selected = new PlanRemoteTransportConfig(1.25, 95.5, runtime);
  assert.equal(selected.requestTimeoutSeconds(false), 1.25);
  assert.equal(selected.requestTimeoutSeconds(true), 95.5);
  for (const value of [0, -1, Infinity, NaN]) {
    assert.throws(() => new PlanRemoteTransportConfig(value, undefined, runtime), nativeFailure);
    assert.throws(() => new PlanRemoteTransportConfig(undefined, value, runtime), nativeFailure);
  }
  const server = await planHttpFixture(context);
  const manager = new PlanHttpClientManager(server.baseUrl, { repositoryIndex: 7, timeoutMs: 2000 }, runtime);
  context.after(() => manager.close());
  const initial = manager.inspect();
  assert.equal(initial.request_count, 0);
  assert.equal(initial.default_timeout_ms, 2000);
  assert.equal(initial.closed, false);
  const create = { repoName: "fixture", title: "Native", artifactPath: "docs/plan.md", artifactHeading: "Native", items: [] };
  const revise = { planId: "PL-1", artifactPath: "docs/plan.md", artifactHeading: "Native", items: [] };
  for (const invoke of [
    () => manager.createPlan(create), () => manager.revisePlan(revise),
    () => manager.updatePlanStatus("PL-1", "archived"), () => manager.putPlanRevisionArtifacts("PL-1", "PR-1", []),
  ]) assert.throws(invoke, (error) => { nativeFailure(error); assert.match(error.message, /Actor registration/); return true; });
  assert.equal(manager.inspect().request_count, 0);
  assert.equal(manager.listPlans("fixture")[0].plan_id, "PL-1");
  assert.equal(manager.getPlan("PL-1").plan_id, "PL-1");
  assert.equal(manager.listPlanRevisions("PL-1").length, 1);
  assert.equal(manager.getPlanRevision("PL-1", "PR-1").plan_id, "PL-1");
  assert.equal(manager.createPlanningSession("PL-1").planning_session_id, "PS-1");
  assert.equal(manager.listPlanningSessions("PL-1").length, 1);
  assert.equal(manager.getPlanningSession("PS-1").planning_session_id, "PS-1");
  assert.equal(manager.appendPlanningSessionEvent("PS-1", "fixture", { wide: 9007199254740993n }).wide, 9007199254740993n);
  assert.equal(manager.listPlanningSessionEvents("PS-1")[0].sequence, 9007199254740993n);
  assert.equal(manager.joinPlanningSession("PS-1").planning_session_id, "PS-1");
  assert.equal(manager.promotePlanningSession("PS-1", {
    artifactPath: "docs/plan.md", artifactSelector: "native/root", artifactHeading: "Native", items: [],
  }).planning_session_id, "PS-1");
  assert.equal(manager.closePlanningSession("PS-1").planning_session_id, "PS-1");
  assert.equal(manager.inspect().request_count, 12);
  assert.equal(manager.inspect().retry_count, 0);
  const closed = manager.close();
  assert.equal(closed.closed, true);
  assert.deepEqual(manager.close(), closed);
  assert.throws(() => manager.getPlan("PL-1"), (error) => { nativeFailure(error); assert.match(error.message, /closed/); return true; });
  const workflow = new TaskWorkflowHttpClientManager(server.baseUrl, undefined, runtime);
  assert.equal(workflow.inspect().default_timeout_ms, 30000);
  assert.equal(workflow.inspect().request_count, 0);
  assert.equal(workflow.close().closed, true);
  assert.equal(workflow.inspect().closed, true);
  assert.equal(workflow.close().closed, true);
});


test("real task HTTP adapter retains lineage, Task/Line reads, binary downloads and Actor guards", { timeout: 15000 }, async (context) => {
  const runtime = new NativeRuntime();
  const prefix = "/v1/native/repository-authorities/7";
  const task = { task_id: "LNT-0007", title: "Native adapter fixture", status: "active", repository_index: 7, successful_land: null };
  const line = { line_name: "main", head_snapshot_id: "SNP-000000000001" };
  const bytes = [0, 255, 128, 10, 13];
  const server = await planHttpFixture(context, {
    [`GET ${prefix}/lines/main`]: { body: runtime.encodeJson(line) },
    [`GET ${prefix}/tasks/LNT-0007`]: { body: runtime.encodeJson(task) },
    [`GET ${prefix}/tasks`]: { body: runtime.encodeJson([task]) },
    [`POST ${prefix}/remote-sync/zstd-bulk/plan`]: { status: 400, body: '{"detail":"native fixture invalid bulk plan"}' },
    [`GET ${prefix}/remote-sync/zstd-bulk/object-packs/PCK-NODE`]: { bytes, headers: { "content-type": "application/octet-stream" } },
    [`GET ${prefix}/remote-sync/zstd-bulk/tree-packs/TPK-NODE`]: { bytes, headers: { "content-type": "application/octet-stream" } },
  });
  const remote = new HttpTaskRemote(server.baseUrl, { repositoryIndex: 7, timeoutMs: 2000 }, runtime);
  context.after(() => remote.closeClient());
  assert.deepEqual(remote.changeLineagePayload("main", line), { forked_from_line: "main", fork_snapshot_id: line.head_snapshot_id });
  assert.throws(() => remote.changeLineagePayload(" "), nativeFailure);
  assert.throws(() => remote.changeLineagePayload("main", []), nativeFailure);
  assert.deepEqual(remote.getLine("fixture", "main"), line);
  assert.deepEqual(remote.getTask("LNT-0007", "fixture"), task);
  assert.deepEqual(remote.listTasks("fixture"), [task]);
  assert.deepEqual(remote.getRemoteZstdObjectPack("fixture", "PCK-NODE"), Buffer.from(bytes));
  assert.deepEqual(remote.getRemoteZstdTreePack("fixture", "TPK-NODE"), Buffer.from(bytes));
  assert.throws(() => remote.planRemoteZstdBulk("fixture", null), (error) => {
    nativeFailure(error);
    assert.match(error.message, /400/);
    assert.match(error.message, /native fixture invalid bulk plan/);
    return true;
  });
  assert.throws(() => remote.commitRemoteZstdBulk("fixture", null), nativeFailure);
  for (const invoke of [
    () => remote.putRemoteZstdObjectPack("fixture", "PCK-NODE", Buffer.from(bytes)),
    () => remote.putRemoteZstdTreePack("fixture", "TPK-NODE", Buffer.from(bytes)),
    () => remote.commitRemoteZstdBulk("fixture", {}),
  ]) assert.throws(invoke, (error) => { nativeFailure(error); assert.match(error.message, /Actor registration/); return true; });
  assert.equal((await server.requests()).length, 6);
  const closed = remote.closeClient();
  assert.equal(closed.closed, true);
  assert.deepEqual(remote.closeClient(), closed);
  assert.throws(() => remote.getTask("LNT-0007", "fixture"), (error) => { nativeFailure(error); assert.match(error.message, /closed/); return true; });
});

test("real closeout HTTP adapter preserves receipt filtering, compact statement errors and lifecycle", () => {
  const runtime = new NativeRuntime();
  const remote = new HttpWorkflowCloseoutRemote("http://127.0.0.1:1", { repositoryIndex: 7, timeoutMs: 50 }, runtime);
  const receipt = remote.mutationReceipt({ action: "submit_land", sourceAction: "submit_land", delivery: "direct_response",
    responseRecovery: { state: "recovered", removed: null },
    result: { submission_id: "SUB-FIXTURE", status: "queued", task_id: "LNT-0007", opaque: "filtered out" } });
  assert.deepEqual(receipt, { action: "submit_land", source_action: "submit_land", delivery: "direct_response",
    submission_id: "SUB-FIXTURE", status: "queued", task_id: "LNT-0007", response_recovery: { state: "recovered" } });
  assert.ok(Array.isArray(remote.actionMutationReceipts("record_review", { review_id: "RV-FIXTURE" })));
  assert.throws(() => remote.actionMutationReceipts("record_review", []), nativeFailure);
  assert.throws(() => remote.mutationReceipt({ action: " ", sourceAction: "fixture", delivery: "fixture" }), nativeFailure);
  assert.throws(() => remote.putAttestation("LNT-0007/P-01", "ai_only_experimental", { extra: true }), (error) => {
    nativeFailure(error); assert.match(error.message, /Unsupported compact Attestation statement field/); return true;
  });
  assert.throws(() => remote.readPatchsetCiStatus("LNT-0007/P-01", { recentLimit: 9223372036854775808n }), nativeFailure);
  assert.equal(remote.inspectClient().request_count, 0);
  const closed = remote.closeClient();
  assert.equal(closed.closed, true);
  assert.deepEqual(remote.closeClient(), closed);
  assert.throws(() => remote.getPolicy("LNT-0007/P-01", { exactId: true }), (error) => {
    nativeFailure(error); assert.match(error.message, /closed/); return true;
  });
});

test("real runtime and manifest normalizers retain schemas, issues and retirement boundaries", () => {
  const runtime = new NativeRuntime();
  assert.deepEqual(runtime.languageBindingInfo(), runtime.bindingInfo());
  const state = runtime.runtimeBindingStateDefaultPayload();
  assert.equal(typeof state.surface_bindings, "object");
  const normalized = runtime.runtimeBindingStateNormalizeDocument({ version: 2, chats: { legacy: { session_id: "retired" } }, surface_bindings: {} });
  assert.equal(normalized.ir_version, runtime.runtimeBindingStateIrVersion());
  assert.equal(normalized.state.chats, undefined);
  assert.equal(typeof runtime.runtimeBindingStateSchema(), "object");
  assert.deepEqual(runtime.workerManifestDefaultConfig(), { version: 1, workers: {} });
  assert.equal(runtime.workerManifestSchema().properties.ir_version.const, runtime.workerManifestIrVersion());
  const malformed = runtime.workerManifestNormalizeDocument({ version: "bad", workers: {
    "telegram/main": { kind: 123, token: 99 },
  } }, "/fixture/manifest.json");
  assert.equal(malformed.ir_version, runtime.workerManifestIrVersion());
  assert.equal(malformed.config.workers["telegram/main"].token, null);
  assert.ok(malformed.issues.length > 0);
  assert.equal(runtime.workerManifestSelectTelegramWorker({ workers: {
    "telegram/main": { name: "main" }, "telegram/side": { name: "side" },
  } }, "side").name, "side");
  assert.equal(runtime.workerManifestSelectTelegramWorker({ workers: {} }, "missing"), null);
  assert.deepEqual(runtime.projectRepoRetireRuntimeBlockers({
    active_agent_runtime_rows: [{ runtime_kind: "agent_run", status: "active" }, { runtime_kind: "agent_run", status: "active", count: 2 }],
    active_planning_runtime_rows: [{ status: "active" }],
  }), { active_agent_runtime_groups: [{ runtime_kind: "agent_run", status: "active", count: 3 }],
    active_planning_runtime_groups: [{ status: "active", count: 1 }] });
  assert.throws(() => runtime.projectRepoRetireRuntimeBlockers([]), nativeFailure);
  const addon = runtime.loadAddon();
  for (const name of [
    "aitAgentWebRuntimeExecute", "aitAgentEnvFileLoad", "aitAgentTelegramWorkflowNotificationFormat",
    "aitAgentTelegramTurnInputPlan", "aitAgentTelegramMessageDeliveryExecute", "aitAgentTelegramWorkflowQueryPlan",
  ]) assert.equal(name in addon, false);
});

test("real transport data helpers preserve native identity, signed message IDs and compaction", () => {
  const runtime = new NativeRuntime();
  assert.equal(runtime.transportEnvelopeIrVersion(), "ait.transport_envelope.v2");
  const schema = runtime.encodeJson(runtime.transportEnvelopeSchema());
  assert.match(schema, /conversation_key/);
  assert.doesNotMatch(schema, /session_metadata|shared_session/);
  const binding = runtime.transportEnvelopeBuildBindingMetadata({
    transport: "discord", surfaceId: "C123", conversationKey: "discord:C123:TH999", threadId: "TH999",
    replyTarget: { channel_id: "C123", thread_id: "TH999" }, metadataExtra: { tenant: "fixture", transport: "cannot-override" },
  });
  assert.equal(binding.transport, "discord");
  assert.equal(binding.conversation_key, "discord:C123:TH999");
  assert.equal(binding.tenant, "fixture");
  const ids = [9007199254740993n, 9223372036854775807n];
  const event = runtime.transportEnvelopeBuildEventEnvelope({
    transport: "telegram", actorIdentity: "telegram:fixture", channelId: "123", text: "Metadata fixture",
    messageId: ids[0], messageIds: [...ids, ids[0], 0, 9223372036854775808n],
    attachments: [{ media_kind: "music", file_id: "fixture-file", duration: 42 }],
  });
  assert.deepEqual(event.message.message_ids, ids);
  assert.equal(event.message.attachments[0].kind, "music");
  const compactEvent = runtime.transportEnvelopeCompactEventEnvelope(event);
  assert.deepEqual(compactEvent.message.message_ids, ids);
  assert.equal(compactEvent.channel.channel_id, "123");
  const reply = runtime.transportEnvelopeBuildReplyEnvelope({
    transport: "slack", channelId: "C123", text: "Metadata fixture", threadId: "T456",
    replyToMessageId: ids[0], replyToMessageIds: [...ids, ids[0]], metadata: { marker: "fixture" },
  });
  assert.equal(reply.delivery_kind, "chat_reply");
  assert.deepEqual(reply.reply_to.message_ids, ids);
  const compactReply = runtime.transportEnvelopeCompactReplyEnvelope(reply);
  assert.equal(compactReply.target.thread_id, "T456");
  assert.deepEqual(compactReply.reply_to.message_ids, ids);
});

test("real usage helpers extract and aggregate temporary JSONL with native scope and file errors", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-usage-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const runtime = new NativeRuntime();
  const coordinator = path.join(root, "coordinator.jsonl");
  const worker = path.join(root, "worker.jsonl");
  const malformed = path.join(root, "malformed.jsonl");
  await writeFile(coordinator, '{"payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":3,"output_tokens":2},"total_token_usage":{"input_tokens":8,"output_tokens":4,"cached_input_tokens":1,"reasoning_output_tokens":2}}}}\n');
  await writeFile(worker, '{"type":"turn.completed","usage":{"input_tokens":11,"output_tokens":4,"total_tokens":15}}\n');
  await writeFile(malformed, "{\n");
  assert.equal(runtime.extractCodexUsageJsonl(coordinator, "total").manifest_usage.total_tokens, 12);
  assert.equal(runtime.extractCodexUsageJsonl(coordinator, "last").manifest_usage.total_tokens, 5);
  const bundle = runtime.extractCodexUsageBundleJsonl([coordinator, worker], { usageRoles: ["coordinator", "worker"] });
  assert.equal(bundle.usage_file_count, 2);
  assert.equal(bundle.manifest_usage.total_tokens, 27);
  assert.equal(bundle.role_breakdown.coordinator.usage.total_tokens, 12);
  assert.equal(bundle.role_breakdown.worker.usage.total_tokens, 15);
  for (const invoke of [
    () => runtime.extractCodexUsageJsonl(malformed, "total"),
    () => runtime.extractCodexUsageJsonl(path.join(root, "missing.jsonl"), "total"),
    () => runtime.extractCodexUsageJsonl(coordinator, "unknown-scope"),
    () => runtime.extractCodexUsageBundleJsonl([coordinator, worker], { usageRoles: ["worker"] }),
    () => runtime.extractCodexUsageBundleJsonl([]),
  ]) assert.throws(invoke, nativeFailure);
});


test("real lower-level Task identity facts retain native Python fixtures and checked values", () => {
  const runtime = new NativeRuntime();
  assert.deepEqual(runtime.taskWorkflowTimestampFacts("2026-10-26T00:00:00Z"), {
    epoch_ms: 1792972800000, source: "injected", timestamp: "2026-10-26T00:00:00Z",
  });
  assert.deepEqual(runtime.taskWorkflowSequenceIdentityFacts("T", 7n, { namespacePrefix: "L", width: 4n }), {
    family: "T", generated_id: "LT-0007", namespace_prefix: "L", number: 7, width: 4,
  });
  assert.deepEqual(runtime.taskWorkflowWorkflowIdFacts("T", {
    namespacePrefix: "L", timestampMs: 42n, randomnessHex: "0123456789abcdef0123",
  }), { family: "T", generated_id: "LT-000000001A04HMASW9NF6YY093", namespace_prefix: "L",
    randomness_hex: "0123456789abcdef0123", timestamp_ms: 42 });
  const overrides = { plan_core_backend: "rust" };
  assert.deepEqual(runtime.taskWorkflowRuntimeSelectionFacts(overrides),
    runtime.planConfigRuntimeBuildSelectionFacts(runtime.encodeJson({ overrides })));
  for (const invoke of [
    () => runtime.taskWorkflowTimestampFacts("invalid"),
    () => runtime.taskWorkflowSequenceIdentityFacts("T", 9223372036854775808n),
    () => runtime.taskWorkflowSequenceIdentityFacts("T", 1n, { width: 18446744073709551616n }),
    () => runtime.taskWorkflowWorkflowIdFacts("T", { timestampMs: 42n }),
    () => runtime.taskWorkflowWorkflowIdFacts("T", { timestampMs: 9223372036854775808n }),
    () => runtime.taskWorkflowRuntimeSelectionFacts([]),
  ]) assert.throws(invoke, Error);
  assert.deepEqual(runtime.taskRemoteChangeLineagePayload("main", { head_snapshot_id: "SNP-BASE" }), {
    forked_from_line: "main", fork_snapshot_id: "SNP-BASE",
  });
});

async function makeFixtureDirectoriesWritable(root) {
  await chmod(root, 0o700);
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink()) {
      await makeFixtureDirectoriesWritable(path.join(root, entry.name));
    }
  }
}

test("real lower-level Task helpers retain native files, layout, status metadata and bound apply", async (context) => {
  const fixture = await realpath(await mkdtemp(path.join(os.tmpdir(), "ait-node-task-sdk-")));
  context.after(async () => {
    await makeFixtureDirectoriesWritable(fixture);
    await rm(fixture, { recursive: true, force: true });
  });
  const root = path.join(fixture, "repository");
  await mkdir(root);
  const runtime = new NativeRuntime();
  runtime.taskWorkflowInit(root);
  runtime.configureRepository(root, { agent_teams: "disabled", sprint: "off", user_name: "Node binding fixture" });
  // Keep this isolated repository's physical worktree/cache and alias roots
  // inside its fixture. This changes no production Task or repository state.
  const configPath = path.join(root, ".ait", "config.json");
  const config = runtime.repoStateReadRepoConfigJson(configPath);
  config.task_worktree = { ephemeral_root: path.join(fixture, "worktrees"), alias_root: path.join(fixture, "aliases") };
  runtime.repoStateWriteRepoConfigJson(configPath, config);
  const textPath = path.join(root, "text.txt");
  const bytesPath = path.join(root, "bytes.bin");
  const jsonPath = path.join(root, "value.json");
  await writeFile(textPath, "text\r\n");
  await writeFile(bytesPath, Buffer.from([0, 255, 128]));
  await writeFile(jsonPath, '{"n":18446744073709551615}');
  assert.equal(runtime.taskWorkflowReadUtf8TextFile(textPath), "text\n");
  assert.deepEqual(runtime.taskWorkflowReadBinaryFile(bytesPath), Buffer.from([0, 255, 128]));
  assert.equal(runtime.taskWorkflowReadJsonFile(jsonPath).n, 18446744073709551615n);
  assert.equal(runtime.taskWorkflowResolveRepoArtifactPath(root, "text.txt").artifact_path, "text.txt");
  assert.throws(() => runtime.taskWorkflowResolveRepoArtifactPath(root, "../outside"), nativeFailure);
  assert.throws(() => runtime.taskWorkflowReadUtf8TextFile(bytesPath), (error) => {
    nativeFailure(error); assert.match(error.message, /Io\(|Invalid\(/); return true;
  });
  assert.throws(() => runtime.planFilesystemReadUtf8TextFile(bytesPath), (error) => {
    nativeFailure(error); assert.match(error.message, /not valid UTF-8/); return true;
  });
  assert.equal(runtime.taskWorkflowEnsureMainSeedMirror(root, { lineName: "other" }).reason, "target_not_default_line");
  const seed = runtime.taskWorkflowResolveMainSeedMirrorLocation(root, "fixture-seed");
  assert.ok(seed === null || seed.target_path.startsWith(fixture + path.sep));
  runtime.configureRepository(root, { sprint: "on" });
  const task = runtime.taskQuick(root, { intent: "Exercise retained raw Task SDK", local: true, editRoot: path.join(fixture, "work") });
  const location = runtime.taskWorkflowResolveTaskWorktreeLocation(root, task.worktree.name);
  assert.ok(location.target_path.startsWith(fixture + path.sep));
  await writeFile(path.join(task.worktree.path, "sdk.txt"), "Retained raw SDK content\n");
  const snapshot = runtime.snapshotCapture(task.worktree.path, task.task_id, { message: "Retained raw SDK fixture" });
  assert.equal(runtime.taskWorkflowEnsureStatusManifest(root, snapshot.snapshot_id).source, "snapshot_binary_metadata");
  assert.equal(runtime.taskWorkflowTaskAudit(root, task.task_id, { local: true }).task.task_id, task.task_id);
  assert.throws(() => runtime.taskWorkflowTaskLand(root, task.task_id, { local: true }), (error) => {
    nativeFailure(error); assert.match(error.message, /task-bound worktree/); return true;
  });
  const applied = runtime.taskWorkflowTaskLand(task.worktree.path, task.task_id, { local: true });
  assert.equal(applied.task_id, task.task_id);
  assert.equal(await readFile(path.join(root, "sdk.txt"), "utf8"), "Retained raw SDK content\n");
  assert.equal(runtime.taskWorkflowTaskShow(root, task.task_id, { local: true }).status, "completed");
  for (const invoke of [
    () => runtime.taskWorkflowTaskAbandon(root, "LNT-9999", { local: true }),
    () => runtime.taskWorkflowQueueSummary(root, "missing-remote"),
    () => runtime.taskWorkflowLandSubmit(root, "LNT-9999", { targetLine: "main", mode: "direct", remoteName: "missing-remote" }),
    () => runtime.taskWorkflowLandShow(root, "missing-receipt", { remoteName: "missing-remote" }),
    () => runtime.taskWorkflowLandRetry(root, "missing-receipt", { remoteName: "missing-remote" }),
  ]) assert.throws(invoke, nativeFailure);
});


test("real workflow data SDK preserves retained producer Python fixtures and native summaries", () => {
  const runtime = new NativeRuntime();
  // Retained data fixtures from the producer's direct API contract cases.
  const cases = [
  {
    "name": "build_linked_task_lookup_payload",
    "args": [
      null,
      null
    ],
    "kwargs": {},
    "result": {
      "linked_task_count": 0,
      "task_links_by_item": [],
      "tasks_by_plan": []
    }
  },
  {
    "name": "build_linked_change_lookup_payload",
    "args": [
      null
    ],
    "kwargs": {},
    "result": {
      "change_links_by_task": [],
      "linked_change_count": 0
    }
  },
  {
    "name": "build_task_tracking_title_payload",
    "args": [
      {
        "task_id": "LT-0001",
        "title": "SDK fixture",
        "intent": "Exercise retained data helpers",
        "status": "active"
      }
    ],
    "kwargs": {},
    "result": {
      "title": "LT-0001: SDK fixture"
    }
  },
  {
    "name": "build_task_tracking_metadata_payload",
    "args": [
      {
        "task_id": "LT-0001",
        "title": "SDK fixture",
        "intent": "Exercise retained data helpers",
        "status": "active"
      },
      "ai_only_experimental",
      "required"
    ],
    "kwargs": {},
    "result": {
      "author_mode": "ai_only_experimental",
      "objective": "Exercise retained data helpers",
      "task_id": "LT-0001",
      "tracking_policy": "required"
    }
  },
  {
    "name": "build_task_audit_verdict_payload",
    "args": [
      {
        "task_id": "LT-0001",
        "title": "SDK fixture",
        "intent": "Exercise retained data helpers",
        "status": "active"
      },
      [],
      "main"
    ],
    "kwargs": {},
    "result": {
      "recommended_action": {
        "change_id": null,
        "code": "create_change",
        "detail": "This task has no linked local changes yet.",
        "label": "Create a change"
      },
      "summary": {
        "ambiguous_line_change_count": 0,
        "change_count": 0,
        "effective_on_target_change_count": 0,
        "effectively_complete_on_target": false,
        "landed_change_count": 0,
        "line_evidence_change_count": 0,
        "missing_remote_change_count": 0,
        "open_change_count": 0,
        "open_on_target_change_count": 0,
        "patchset_count": 0,
        "ready_to_complete": false,
        "stale_workflow_change_count": 0,
        "stale_workflow_records": false,
        "verdict": "no_changes"
      },
      "workflow": {
        "reason": "No linked local changes exist yet.",
        "state": "planning"
      }
    }
  },
  {
    "name": "workflow_landed_facts",
    "args": [
      {}
    ],
    "kwargs": {},
    "result": {
      "change": {},
      "current_line_name": "unknown",
      "ignore_workspace_authoring": false,
      "landed": false,
      "landing_summary": null,
      "patchset": null,
      "patchset_label": "",
      "patchset_revision_snapshot_id": null,
      "patchset_source": null,
      "revision_snapshot_id": null,
      "target_line": "main",
      "task": {},
      "workspace": {}
    }
  },
  {
    "name": "workflow_land_full_facts",
    "args": [
      {}
    ],
    "kwargs": {},
    "result": {
      "attestation": null,
      "base_is_fresh": false,
      "base_line_name": "main",
      "change": {},
      "code_review_summary_count": 0,
      "current_line_name": "unknown",
      "ignore_workspace_authoring": false,
      "landing_blocker_class": "",
      "landing_result": {},
      "landing_status": "",
      "landing_submission_id": null,
      "landing_summary": null,
      "patchset": null,
      "patchset_base_snapshot_id": null,
      "patchset_ci_status": null,
      "patchset_is_authoritative": false,
      "patchset_refresh": null,
      "patchset_revision_snapshot_id": null,
      "patchset_source": null,
      "policy": null,
      "policy_decision": "pending",
      "remote_base_snapshot_id": null,
      "requires_code_review_summary": false,
      "resolved_change_id": "",
      "review_approvals": 0,
      "review_blocking": 0,
      "review_summary": {},
      "revision_snapshot_id": null,
      "stale_policy_blocker_cleared": false,
      "target_line": "main",
      "task": {},
      "task_review_approvals": 0,
      "team_review_approvals": 0,
      "tests_state": "",
      "workspace": {},
      "workspace_matches_patchset": null,
      "worktree_retarget": null
    }
  },
  {
    "name": "workflow_ready_facts",
    "args": [
      {}
    ],
    "kwargs": {},
    "result": {
      "attestation": null,
      "base_line": {},
      "base_line_name": "main",
      "change": {},
      "code_review_summary_count": 0,
      "external_readiness": null,
      "freshness": {},
      "patchset": null,
      "patchset_ci_status": null,
      "patchset_refresh": null,
      "payload_seed": {},
      "policy": null,
      "requires_code_review_summary": false,
      "review_blocking": 0,
      "task": {},
      "task_review_approvals": 0,
      "task_review_enabled": false,
      "tests_state": "",
      "workspace": {},
      "worktree_retarget": null
    }
  },
  {
    "name": "workflow_land_phase_facts",
    "args": [
      {},
      {}
    ],
    "kwargs": {},
    "result": {
      "change": {},
      "change_id": "",
      "full_steps": {},
      "payload_seed": {},
      "ready_command": "ait workflow ready --apply",
      "ready_done": false,
      "ready_next_action": {},
      "state_next_action_command": "",
      "task": {}
    }
  },
  {
    "name": "project_workflow_land_phase_read_model",
    "args": [
      {},
      {}
    ],
    "kwargs": {},
    "result": {
      "next_action": {
        "code": "workflow_ready",
        "command": "ait workflow ready --apply",
        "detail": "The change still needs a ready patchset and attestation before finish can continue.",
        "summary": "Run workflow ready before review or finish."
      },
      "steps": [
        {
          "code": "ready",
          "command": "ait workflow ready --apply",
          "detail": "Run workflow ready before review or finish.",
          "label": "Ready",
          "status": "pending"
        },
        {
          "code": "review",
          "detail": "Review starts after `workflow ready` prepares a selected patchset and attestation.",
          "label": "Review",
          "status": "waiting"
        },
        {
          "code": "policy",
          "detail": "Finish preflight will evaluate policy after review clears on a ready patchset.",
          "label": "Policy",
          "status": "waiting"
        },
        {
          "code": "land",
          "detail": "Finish starts after `workflow ready` and review are complete.",
          "label": "Finish",
          "status": "waiting"
        },
        {
          "code": "task",
          "detail": "Task completion follows a successful finish.",
          "label": "Task",
          "status": "waiting"
        }
      ],
      "suggested_commands": [
        "ait workflow ready --apply"
      ],
      "workflow_phase": "land"
    }
  },
  {
    "name": "workflow_remote_mutation_receipt",
    "args": [
      "record_review",
      "workflow ready",
      "direct_response",
      null,
      {
        "review_id": "RV-1"
      }
    ],
    "kwargs": {},
    "result": {
      "action": "record_review",
      "delivery": "direct_response",
      "review_id": "RV-1",
      "source_action": "workflow ready"
    }
  },
  {
    "name": "workflow_remote_action_mutation_receipts",
    "args": [
      "record_review",
      {
        "review_id": "RV-1",
        "policy_refresh": {
          "decision": "pass"
        }
      }
    ],
    "kwargs": {},
    "result": [
      {
        "action": "record_review",
        "delivery": "direct_response",
        "review_id": "RV-1",
        "source_action": "record_review"
      },
      {
        "action": "evaluate_policy",
        "decision": "pass",
        "delivery": "direct_response",
        "source_action": "record_review"
      }
    ]
  },
  {
    "name": "workflow_apply_phase_payload",
    "args": [
      "publish",
      "published"
    ],
    "kwargs": {
      "detail": "Fixture detail"
    },
    "result": {
      "code": "published",
      "detail": "Fixture detail",
      "phase": "publish"
    }
  },
  {
    "name": "workflow_apply_phase_summary",
    "args": [
      {
        "phase": "publish",
        "code": "published",
        "detail": "Fixture detail"
      }
    ],
    "kwargs": {},
    "result": "Fixture detail"
  },
  {
    "name": "workflow_mutation_receipt_summary",
    "args": [
      {
        "action": "record_review",
        "source_action": "workflow ready",
        "delivery": "direct_response",
        "review_id": "RV-1"
      }
    ],
    "kwargs": {},
    "result": "`record_review` accepted via direct response (review_id=RV-1)"
  },
  {
    "name": "workflow_applied_action_summary",
    "args": [
      {
        "code": "record_review",
        "action": "record_review",
        "detail": "Fixture detail"
      }
    ],
    "kwargs": {},
    "result": "recorded review `unknown`"
  }
];
  for (const fixture of cases) {
    const method = fixture.name.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());
    let args = fixture.args;
    if (fixture.name === "workflow_apply_phase_payload") args = [...args, fixture.kwargs];
    if (fixture.name === "workflow_remote_mutation_receipt") {
      const [action, sourceAction, delivery, responseRecovery, result] = args;
      args = [{ action, sourceAction, delivery, responseRecovery: responseRecovery ?? undefined, result }];
    }
    assert.deepEqual(runtime[method](...args), fixture.result, fixture.name);
  }
  assert.equal(runtime.workflowApplyPhaseSummary(null), null);
  assert.equal(runtime.workflowApplyPhaseSummary({}), " `unknown`");
  for (const invoke of [
    () => runtime.workflowReadyFacts([]),
    () => runtime.buildLinkedTaskLookupPayload({}, null),
    () => runtime.workflowApplyPhasePayload("", "ready"),
    () => runtime.workflowRemoteActionMutationReceipts("record_review", []),
  ]) assert.throws(invoke, nativeFailure);
});

test("real direct workflow SDK operations preserve native invalid Task and remote failures", async (context) => {
  const { root, runtime } = await repository(context);
  for (const invoke of [
    () => runtime.workflowReadyPayloadDirect(root, "LNT-9999", "missing-remote"),
    () => runtime.workflowLandPayloadDirect(root, "LNT-9999", "missing-remote"),
    () => runtime.workflowReadyApplyDirect(root, "LNT-9999", { remoteName: "missing-remote" }),
    () => runtime.workflowLandApplyDirect(root, "LNT-9999", { remoteName: "missing-remote" }),
  ]) assert.throws(invoke, nativeFailure);
});


test("real contextual workflow projections preserve six retained Python payloads", () => {
  const runtime = new NativeRuntime();
  const cases = [
  {
    "name": "project_workflow_landed_read_model",
    "context": {},
    "args": [
      {
        "task": {
          "task_id": "RT-1",
          "status": "active"
        },
        "change": {},
        "patchset": {
          "patchset_id": "RT-1/P-01",
          "revision_snapshot_id": "SNP-REV"
        },
        "target_line": "main",
        "base_line_name": "main"
      }
    ],
    "result": {
      "attestation": null,
      "base_line": {
        "head_snapshot_id": null,
        "line_name": "main"
      },
      "change": null,
      "freshness": {
        "base_is_fresh": true,
        "patchset_base_snapshot_id": null,
        "patchset_revision_snapshot_id": "SNP-REV",
        "preflight_state": "not_required",
        "rebase_state": "idle",
        "recovery_required": false,
        "remote_base_snapshot_id": null,
        "worktree_needs_retarget": false
      },
      "landing_summary": null,
      "next_action": {
        "code": "workflow_ready",
        "command": "ait workflow ready RT-1 --apply",
        "detail": "The workspace still has unsaved changes.",
        "summary": "Run workflow ready before review or finish."
      },
      "patchset": {
        "patchset_id": "RT-1/P-01",
        "revision_snapshot_id": "SNP-REV"
      },
      "patchset_source": null,
      "policy": {
        "checks": [],
        "decision": "pass",
        "status_source": "landed_fast_path"
      },
      "review": {
        "approvals": 1,
        "blocking": 0,
        "current_patchset_id": "RT-1/P-01",
        "reviews": [],
        "task_approvals": 1,
        "team_approvals": 0
      },
      "steps": [
        {
          "code": "snapshot",
          "detail": "No additional authoring snapshot is required because the change is already applied.",
          "label": "Snapshot",
          "status": "done"
        },
        {
          "code": "patchset",
          "detail": "Patchset `RT-1/P-01` is already part of the accepted history for this change.",
          "label": "Patchset",
          "status": "done"
        },
        {
          "code": "attestation",
          "detail": "Finish already succeeded, so attestation gating has already cleared.",
          "label": "Attestation",
          "status": "done"
        },
        {
          "code": "review",
          "detail": "Finish already succeeded, so review requirements were already satisfied.",
          "label": "Review",
          "status": "done"
        },
        {
          "code": "policy",
          "detail": "Finish already succeeded, so policy has already cleared.",
          "label": "Policy",
          "status": "done"
        },
        {
          "code": "land",
          "detail": "Task `RT-1` already has accepted work applied to `main`.",
          "label": "Finish",
          "status": "done"
        }
      ],
      "suggested_commands": [
        "ait workflow ready RT-1 --apply"
      ],
      "task": {
        "status": "active",
        "task_id": "RT-1"
      },
      "workspace": {
        "changed_count": 0,
        "clean": false,
        "current_line": "unknown",
        "head_snapshot_id": null,
        "workspace_matches_patchset": true,
        "workspace_status": "dirty"
      }
    }
  },
  {
    "name": "project_workflow_land_full_read_model",
    "context": {},
    "args": [
      {
        "task": {
          "task_id": "RT-1",
          "status": "active"
        },
        "change": {},
        "patchset": {
          "patchset_id": "RT-1/P-01",
          "revision_snapshot_id": "SNP-REV"
        },
        "target_line": "main",
        "base_line_name": "main"
      },
      false
    ],
    "result": {
      "attestation": null,
      "base_line": {
        "head_snapshot_id": null,
        "line_name": "main"
      },
      "change": null,
      "freshness": {
        "base_is_fresh": false,
        "patchset_base_snapshot_id": null,
        "patchset_revision_snapshot_id": null,
        "preflight_state": "stale",
        "rebase_state": "idle",
        "recovery_required": true,
        "remote_base_snapshot_id": null,
        "worktree_needs_retarget": false
      },
      "landing_summary": null,
      "next_action": {
        "code": "workflow_ready",
        "command": "ait workflow ready RT-1 --apply",
        "detail": "The workspace still has unsaved changes.",
        "summary": "Run workflow ready before review or finish."
      },
      "patchset": {
        "patchset_id": "RT-1/P-01",
        "revision_snapshot_id": "SNP-REV"
      },
      "patchset_ci_status": null,
      "patchset_refresh": null,
      "patchset_source": null,
      "policy": null,
      "review": {
        "approvals": 0,
        "blocking": 0,
        "task_approvals": 0,
        "team_approvals": 0
      },
      "steps": [
        {
          "code": "snapshot",
          "command": "ait snapshot create RT-1 --message \"reviewable snapshot\"",
          "detail": "Workspace changes are still dirty, so publishable finish state needs a fresh snapshot first.",
          "label": "Snapshot",
          "status": "pending"
        },
        {
          "code": "patchset",
          "command": "ait patchset publish RT-1 --summary \"review summary\"",
          "detail": "The base line `main` moved to `` after patchset `RT-1/P-01` was published.",
          "label": "Patchset",
          "status": "stale"
        },
        {
          "code": "attestation",
          "command": "ait attest put RT-1/P-01 --tests pass",
          "detail": "No attestation is recorded yet for patchset `RT-1/P-01`. Run workflow ready so it completes Patchset CI and records the compact Attestation gate statement.",
          "label": "Attestation",
          "status": "pending"
        },
        {
          "code": "review",
          "command": "ait config set --user-name \"<name>\"",
          "detail": "Code review: not required; Task review: pending; Team review: unavailable while Agent Teams is disabled. Supply the acting Reviewer's own name with `--reviewer` when recording automatic approval.",
          "label": "Review",
          "status": "pending"
        },
        {
          "code": "policy",
          "detail": "Policy is folded into `task finish` for Patchset `RT-1/P-01`; finish preflight will run the authoritative evaluation.",
          "label": "Policy",
          "status": "advisory"
        },
        {
          "code": "land",
          "command": "ait task finish RT-1",
          "detail": "Finish waits for review to clear first.",
          "label": "Finish",
          "status": "waiting"
        }
      ],
      "suggested_commands": [
        "ait workflow ready RT-1 --apply"
      ],
      "task": {
        "status": "active",
        "task_id": "RT-1"
      },
      "task_review": {
        "source": "built_in",
        "value": false
      },
      "workspace": {
        "changed_count": 0,
        "clean": false,
        "current_line": "unknown",
        "head_snapshot_id": null,
        "workspace_matches_patchset": null,
        "workspace_status": "dirty"
      },
      "worktree_retarget": null
    }
  },
  {
    "name": "project_workflow_ready_read_model",
    "context": {},
    "args": [
      {
        "task": {
          "task_id": "RT-1",
          "status": "active"
        },
        "change": {},
        "patchset": {
          "patchset_id": "RT-1/P-01",
          "revision_snapshot_id": "SNP-REV"
        },
        "target_line": "main",
        "base_line_name": "main"
      },
      false,
      false,
      false
    ],
    "result": {
      "base_line_name": "main",
      "change": null,
      "next_action": {
        "code": "snapshot_create",
        "command": "ait snapshot create RT-1 --message \"reviewable snapshot\"",
        "detail": "The workspace still has unsaved changes.",
        "summary": "Capture a fresh snapshot before publishing or refreshing the patchset."
      },
      "patchset": {
        "patchset_id": "RT-1/P-01",
        "revision_snapshot_id": "SNP-REV"
      },
      "steps": [
        {
          "code": "snapshot",
          "command": "ait snapshot create RT-1 --message \"reviewable snapshot\"",
          "detail": "Workspace changes are still dirty, so ready state needs a fresh snapshot first.",
          "label": "Snapshot",
          "status": "pending"
        },
        {
          "code": "patchset",
          "detail": "Patchset `RT-1/P-01` is published for Task `RT-1`.",
          "label": "Patchset",
          "status": "done"
        },
        {
          "code": "attestation",
          "command": "ait workflow ready RT-1 --apply",
          "detail": "No attestation is recorded yet for patchset `RT-1/P-01`. Workflow ready apply owns Patchset CI completion and then records the compact Attestation gate statement.",
          "label": "Attestation",
          "status": "pending"
        }
      ],
      "suggested_commands": [
        "ait snapshot create RT-1 --message \"reviewable snapshot\"",
        "ait patchset publish RT-1 --summary \"review summary\"",
        "ait attest put RT-1/P-01 --tests pass"
      ],
      "target_line": "main",
      "task": {
        "status": "active",
        "task_id": "RT-1"
      },
      "workflow_phase": "ready"
    }
  },
  {
    "name": "project_workflow_landed_read_model",
    "context": {
      "task_review": true
    },
    "args": [
      {
        "task": {
          "task_id": "RT-1",
          "status": "active"
        },
        "change": {},
        "patchset": {
          "patchset_id": "RT-1/P-01",
          "revision_snapshot_id": "SNP-REV"
        },
        "target_line": "main",
        "base_line_name": "main"
      }
    ],
    "result": {
      "attestation": null,
      "base_line": {
        "head_snapshot_id": null,
        "line_name": "main"
      },
      "change": null,
      "freshness": {
        "base_is_fresh": true,
        "patchset_base_snapshot_id": null,
        "patchset_revision_snapshot_id": "SNP-REV",
        "preflight_state": "not_required",
        "rebase_state": "idle",
        "recovery_required": false,
        "remote_base_snapshot_id": null,
        "worktree_needs_retarget": false
      },
      "landing_summary": null,
      "next_action": {
        "code": "workflow_ready",
        "command": "ait workflow ready RT-1 --apply",
        "detail": "The workspace still has unsaved changes.",
        "summary": "Run workflow ready before review or finish."
      },
      "patchset": {
        "patchset_id": "RT-1/P-01",
        "revision_snapshot_id": "SNP-REV"
      },
      "patchset_source": null,
      "policy": {
        "checks": [],
        "decision": "pass",
        "status_source": "landed_fast_path"
      },
      "review": {
        "approvals": 1,
        "blocking": 0,
        "current_patchset_id": "RT-1/P-01",
        "reviews": [],
        "task_approvals": 1,
        "team_approvals": 0
      },
      "steps": [
        {
          "code": "snapshot",
          "detail": "No additional authoring snapshot is required because the change is already applied.",
          "label": "Snapshot",
          "status": "done"
        },
        {
          "code": "patchset",
          "detail": "Patchset `RT-1/P-01` is already part of the accepted history for this change.",
          "label": "Patchset",
          "status": "done"
        },
        {
          "code": "attestation",
          "detail": "Finish already succeeded, so attestation gating has already cleared.",
          "label": "Attestation",
          "status": "done"
        },
        {
          "code": "review",
          "detail": "Finish already succeeded, so review requirements were already satisfied.",
          "label": "Review",
          "status": "done"
        },
        {
          "code": "policy",
          "detail": "Finish already succeeded, so policy has already cleared.",
          "label": "Policy",
          "status": "done"
        },
        {
          "code": "land",
          "detail": "Task `RT-1` already has accepted work applied to `main`.",
          "label": "Finish",
          "status": "done"
        }
      ],
      "suggested_commands": [
        "ait workflow ready RT-1 --apply"
      ],
      "task": {
        "status": "active",
        "task_id": "RT-1"
      },
      "workspace": {
        "changed_count": 0,
        "clean": false,
        "current_line": "unknown",
        "head_snapshot_id": null,
        "workspace_matches_patchset": true,
        "workspace_status": "dirty"
      }
    }
  },
  {
    "name": "project_workflow_land_full_read_model",
    "context": {
      "task_review": true
    },
    "args": [
      {
        "task": {
          "task_id": "RT-1",
          "status": "active"
        },
        "change": {},
        "patchset": {
          "patchset_id": "RT-1/P-01",
          "revision_snapshot_id": "SNP-REV"
        },
        "target_line": "main",
        "base_line_name": "main"
      },
      false
    ],
    "result": {
      "attestation": null,
      "base_line": {
        "head_snapshot_id": null,
        "line_name": "main"
      },
      "change": null,
      "freshness": {
        "base_is_fresh": false,
        "patchset_base_snapshot_id": null,
        "patchset_revision_snapshot_id": null,
        "preflight_state": "stale",
        "rebase_state": "idle",
        "recovery_required": true,
        "remote_base_snapshot_id": null,
        "worktree_needs_retarget": false
      },
      "landing_summary": null,
      "next_action": {
        "code": "workflow_ready",
        "command": "ait workflow ready RT-1 --apply",
        "detail": "The workspace still has unsaved changes.",
        "summary": "Run workflow ready before review or finish."
      },
      "patchset": {
        "patchset_id": "RT-1/P-01",
        "revision_snapshot_id": "SNP-REV"
      },
      "patchset_ci_status": null,
      "patchset_refresh": null,
      "patchset_source": null,
      "policy": null,
      "review": {
        "approvals": 0,
        "blocking": 0,
        "task_approvals": 0,
        "team_approvals": 0
      },
      "steps": [
        {
          "code": "snapshot",
          "command": "ait snapshot create RT-1 --message \"reviewable snapshot\"",
          "detail": "Workspace changes are still dirty, so publishable finish state needs a fresh snapshot first.",
          "label": "Snapshot",
          "status": "pending"
        },
        {
          "code": "patchset",
          "command": "ait patchset publish RT-1 --summary \"review summary\"",
          "detail": "The base line `main` moved to `` after patchset `RT-1/P-01` was published.",
          "label": "Patchset",
          "status": "stale"
        },
        {
          "code": "attestation",
          "command": "ait attest put RT-1/P-01 --tests pass",
          "detail": "No attestation is recorded yet for patchset `RT-1/P-01`. Run workflow ready so it completes Patchset CI and records the compact Attestation gate statement.",
          "label": "Attestation",
          "status": "pending"
        },
        {
          "code": "review",
          "command": "ait review task approve RT-1 --patchset RT-1/P-01 --message \"<functional validation>\"",
          "detail": "Code review: not required; Task review: pending; Team review: unavailable while Agent Teams is disabled.",
          "label": "Review",
          "status": "pending"
        },
        {
          "code": "policy",
          "detail": "Policy is folded into `task finish` for Patchset `RT-1/P-01`; finish preflight will run the authoritative evaluation.",
          "label": "Policy",
          "status": "advisory"
        },
        {
          "code": "land",
          "command": "ait task finish RT-1",
          "detail": "Finish waits for review to clear first.",
          "label": "Finish",
          "status": "waiting"
        }
      ],
      "suggested_commands": [
        "ait workflow ready RT-1 --apply"
      ],
      "task": {
        "status": "active",
        "task_id": "RT-1"
      },
      "task_review": {
        "value": true
      },
      "workspace": {
        "changed_count": 0,
        "clean": false,
        "current_line": "unknown",
        "head_snapshot_id": null,
        "workspace_matches_patchset": null,
        "workspace_status": "dirty"
      },
      "worktree_retarget": null
    }
  },
  {
    "name": "project_workflow_ready_read_model",
    "context": {
      "task_review": true
    },
    "args": [
      {
        "task": {
          "task_id": "RT-1",
          "status": "active"
        },
        "change": {},
        "patchset": {
          "patchset_id": "RT-1/P-01",
          "revision_snapshot_id": "SNP-REV"
        },
        "target_line": "main",
        "base_line_name": "main"
      },
      false,
      false,
      false
    ],
    "result": {
      "base_line_name": "main",
      "change": null,
      "next_action": {
        "code": "snapshot_create",
        "command": "ait snapshot create RT-1 --message \"reviewable snapshot\"",
        "detail": "The workspace still has unsaved changes.",
        "summary": "Capture a fresh snapshot before publishing or refreshing the patchset."
      },
      "patchset": {
        "patchset_id": "RT-1/P-01",
        "revision_snapshot_id": "SNP-REV"
      },
      "steps": [
        {
          "code": "snapshot",
          "command": "ait snapshot create RT-1 --message \"reviewable snapshot\"",
          "detail": "Workspace changes are still dirty, so ready state needs a fresh snapshot first.",
          "label": "Snapshot",
          "status": "pending"
        },
        {
          "code": "patchset",
          "detail": "Patchset `RT-1/P-01` is published for Task `RT-1`.",
          "label": "Patchset",
          "status": "done"
        },
        {
          "code": "attestation",
          "command": "ait workflow ready RT-1 --apply",
          "detail": "No attestation is recorded yet for patchset `RT-1/P-01`. Workflow ready apply owns Patchset CI completion and then records the compact Attestation gate statement.",
          "label": "Attestation",
          "status": "pending"
        }
      ],
      "suggested_commands": [
        "ait snapshot create RT-1 --message \"reviewable snapshot\"",
        "ait patchset publish RT-1 --summary \"review summary\"",
        "ait attest put RT-1/P-01 --tests pass"
      ],
      "target_line": "main",
      "task": {
        "status": "active",
        "task_id": "RT-1"
      },
      "workflow_phase": "ready"
    }
  }
];
  for (const fixture of cases) {
    const method = fixture.name.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());
    const context = Object.fromEntries(Object.entries(fixture.context)
      .map(([key, value]) => [key.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase()), value]));
    assert.deepEqual(runtime[method](context, ...fixture.args), fixture.result, fixture.name);
  }
  assert.throws(() => runtime.projectWorkflowReadyReadModel({}, [], false, false, false), nativeFailure);
});

test("real contextual workflow projection preserves native config overlay and explicit review precedence", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-workflow-context-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const runtime = new NativeRuntime();
  const basePath = path.join(root, "config.json");
  const overlayPath = path.join(root, "worktree.json");
  const baseText = '{"task_review":false,"workflow_ready_poll_seconds":30}';
  await writeFile(basePath, baseText);
  await writeFile(overlayPath, '{"task_review":true,"workflow_ready_poll_seconds":55}');
  const projectionContext = { configPath: ` ${basePath} `, worktreeConfigPath: overlayPath };
  const state = { task: { task_id: "RT-1", status: "active" }, change: {},
    patchset: { patchset_id: "RT-1/P-01", revision_snapshot_id: "SNP-REV" }, target_line: "main", base_line_name: "main" };
  assert.deepEqual(runtime.projectWorkflowLandFullReadModel(projectionContext, state, false).task_review,
    { value: true, source: "repo_config" });
  assert.deepEqual(runtime.projectWorkflowLandFullReadModel({ ...projectionContext, taskReview: false }, state, false).task_review,
    { value: false });
  assert.equal(await readFile(basePath, "utf8"), baseText);
});

test("real retained release SDK preserves native validation and unsupported Local authority", async (context) => {
  const { root, runtime } = await repository(context);
  const input = { releaseId: "REL-SDK", repoName: "fixture", version: "1.2.0", lineName: "main",
    snapshotId: "SNP-FIXTURE", manifestHash: "fixture-hash", profile: "local-cli",
    metadataJson: '{ "n": 18446744073709551615 }' };
  const unsupported = "Local workflow event/release/ownership authority is unsupported; Task and Change use the explicit local Binary DB authority.";
  for (const invoke of [
    () => runtime.listWorkflowReleases(root),
    () => runtime.getWorkflowRelease(root, "absent"),
    () => runtime.createWorkflowRelease(root, input),
    () => runtime.updateWorkflowRelease(root, "REL-SDK", { metadataJson: "{" }),
    () => runtime.updateWorkflowRelease(root, "REL-SDK", { metadataSet: true }),
    () => runtime.taskWorkflowReleaseCheck(root, "absent"),
    () => runtime.taskWorkflowReleaseBuild(root, "absent"),
    () => runtime.taskWorkflowReleaseFormula(root, "absent", "fixture"),
    () => runtime.taskWorkflowReleaseShow(root, "absent"),
    () => runtime.taskWorkflowReleasePublish(root, "absent"),
  ]) assert.throws(invoke, (error) => {
    nativeFailure(error); assert.equal(error.message, unsupported); return true;
  });
  for (const [invoke, message] of [
    [() => runtime.createWorkflowRelease(root, { ...input, metadataJson: "{" }), /metadata_json/],
    [() => runtime.updateWorkflowRelease(root, "REL-SDK", { metadataSet: true, metadataJson: "{" }), /metadata_json/],
    [() => runtime.updateWorkflowRelease(root, "REL-SDK", { formulaSet: true, formulaJson: "{" }), /formula_json/],
    [() => runtime.taskWorkflowReleaseCandidateCreate(root, "1.2.0", "main", "unknown-profile"), /profile/],
    [() => runtime.taskWorkflowReleaseCandidateCreate(root, "1.2.0", "main", "local-cli"), /head snapshot/],
    [() => runtime.taskWorkflowReleaseShow(root, "absent", "missing-remote"), /missing-remote/],
  ]) assert.throws(invoke, (error) => {
    nativeFailure(error); assert.match(error.message, message); return true;
  });
});

test("real active release operations retain native adapter execution and family CLI routing", async (context) => {
  const fixture = await realpath(await mkdtemp(path.join(os.tmpdir(), "ait-node-release-operations-")));
  context.after(async () => {
    await makeFixtureDirectoriesWritable(fixture);
    await rm(fixture, { recursive: true, force: true });
  });
  const source = path.join(fixture, "source");
  const root = path.join(fixture, "repository");
  const hooks = path.join(fixture, "empty-hooks");
  const gitConfig = path.join(fixture, "empty-git-config");
  await mkdir(source);
  await mkdir(root);
  await mkdir(hooks);
  await writeFile(gitConfig, "");
  const target = "x86_64-unknown-linux-gnu";
  const manifest = { schema: "ait.release.family/v3",
    family: { name: "ait-native", version: "1.0.0-rc.1", channel: "rc", tag: "v1.0.0-rc.1" }, targets: [target],
    public_source: { model: "release-monorepo", identity: "weita2026/ait-native", product_document: "docs/distribution.md",
      family_manifest: "ait-release-family.json", mapping_manifest: "ait-monorepo-source.json",
      build_entrypoints: { unix: "build-release.sh", windows: "build-release.ps1", implementation: "build-release.mjs" },
      subtrees: [{ source_repository: "ait-core", path: "ait-core", transforms: [] }], transforms: [] },
    components: [{ id: "ait", source_repository: "ait-core", source_snapshot: "SNP-111111111111", ecosystem: "native",
      license: "Apache-2.0", version_scheme: "family", version: "1.0.0-rc.1", artifacts: [{ kind: "native-executable", targets: [target] }] }],
    distributions: [{ channel: "github", role: "product", identity: "weita2026/ait-native", components: ["ait"], targets: [target] }], compatibility: {} };
  const executable = process.platform === "win32" ? "component-bin.exe" : "component-bin";
  const adapter = { schema: "ait.release.adapter/v1", package: { name: "adapter-fixture", version: "1.2.3",
    license_files: [{ path: "LICENSE", role: "license" }, { path: "NOTICE", role: "notice" }] },
    components: [{ id: "component", ecosystem: "rust-fixture", working_directory: ".", dependency_files: ["component.rs"],
      commands: { test: [["rustc", "--version"]], build: [["rustc", "component.rs", "-o", executable]],
        smoke: [[process.platform === "win32" ? ".\\component-bin.exe" : "./component-bin"]] },
      artifacts: [{ path: executable, kind: "native-test-binary" }] }] };
  await writeFile(path.join(source, "ait-release-family.json"), JSON.stringify(manifest));
  await writeFile(path.join(source, "ait-release.json"), JSON.stringify(adapter));
  await writeFile(path.join(source, "LICENSE"), "fixture license\n");
  await writeFile(path.join(source, "NOTICE"), "fixture notice\n");
  await writeFile(path.join(source, "component.rs"), 'fn main() { println!("adapter smoke ok"); }\n');
  function git(...args) {
    const result = spawnSync("git", ["-c", `core.hooksPath=${hooks}`, "-c", "core.autocrlf=false",
      "-c", "user.name=Release Fixture", "-c", "user.email=fixture@example.test", "-c", "commit.gpgSign=false", ...args], {
      cwd: source, encoding: "utf8", windowsHide: true,
      env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: gitConfig },
    });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
  }
  git("init", "-q", "-b", "main");
  git("add", ".");
  git("commit", "-q", "-m", "Owned release source fixture");
  const runtime = new NativeRuntime();
  runtime.taskWorkflowInit(root);
  runtime.taskWorkflowConfigSet(root, { agent_teams: "disabled", sprint: "off" });
  runtime.gitImport(root, source, { allBranchesAndTags: true });
  for (const [operation, command] of [["releaseAdapterCheck", "check"], ["releaseAdapterBuild", "build"]]) {
    const receipt = runtime[operation](root, "1.2.3");
    assert.equal(receipt.contract, "ait.release.adapter.receipt/v1");
    assert.equal(receipt.check_summary.blocking, 0);
    const result = cli(root, "release", "adapter", command, "--version", "1.2.3", "--json");
    assert.equal(result.status, 0, result.stderr);
    assert.equal(runtime.decodeJson(result.stdout).check_summary.blocking, 0);
  }
  assert.throws(() => runtime.releaseCandidateCreate(root, "1.0.0-rc.1", { channel: "stable" }), nativeFailure);
  const candidate = runtime.releaseCandidateCreate(root, "1.0.0-rc.1", { channel: "rc" });
  assert.equal(candidate.profile, "family");
  assert.equal(candidate.authority.persistence, "portable_dist_dossier");
  assert.equal(candidate.authority.binary_db_layout_changed, false);
  const id = candidate.release_id;
  assert.equal(runtime.releaseCandidateCreate(root, "1.0.0-rc.1", { profile: "family", channel: "rc" }).release_id, id);
  const replayed = cli(root, "release", "candidate", "create", "--version", "1.0.0-rc.1", "--channel", "rc", "--json");
  assert.equal(replayed.status, 0, replayed.stderr);
  assert.equal(runtime.decodeJson(replayed.stdout).release_id, id);
  const shown = cli(root, "release", "show", id);
  assert.equal(shown.status, 0, shown.stderr);
  assert.equal(shown.stdout, `${runtime.formatRelease(runtime.releaseShow(root, id))}\n`);
  for (const args of [
    ["check", id], ["build", id], ["check", id, "--tests-command", "never execute"],
    ["build", id, "--native-matrix-dir", fixture], ["show", id, "--remote", "missing-remote"],
    ["publish", id], ["package", id, "--channel", "pypi"], ["promote", id, "--channel", "rc"],
  ]) {
    const rejected = cli(root, "release", ...args, "--json");
    assert.equal(rejected.status, runtime.domainErrorExitCode());
    assert.equal(rejected.stdout, "");
    assert.ok(rejected.stderr.length > 0);
  }
  assert.throws(() => runtime.releaseNativeBundle(root, "absent", fixture), nativeFailure);
  assert.throws(() => runtime.releaseNativeSource(root, { releaseId: "absent", target, sourceDir: fixture,
    runner: "fixture", runnerImage: "fixture", rustToolchain: "fixture", rustcPath: "rustc" }), nativeFailure);
  assert.throws(() => runtime.releaseFormula(root, "absent", "fixture"), nativeFailure);
  assert.throws(() => runtime.releaseCandidateCreate(root, "1.0.0-rc.1", { publicSourceRoot: fixture }), nativeFailure);
  assert.throws(() => runtime.releaseCheck(root, "absent", { receipts: fixture }), nativeFailure);
  assert.throws(() => runtime.releaseBuild(root, "absent", { publicSourceRoot: fixture }), nativeFailure);
});

test("real release formatter preserves native source and family detail without a final newline", () => {
  const runtime = new NativeRuntime();
  assert.equal(runtime.formatRelease({ command: "release native-source", release_id: "R", status: "recorded", version: "1",
    snapshot_id: "S", profile: "native", target: "T", descriptor_path: "descriptor", descriptor_sha256: "sha" }),
  "ait release native-source R\nstatus: recorded\nversion: 1\nsnapshot: S\nprofile: native\ntarget: T\ndescriptor: descriptor\ndescriptor sha256: sha");
  assert.equal(runtime.formatRelease({ release_id: "R", status: "ready_for_protected_ci", version: "1", line: "main", snapshot_id: "S", profile: "family", channel: "rc",
    family: { components: [{}], targets: ["T"] }, checks: [{ check_id: "C", blocking: true, status: "fail", details: "D" }],
    check_summary: { total: 1, blocking: 1, decision: "fail" }, artifacts: [{ kind: "wheel", path: "a.whl" }],
    next_action: { detail: "next step" }, formula: { path: "f.rb" } }),
  "ait release R\nstatus: ready_for_protected_ci\nversion: 1\nline: main\nsnapshot: S\nprofile: family\nchecks: fail (1 blocking / 1 recorded)\nartifacts: wheel:a.whl\nfamily channel: rc; components: 1; targets: 1\npromotion: protected CI handoff only; registry writes: no\nnext: next step\n\nrelease check detail\n- C [fail]: D\nformula draft: f.rb");
});

test("real External and disabled Team CLI services preserve reports, mutations and native statuses", async (context) => {
  const { root, runtime } = await repository(context);
  const linked = path.join(path.dirname(root), "linked fixture");
  await mkdir(linked);
  const status = runtime.externalStatus(root);
  assert.deepEqual(status.externals, []);
  const statusCli = cli(root, "external", "status");
  assert.equal(statusCli.status, 0, statusCli.stderr);
  assert.equal(statusCli.stdout, `${runtime.formatExternalStatus(status)}\n`);
  const diagnostic = runtime.externalDoctor(root, true);
  assert.equal(diagnostic.exitCode, 0);
  assert.equal(diagnostic.payload.release_ready, true);
  const update = runtime.externalUpdate(root);
  assert.equal(update.lockfile_changed, true);
  assert.equal(update.manifest_changed, false);
  const locked = cli(root, "external", "update", "--locked", "--validate", "--no-recursive", "--json");
  assert.equal(locked.status, 0, locked.stderr);
  assert.equal(runtime.decodeJson(locked.stdout).locked, true);
  assert.equal(runtime.decodeJson(locked.stdout).recursive, false);
  assert.equal(runtime.decodeJson(locked.stdout).validated, true);
  for (const options of [{ snapshot: "S", latest: true }, { locked: true, name: "dep" }, { snapshot: "S" }, { latest: true }, { name: "dep" }]) {
    assert.throws(() => runtime.externalUpdate(root, options), nativeFailure);
  }
  await rm(path.join(root, "ait-external.lock"));
  await writeFile(path.join(root, "ait-external.toml"), '[[external]]\nname = "dep"\nrepo_name = "dependency"\nrepository_index = 1\nremote = "origin"\nline = "main"\nsnapshot = "SNP-MISSING"\nmaterialize_to = ".ait-external/dep"\nlicense = "Apache-2.0"\n');
  const blocked = runtime.externalDoctor(root, true);
  const advisory = runtime.externalDoctor(root, false);
  assert.equal(blocked.exitCode, 2);
  assert.equal(blocked.payload.release_ready, false);
  assert.equal(advisory.exitCode, 0);
  assert.deepEqual(advisory.payload, blocked.payload);
  const doctorCli = cli(root, "external", "doctor", "--fail-on-blocking", "--json");
  assert.equal(doctorCli.status, 2);
  assert.equal(doctorCli.stderr, "");
  assert.equal(runtime.decodeJson(doctorCli.stdout).release_ready, false);
  assert.throws(() => runtime.externalLink(root, "dep", root), nativeFailure);
  const link = cli(root, "external", "link", "dep", linked, "--json");
  assert.equal(link.status, 0, link.stderr);
  assert.equal(runtime.decodeJson(link.stdout).state, "linked");
  assert.equal(runtime.decodeJson(link.stdout).changed, true);
  const unlink = cli(root, "external", "unlink", "dep", "--json");
  assert.equal(unlink.status, 0, unlink.stderr);
  assert.equal(runtime.decodeJson(unlink.stdout).restore_state, "skipped_no_lockfile");
  assert.equal(runtime.externalUnlink(root, "dep").changed, false);
  const contacts = runtime.teamContacts(root);
  assert.equal(contacts.exitCode, 0);
  assert.equal(contacts.payload.team_enabled, false);
  assert.deepEqual(contacts.payload.members, []);
  assert.equal(runtime.formatTeamContacts(contacts.payload), "Agent Teams disabled.");
  const contactCli = cli(root, "team", "contacts", "--json");
  assert.equal(contactCli.status, 0, contactCli.stderr);
  assert.equal(runtime.decodeJson(contactCli.stdout).team_enabled, false);
  const ensured = runtime.teamSessionEnsure(root, 1);
  assert.equal(ensured.exitCode, 1);
  assert.equal(ensured.payload.work_started, false);
  assert.equal(runtime.formatTeamSessionEnsure(ensured.payload, 1), 'Team session: "pending"\nAgent: 1\nSession ID: unbound\nAction: none\nAgent Teams is disabled');
  const ensureCli = cli(root, "team", "session", "ensure", "--agent", "1");
  assert.equal(ensureCli.status, 1);
  assert.equal(ensureCli.stderr, "");
  assert.equal(ensureCli.stdout, `${runtime.formatTeamSessionEnsure(ensured.payload, 1)}\n`);
  const acknowledged = cli(root, "team", "session", "acknowledge", "--activation", "invalid", "--json");
  assert.equal(acknowledged.status, 1);
  assert.equal(acknowledged.stderr, "");
  assert.equal(runtime.decodeJson(acknowledged.stdout).work_started, false);
  assert.equal(runtime.teamSessionAcknowledge(root, "invalid").exitCode, 1);
  assert.throws(() => runtime.teamSessionEnsure(root, 4294967296n), nativeFailure);
  assert.throws(() => runtime.teamSessionEnsure(root, 1, -1), nativeFailure);
  assert.throws(() => runtime.formatTeamSessionEnsure({}, 4294967296n), nativeFailure);
});

test("real Binary DB maintenance CLI rejects unsupported sources before discovery or writes", async (context) => {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "ait-node-binary-db-")));
  context.after(() => rm(root, { recursive: true, force: true }));
  const generation = path.join(root, "generation");
  const runtime = new NativeRuntime();
  for (const selector of ["", "u32", "u64-second-v0"]) {
    assert.throws(() => runtime.binaryDbUpgradeU64Seconds(root, generation, selector, true), (error) => {
      nativeFailure(error); assert.match(error.message, /expected exact selector "u32-time-v0"/); return true;
    });
    assert.deepEqual(await readdir(root), []);
  }
  const rejected = cli(root, "binary-db", "upgrade-u64-seconds", "--repo-root", root,
    "--generation-root", generation, "--source-time-width", "u32", "--activate", "--json");
  assert.equal(rejected.status, runtime.domainErrorExitCode());
  assert.equal(rejected.stdout, "");
  assert.match(rejected.stderr, /expected exact selector "u32-time-v0"/);
  assert.deepEqual(await readdir(root), []);
  await mkdir(generation);
  assert.throws(() => runtime.binaryDbUpgradeU64Seconds(root, generation, "u32-time-v0"), (error) => {
    nativeFailure(error); assert.match(error.message, /output root must not already exist/); return true;
  });
  assert.throws(() => runtime.binaryDbAgentPool50(root, generation, true), (error) => {
    nativeFailure(error); assert.match(error.message, /restricted to ait-core/); return true;
  });
  const agentCli = cli(root, "binary-db", "agent-pool50", "--repo-root", root, "--generation-root", generation, "--activate", "--json");
  assert.equal(agentCli.status, runtime.domainErrorExitCode());
  assert.equal(agentCli.stdout, "");
  assert.match(agentCli.stderr, /restricted to ait-core/);
  assert.deepEqual(await readdir(generation), []);
});

test("real Binary DB formatters retain all branches and full-width integer text", () => {
  const runtime = new NativeRuntime();
  assert.equal(runtime.formatBinaryDbAgentPool50({ status: "already_converted", activated: true }),
    "ait binary-db agent-pool50\nstatus: already converted");
  assert.equal(runtime.formatBinaryDbAgentPool50({ status: "staged_retry", activated: false }),
    "ait binary-db agent-pool50\nstatus: staged retry\nactivated: false");
  assert.equal(runtime.formatBinaryDbAgentPool50({ generation_root: "out", activated: true, retained_previous_authority: "before" }),
    "ait binary-db agent-pool50\ngeneration root: out\nactivated: true\nretained previous authority: before");
  assert.equal(runtime.formatBinaryDbUpgradeU64Seconds({ repo_name: "fixture", source_time_width: "u32-time-v0", target_time_width: "u64-second-v0",
    generation_root: "out", source_bytes: 18446744073709551615n, target_bytes: 42, rebuilt_index_file_count: 2, activated: false, retained_previous_authority: null }),
  "ait binary-db upgrade-u64-seconds\nrepository: fixture\nsource time width: u32-time-v0\ntarget time width: u64-second-v0\ngeneration root: out\nsource bytes: 18446744073709551615\ntarget bytes: 42\nrebuilt content indexes: 2\nactivated: false");
});

test("real Snapshot authoring scalar preserves native validation without repository discovery", () => {
  const runtime = new NativeRuntime();
  for (const value of ["LT-0001", "LCT-1159", "LT-0001/C-01"]) {
    assert.equal(runtime.parseSnapshotAuthoringReference(`  ${value}  `), value);
  }
  for (const value of ["", "C-01", "SNP-123456789ABC", "invalid"]) {
    assert.throws(() => runtime.parseSnapshotAuthoringReference(value), (error) => {
      nativeFailure(error); assert.equal(error.message, "Snapshot creation requires a Task ID."); return true;
    });
  }
});

test("real remote head preview walks ancestry without opening damaged local authority", { timeout: 15000 }, async (context) => {
  const runtime = new NativeRuntime();
  const prefix = "/v1/native/repository-authorities/7";
  function manifest(id, parents) {
    return { contract: "ait.remote_sync.zstd_bulk.import_manifest.v1", repo_name: "fixture", snapshot_id: id,
      snapshots: [{ snapshot_id: id, parent_snapshot_ids: parents, primary_parent_snapshot_id: parents[0] ?? null,
        parent_snapshot_id: parents[0] ?? null, root_tree_pack_id: "TPK-FIXTURE", root_entry_ordinal: 0,
        manifest_hash: "manifest", snapshot_kind: "workspace", file_count: 0, total_bytes: 0, created_at: "2026-10-26T00:00:00Z" }],
      object_packs: [], tree_packs: [{ pack_id: "TPK-FIXTURE", pack_format: "ait-tree-pack-v2-zstd-chunked", tree_count: 1,
        total_bytes: 0, pack_index_entry_name: "index", pack_index_checksum: "checksum", created_at: "2026-10-26T00:00:00Z" }],
      blob_locators: [], tree_locators: [{ tree_id: "TRE-FIXTURE", entry_count: 0, tree_pack_id: "TPK-FIXTURE",
        tree_pack_checksum: "checksum", created_at: "2026-10-26T00:00:00Z" }], line_update: null };
  }
  const server = await planHttpFixture(context, {
    [`GET ${prefix}/lines/main`]: { body: runtime.encodeJson({ repo_name: "fixture", line_name: "main", head_snapshot_id: "SNP-HEAD" }) },
    [`GET ${prefix}/remote-sync/zstd-bulk/import-manifests/SNP-HEAD`]: { body: runtime.encodeJson(manifest("SNP-HEAD", ["SNP-ROOT"])) },
    [`GET ${prefix}/remote-sync/zstd-bulk/import-manifests/SNP-ROOT`]: { body: runtime.encodeJson(manifest("SNP-ROOT", [])) },
  });
  const temporary = await realpath(await mkdtemp(path.join(os.tmpdir(), "ait-node-head-recovery-")));
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const root = path.join(temporary, "fixture");
  await mkdir(path.join(root, ".ait"), { recursive: true });
  await writeFile(path.join(root, ".ait", "config.json"), runtime.encodeJson({ name: "fixture", repository_index: 7,
    default_line: "main", default_remote: "origin", remotes: { origin: { url: server.baseUrl, repo_name: "fixture" } } }));
  const authority = path.join(root, ".ait", "binary-db");
  const damaged = "deliberately damaged authority";
  await writeFile(authority, damaged);
  const preview = runtime.remoteRecoverHead(root);
  assert.equal(preview.apply, false);
  assert.equal(preview.snapshot_id, "SNP-HEAD");
  assert.equal(preview.history_mode, "complete_ancestry");
  assert.equal(preview.reachable_snapshot_count, 2);
  assert.equal(preview.object_pack_count, 0);
  assert.equal(preview.tree_pack_count, 1);
  assert.deepEqual(preview.source_parent_snapshot_ids, ["SNP-ROOT"]);
  assert.match(runtime.formatRemoteRecoverHead(preview), /^ait remote recover-head\n/);
  const result = cli(root, "remote", "recover-head", "--jobs", "64", "--json");
  assert.equal(result.status, 0, result.stderr);
  assert.equal(runtime.decodeJson(result.stdout).reachable_snapshot_count, 2);
  assert.equal(await readFile(authority, "utf8"), damaged);
  for (const jobs of [0, 65, -1, 18446744073709551615n]) assert.throws(() => runtime.remoteRecoverHead(root, { jobs }), nativeFailure);
  assert.throws(() => runtime.remoteRecoverHead(root, { remoteName: "absent" }), nativeFailure);
  const calls = await server.requests();
  assert.equal(calls.length, 6);
  assert.ok(calls.every((request) => request.method === "GET"));
});

test("real retained SDK values remain native data and diagnostic metadata", () => {
  const runtime = new NativeRuntime();
  const addon = runtime.loadAddon();
  for (const name of ["CODE_REVIEW_SUMMARY_TEMPLATE", "CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE", "CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND",
    "DEFAULT_ID_NAMESPACE_PREFIX", "LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX", "REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX", "RUNTIME_DATA_ENV",
    "LEGACY_SERVER_DATA_ENV", "CONTRACT_VERSION", "PLAN_CONTRACT_VERSION", "TARGET_TRIPLE"]) {
    assert.equal(typeof runtime[name], "string");
    assert.equal(runtime[name], addon[name]);
  }
  assert.equal(runtime.CODE_REVIEW_SUMMARY_TEMPLATE, runtime.renderCodeReviewSummaryTemplate("inline"));
  assert.equal(runtime.CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE, runtime.renderCodeReviewSummaryTemplate("numbered"));
  assert.equal(runtime.DEFAULT_ID_NAMESPACE_PREFIX, runtime.normalizeIdNamespacePrefix());
  assert.equal(runtime.CONTRACT_VERSION, "task-close-v1");
  assert.equal(runtime.PLAN_CONTRACT_VERSION, "plan-foundation-v7");
  assert.equal(runtime.DEFAULT_REQUEST_TIMEOUT_SECONDS, DEFAULT_REQUEST_TIMEOUT_SECONDS);
  assert.equal(runtime.LONG_RUNNING_REQUEST_TIMEOUT_SECONDS, LONG_RUNNING_REQUEST_TIMEOUT_SECONDS);
  assert.ok(runtime.workflowIdFamilies().includes("T"));
  assert.ok(runtime.workflowTaskChangeOriginNamespacePrefixes().every((value) => typeof value === "string"));
  assert.ok(runtime.policyRequirementFlags().every((value) => typeof value === "string"));
});

test("real guide and review template CLI preserve repository admission and native text", async (context) => {
  const { root, runtime } = await repository(context);
  const guide = runtime.workflowGuide();
  const shown = cli(root, "workflow", "guide");
  assert.equal(shown.status, 0, shown.stderr);
  assert.equal(shown.stderr, "");
  assert.equal(shown.stdout, `${runtime.formatWorkflowGuide(guide)}\n`);
  for (const topic of ["inventory", " FINISH "]) {
    const result = cli(root, "workflow", "guide", topic);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, `${runtime.formatWorkflowGuide(runtime.workflowGuide(topic))}\n`);
  }
  assert.throws(() => runtime.workflowGuide("unknown"), nativeFailure);
  const unknown = cli(root, "workflow", "guide", "unknown");
  assert.equal(unknown.status, runtime.domainErrorExitCode());
  assert.match(unknown.stderr, /Unknown workflow guide topic: unknown/);
  for (const style of ["inline", "numbered"]) {
    const template = runtime.taskWorkflowReviewCodeTemplate({ style });
    const rendered = cli(root, "review", "code", "template", "--style", style);
    assert.equal(rendered.status, 0, rendered.stderr);
    assert.equal(rendered.stdout, `${runtime.formatReviewCodeTemplate(template)}\n`);
    const json = cli(root, "review", "code", "template", "--style", style, "--json");
    assert.equal(json.status, 0, json.stderr);
    assert.deepEqual(runtime.decodeJson(json.stdout), template);
  }
  const invalidJson = cli(root, "workflow", "guide", "--json");
  assert.equal(invalidJson.status, 2);
  assert.equal(invalidJson.stdout, "");
  assert.match(invalidJson.stderr, /unexpected argument '--json'/);
  const outside = await realpath(await mkdtemp(path.join(os.tmpdir(), "ait-node-guide-admission-")));
  context.after(() => rm(outside, { recursive: true, force: true }));
  for (const args of [["workflow", "guide"], ["review", "code", "template"]]) {
    const rejected = cli(outside, ...args);
    assert.equal(rejected.status, runtime.domainErrorExitCode());
    assert.equal(rejected.stdout, "");
    assert.ok(rejected.stderr.length > 0);
  }
});

test("real reconcile preserves checked native-size inputs and full CLI domain errors", async (context) => {
  const { root, runtime } = await repository(context);
  const missing = path.join(path.dirname(root), "uninitialized");
  await mkdir(missing);
  const rangeError = "--limit must be between 1 and 10000.";
  function stable(payload) {
    const { captured_at, ...rest } = payload;
    return rest;
  }
  for (const value of [undefined, 1, 1n, 10000, 10000n]) {
    const payload = runtime.workflowReconcile(root, value === undefined ? undefined : { limit: value });
    assert.equal(payload.limit, value === undefined ? 100 : Number(value));
    assert.equal(payload.status, "completed");
    assert.equal(payload.mutated, false);
    const result = cli(root, "workflow", "reconcile", ...(value === undefined ? [] : [`--limit=${value}`]), "--json");
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(stable(runtime.decodeJson(result.stdout)), stable(payload));
  }
  for (const value of [0, 0n, 10001, 10001n, 4294967297, 4294967297n,
    Number.MAX_SAFE_INTEGER, 9007199254740993n, 18446744073709551615n]) {
    assert.throws(() => runtime.workflowReconcile(root, { limit: value }), (error) => {
      nativeFailure(error); assert.equal(error.message, rangeError); return true;
    });
    const result = cli(root, "workflow", "reconcile", `--limit=${value}`, "--json");
    assert.equal(result.status, runtime.domainErrorExitCode());
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, `${runtime.formatDomainError(rangeError)}\n`);
  }
  for (const [limit, error] of [
    [-1, /nonnegative native-size integer/], [-1n, /nonnegative native-size integer/],
    [1.5, /nonnegative native-size integer/], [NaN, /Failed to convert js number/],
    [Infinity, /Failed to convert js number/], [Number.MAX_SAFE_INTEGER + 1, /Unsafe JavaScript integer/],
    [18446744073709551616n, /outside the native JSON i64\/u64 range/],
  ]) {
    assert.throws(() => runtime.workflowReconcile(root, { limit, apply: true, safeOnly: true }), error);
    assert.throws(() => runtime.workflowReconcile(missing, { limit }), error);
  }
  const overflow = cli(root, "workflow", "reconcile", "--limit=18446744073709551616", "--json");
  assert.equal(overflow.status, 2);
  assert.equal(overflow.stdout, "");
  assert.equal(runtime.workflowReconcile(root).limit, 100);
  assert.deepEqual(runtime.taskWorkflowTaskList(root, { local: true, all: true }), []);
  assert.deepEqual(runtime.taskWorkflowChangeList(root, { local: true }), []);
});
