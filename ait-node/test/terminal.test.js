import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import os from "node:os";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { runTerminal as runFrontend, terminalExitCode, reportTerminalError } from "../src/terminal.js";
import { NativeResolutionError } from "../src/errors.js";
import { terminalHelp, terminalHelpPaths } from "../src/terminal-help.js";

function mockPublicPatchsetArgument(value) {
  const normalized = value.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
  if (!normalized || /^[0-9]+$/.test(normalized)) {
    throw Object.assign(new Error("native public Patchset argument rejected"), { code: "GenericFailure" });
  }
  return normalized;
}

function runTerminal(args, { runtime = {}, ...options } = {}) {
  return runFrontend(args, {
    stdoutIsTTY: false,
    ...options,
    runtime: {
      encodeJson: (value, pretty) => JSON.stringify(value, null, pretty ? 2 : undefined),
      configurationError: () => null,
      repositoryAuthorityRoot: (root) => root,
      formatDomainError: (message) => `Error: ${message}`,
      domainErrorExitCode: () => 1,
      validatePublicPatchsetArgument: mockPublicPatchsetArgument,
      parseSnapshotAuthoringReference: (value) => value,
      validateReviewPatchsetSelector: (value) => value.startsWith("@") ? value : mockPublicPatchsetArgument(value),
      ...runtime,
    },
  });
}

test("terminal scope and identifiers become typed function parameters", () => {
  const calls = [];
  let output = "";
  const status = runTerminal(["task", "show", "RCT-0007", "--remote", "secondary", "--json"], {
    cwd: "/repository with spaces",
    runtime: {
      taskWorkflowTaskShow(...parameters) {
        calls.push(parameters);
        return { task_id: "RCT-0007", remote_name: "secondary" };
      },
    },
    write: (text) => { output += text; },
  });
  assert.equal(status, 0);
  assert.deepEqual(calls, [["/repository with spaces", "RCT-0007", {
    local: undefined, remoteName: "secondary",
  }]]);
  assert.deepEqual(JSON.parse(output), { task_id: "RCT-0007", remote_name: "secondary" });
});

test("conflicting scope fails before any native operation", () => {
  let called = false;
  assert.throws(() => runTerminal(["task", "list", "--local", "--remote", "origin"], {
    runtime: { taskWorkflowTaskList() { called = true; } },
  }), (error) => {
    assert.equal(terminalExitCode(error), 2);
    assert.match(error.message, /conflicts/);
    return true;
  });
  assert.equal(called, false);
});

test("unknown flags and invalid terminal values do not reach native code", () => {
  for (const args of [
    ["init", "--invented-option"],
    ["task", "show"],
    ["status", "unexpected"],
    ["status", "\0"],
    ["not-a-command"],
  ]) {
    assert.throws(() => runTerminal(args, { runtime: {} }), (error) => {
      assert.equal(terminalExitCode(error), 2);
      return true;
    });
  }
});

test("native errors retain their identity across the JavaScript frontend", () => {
  const nativeError = Object.assign(new Error("Unknown Task: LNT-9999"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["task", "show", "LNT-9999"], {
    runtime: { taskWorkflowTaskShow() { throw nativeError; } },
  }), (error) => error === nativeError);
  assert.equal(terminalExitCode(nativeError), 1);
});

test("init options are converted by JavaScript without forwarding terminal tokens", () => {
  let received;
  runTerminal(["init", "--policy-profile", "team", "--repair-existing", "--json"], {
    cwd: "/isolated-repository",
    runtime: { initializeRepository(...parameters) { received = parameters; return { initialized: true }; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/isolated-repository", { policyProfile: "team", repairExisting: true }]);
});

test("config changes retain domain field names and omit output flags", () => {
  let received;
  runTerminal(["config", "set", "--sprint", "off", "--agent-teams", "disabled", "--json"], {
    cwd: "/isolated-repository",
    runtime: { configureRepository(...parameters) { received = parameters; return {}; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/isolated-repository", { sprint: "off", agent_teams: "disabled" }]);
});

test("remote add converts named options into the existing domain request", () => {
  let received;
  runTerminal(["remote", "add", "secondary", "http://127.0.0.1:12345", "--default", "--json"], {
    cwd: "/isolated-repository",
    runtime: { taskWorkflowRemoteAdd(...parameters) { received = parameters; return {}; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/isolated-repository", {
    name: "secondary", url: "http://127.0.0.1:12345", make_default: true,
  }]);
});

test("Task start passes the exact Plan source and edit-root to native orchestration", () => {
  let received;
  let projected;
  const payload = { task_id: "LNT-0007", native_result: true };
  runTerminal(["task", "start", "--from", "docs/sprints/work.md#exact/ref", "--intent", "repair", "--edit-root", String.raw`C:\Task roots\work`, "--local", "--json", "--full"], {
    cwd: "/repository",
    runtime: {
      taskStart(...parameters) { received = parameters; return payload; },
      projectTaskStart(value, full) { projected = [value, full]; return value; },
    },
    write: () => {},
  });
  assert.deepEqual(received, ["/repository", {
    local: true, remoteName: undefined, intent: "repair", title: undefined,
    editRoot: String.raw`C:\Task roots\work`, source: "docs/sprints/work.md#exact/ref",
    humanProgress: false,
  }]);
  assert.deepEqual(projected, [payload, true]);
});

test("Task and snapshot syntax conflicts fail before mutation", () => {
  for (const args of [
    ["task", "start", "--title", "work", "--from", "card.md#ref", "--intent", "repair"],
    ["task", "start", "--intent", "repair"],
    ["task", "quick", "--title", "work"],
    ["task", "quick", "--intent", "one", "--intent", "two"],
    ["task", "finish", "LNT-0007", "--full"],
    ["snapshot", "create", "LNT-0007", "--resume", "SNP-EXACT", "-m", "replace"],
  ]) {
    assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  }
});

test("finish uses native formatting and exit projection without inferring success", () => {
  const payload = { status: "accepted", closeout_pending: true };
  let received;
  let output = "";
  const status = runTerminal(["task", "finish", "LNT-0007", "--local", "--sync-plan-markdown", "--message", "finish"], {
    cwd: "/repository",
    runtime: {
      taskFinish(...parameters) { received = parameters; return payload; },
      formatTaskFinish(value, requested) { assert.equal(value, payload); assert.equal(requested, "LNT-0007"); return "native finish result"; },
      taskFinishExitCode(value) { assert.equal(value, payload); return 17; },
    },
    write: (text) => { output += text; },
  });
  assert.deepEqual(received, ["/repository", "LNT-0007", {
    local: true, remoteName: undefined, message: "finish", syncPlanMarkdown: true,
  }]);
  assert.equal(status, 17);
  assert.equal(output, "native finish result\n");
});

test("capture and commit use the same native capture and presentation APIs", () => {
  for (const prefix of [["snapshot", "create"], ["commit"]]) {
    let received;
    let output = "";
    runTerminal([...prefix, "LNT-0007/C-01", "--resume", "SNP-EXACT"], {
      cwd: "/repository",
      runtime: {
        snapshotCapture(...parameters) { received = parameters; return { native_capture: true }; },
        formatSnapshotCapture: () => "native capture result",
      },
      write: (text) => { output += text; },
    });
    assert.deepEqual(received, ["/repository", "LNT-0007/C-01", {
      message: undefined, resumeSnapshotId: "SNP-EXACT",
    }]);
    assert.equal(output, "native capture result\n");
  }
});

test("JSON output uses the native codec and does not coerce large integer values", () => {
  const payload = { value: 9007199254740993n };
  let output = "";
  runTerminal(["status", "--json"], {
    runtime: {
      taskWorkflowStatus: () => payload,
      projectStatus(value, full) { assert.equal(value, payload); assert.equal(full, false); return payload; },
      encodeJson(value, pretty) {
        assert.equal(value, payload);
        assert.equal(value.value, 9007199254740993n);
        assert.equal(pretty, true);
        return '{"value":9007199254740993}';
      },
    },
    write: (text) => { output += text; },
  });
  assert.equal(output, '{"value":9007199254740993}\n');
});

test("post-save setup failures are projected after the native payload", () => {
  const events = [];
  const status = runTerminal(["config", "set", "--agent-teams", "enabled", "--json"], {
    runtime: {
      configureRepository: () => ({ saved: true }),
      configurationError: () => "native team setup failure",
    },
    write: () => { events.push("payload"); },
    writeError: (text) => { events.push(text); },
  });
  assert.equal(status, 1);
  assert.deepEqual(events, ["payload", "Error: native team setup failure\n"]);
});

test("status text uses the native renderer rather than pretty JSON", () => {
  const payload = { native_status: true };
  let output = "";
  runTerminal(["status"], {
    runtime: {
      taskWorkflowStatus: () => payload,
      formatStatus(value) { assert.equal(value, payload); return "native status\nnext action"; },
    },
    write: (text) => { output += text; },
  });
  assert.equal(output, "native status\nnext action\n");
});

test("terminal domain errors use native normalization and exit status", () => {
  let output = "";
  let received;
  const error = Object.assign(new Error("original native error"), { code: "GenericFailure" });
  const status = reportTerminalError(error, {
    runtime: {
      formatDomainError(message) { received = message; return "Error: normalized public reference"; },
      domainErrorExitCode: () => 19,
    },
    writeError: (text) => { output += text; },
  });
  assert.equal(received, "original native error");
  assert.equal(output, "Error: normalized public reference\n");
  assert.equal(status, 19);
});

test("an unavailable addon can be reported without calling that addon", () => {
  let output = "";
  assert.equal(reportTerminalError(new NativeResolutionError("missing addon"), {
    runtime: {}, writeError: (text) => { output += text; },
  }), 1);
  assert.equal(output, "ait: missing addon\n");
});

test("Plan queries pass selectors and revisions without synthesizing storage context", () => {
  let received;
  runTerminal(["plan", "show", "artifact:docs/work plan.md", "--revision", "revision-number:2", "--remote", "secondary", "--json"], {
    cwd: "/repository",
    runtime: { planShow(...parameters) { received = parameters; return {}; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/repository", "artifact:docs/work plan.md", {
    local: undefined, remoteName: "secondary", revision: "revision-number:2",
  }]);
});

test("Plan candidates retain native comma-delimited search and all semantics", () => {
  let received;
  runTerminal(["plan", "candidates", "--all", "--contains", "Title,exact/ref", "--local", "--json"], {
    cwd: "/repository",
    runtime: { planCandidates(...parameters) { received = parameters; return {}; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/repository", {
    local: true, remoteName: undefined, includeAll: true, contains: "Title,exact/ref",
  }]);
  for (const command of [["plan", "list"], ["plan", "revisions", "PR-7"]]) {
    assert.throws(() => runTerminal([...command, "--all", "--json"]),
      (error) => terminalExitCode(error) === 2);
  }
});

test("Plan sync delegates native authority and emits partial results before errors", () => {
  let received;
  const events = [];
  const payload = { partial_success: true };
  const status = runTerminal(["plan", "sync", "docs/work.md", "--plan-ref", "exact/plan", "--prune", "--reconcile", "--local", "--json"], {
    cwd: "/repository",
    runtime: {
      planSync(...parameters) { received = parameters; return payload; },
      planSyncError(value) { assert.equal(value, payload); return "native partial sync error"; },
    },
    write: () => { events.push("payload"); },
    writeError: (text) => { events.push(text); },
  });
  assert.deepEqual(received, ["/repository", "docs/work.md", {
    local: true, remoteName: undefined, planRef: "exact/plan", prune: true, reconcile: true,
  }]);
  assert.equal(status, 1);
  assert.deepEqual(events, ["payload", "Error: native partial sync error\n"]);
});

test("workflow phases retain authority before mutation and use native phase status", () => {
  const events = [];
  const payload = { closeout_pending: true };
  let request;
  const status = runTerminal(["workflow", "ready", "LNT-7", "--apply", "--remote", "secondary", "--expected-identity", "/pins/exact.json", "--snapshot-message", "capture", "--json", "--full"], {
    cwd: "/task/worktree",
    runtime: {
      repositoryAuthorityRoot(root) { events.push("authority"); assert.equal(root, "/task/worktree"); return "/canonical/repository"; },
      workflowReady(...parameters) { events.push("operation"); request = parameters; return { payload, exitCode: 75 }; },
      projectWorkflowReady(root, value, taskId, options) {
        events.push("projection");
        assert.equal(root, "/canonical/repository");
        assert.equal(value, payload);
        assert.equal(taskId, "LNT-7");
        assert.deepEqual(options, { remoteName: "secondary", full: true });
        return { native_projection: true };
      },
    },
    write: () => {},
  });
  assert.equal(request[0], "/task/worktree");
  assert.equal(request[1], "LNT-7");
  assert.equal(request[2].expectedIdentityPath, "/pins/exact.json");
  assert.equal(request[2].snapshotMessage, "capture");
  assert.equal(request[2].apply, true);
  assert.equal(status, 75);
  assert.deepEqual(events, ["authority", "operation", "projection"]);
});

test("workflow syntax requires apply and keeps reconciliation policy native", () => {
  for (const args of [
    ["workflow", "ready", "LNT-7", "--snapshot-message", "capture"],
    ["workflow", "finish", "LNT-7", "--reviewer", "reviewer"],
    ["workflow", "reconcile", "--scheduled"],
    ["workflow", "reconcile", "--apply", "--dry-run"],
    ["workflow", "reconcile", "--limit", "18446744073709551616"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  let received;
  runTerminal(["workflow", "reconcile", "--apply", "--scheduled", "--task", "LNT-7", "--limit", "3", "--json"], {
    cwd: "/repository",
    runtime: { workflowReconcile(...parameters) { received = parameters; return {}; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/repository", {
    remoteName: undefined, taskId: "LNT-7", apply: true,
    safeOnly: undefined, scheduled: true, limit: 3,
  }]);
});

test("Plan text and config changes use native renderers with original selections", () => {
  const events = [];
  const payload = [{ native_plan: true }];
  runTerminal(["plan", "list", "--all", "--local"], {
    runtime: {
      planList: () => payload,
      formatPlanList(value, options) {
        assert.equal(value, payload);
        assert.deepEqual(options, { local: true, remoteName: undefined, all: true });
        return "native complete Plan list";
      },
    },
    write: (text) => { events.push(text); },
  });
  let original;
  runTerminal(["config", "set", "--plan-language", "ja-JP"], {
    runtime: {
      configureRepository(_root, request) { original = request; return { saved: true }; },
      formatConfiguration(value, request) {
        assert.deepEqual(value, { saved: true });
        assert.equal(request, original);
        return "native updated configuration fields";
      },
    },
    write: (text) => { events.push(text); },
  });
  assert.deepEqual(events, ["native complete Plan list\n", "native updated configuration fields\n"]);
});

test("signed 64-bit CLI configuration is passed as BigInt without rounding", () => {
  let received;
  runTerminal(["config", "set", "--task-worktree-main-seed-ram-max-bytes", "9007199254740993", "--json"], {
    runtime: { configureRepository(_root, request) { received = request; return {}; } },
    write: () => {},
  });
  assert.equal(received.task_worktree_main_seed_ram_max_bytes, 9007199254740993n);
  assert.throws(() => runTerminal(["config", "set", "--task-worktree-main-seed-ram-max-bytes", "9223372036854775808", "--json"]),
    (error) => terminalExitCode(error) === 2);
});

test("Quick and abandonment retain their native text and outcome semantics", () => {
  let received;
  let output = "";
  runTerminal(["task", "quick", "--intent", "repair"], {
    runtime: {
      taskQuick(...parameters) { received = parameters; return {}; },
      formatTaskQuick: () => "native quick\ncard: native/card.md",
    },
    write: (text) => { output += text; },
  });
  assert.equal(received[1].title, undefined);
  assert.equal("source" in received[1], false);
  assert.equal(output, "native quick\ncard: native/card.md\n");
  const status = runTerminal(["task", "abandon", "@/pins/package.json", "--remote", "secondary", "--json"], {
    runtime: {
      taskAbandon(_root, reference, options) {
        assert.equal(reference, "@/pins/package.json");
        assert.deepEqual(options, { local: undefined, remoteName: "secondary" });
        return { payload: { native_terminal: true }, exitCode: 75 };
      },
    },
    write: () => {},
  });
  assert.equal(status, 75);
});

test("Snapshot DAG options preserve exact native-size values and Tag selectors", () => {
  let received;
  runTerminal(["snapshot", "ancestry", "release-tag", "--descendants", "--first-parent", "--max-depth", "9007199254740993", "--limit", "7", "--json"], {
    cwd: "/repository",
    runtime: { snapshotAncestry(...parameters) { received = parameters; return {}; } },
    write: () => {},
  });
  assert.deepEqual(received, ["/repository", "release-tag", {
    descendants: true, firstParent: true, maxDepth: 9007199254740993n, limit: 7,
  }]);
  for (const args of [
    ["snapshot", "ancestry", "SNP-X", "--ancestors", "--descendants"],
    ["snapshot", "ancestry", "SNP-X", "--limit", "0"],
    ["snapshot", "ancestry", "SNP-X", "--max-depth", "18446744073709551616"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
});

test("Snapshot DAG false outcomes and lookup errors retain distinct native statuses", () => {
  let output = "";
  const status = runTerminal(["snapshot", "merge-base", "left-tag", "right-tag"], {
    runtime: {
      snapshotMergeBase: () => ({ payload: { no_base: true }, exitCode: 1 }),
      formatSnapshotMergeBase: () => "",
    },
    write: (text) => { output += text; },
  });
  assert.equal(status, 1);
  assert.equal(output, "");
  let errorOutput = "";
  const error = new Error("native query lookup failure");
  assert.equal(runTerminal(["snapshot", "is-ancestor", "left-tag", "right-tag"], {
    runtime: {
      snapshotIsAncestor: () => { throw error; },
      formatSnapshotQueryError(message) { assert.equal(message, error.message); return "native query error"; },
      snapshotQueryErrorExitCode: () => 2,
    },
    writeError: (text) => { errorOutput += text; },
  }), 2);
  assert.equal(errorOutput, "native query error\n");
});

test("Task list fetches raw totals before delegating the user's presentation choice", () => {
  const raw = [{ task_id: "active" }, { task_id: "terminal" }];
  let output = "";
  runTerminal(["task", "list", "--remote", "secondary", "--json"], {
    runtime: {
      taskWorkflowTaskList(_root, options) {
        assert.deepEqual(options, { local: undefined, remoteName: "secondary", all: true });
        return raw;
      },
      projectTaskList(value, all) { assert.equal(value, raw); assert.equal(all, false); return { native_filtered: true }; },
    },
    write: (text) => { output += text; },
  });
  assert.deepEqual(JSON.parse(output), { native_filtered: true });
  runTerminal(["task", "list", "--all", "--local"], {
    runtime: {
      taskWorkflowTaskList: () => raw,
      formatTaskList(value, options) {
        assert.equal(value, raw);
        assert.deepEqual(options, { local: true, remoteName: undefined, all: true });
        return "native counts and columns";
      },
    },
    write: () => {},
  });
});

test("Snapshot show requests parent evidence only for text and emits only payload JSON", () => {
  const inspection = { payload: { snapshot_id: "SNP-X" }, delta: { native_parent_evidence: true } };
  let output = "";
  runTerminal(["snapshot", "show", "release-tag", "--json"], {
    runtime: {
      inspectSnapshot(_root, selector, includeDelta) {
        assert.equal(selector, "release-tag");
        assert.equal(includeDelta, false);
        return inspection;
      },
    },
    write: (text) => { output += text; },
  });
  assert.deepEqual(JSON.parse(output), inspection.payload);
  runTerminal(["snapshot", "show", "release-tag", "--files"], {
    runtime: {
      inspectSnapshot(_root, selector, includeDelta) {
        assert.equal(selector, "release-tag");
        assert.equal(includeDelta, true);
        return inspection;
      },
      formatSnapshotShow(value, files) { assert.equal(value, inspection); assert.equal(files, true); return "native files and evidence"; },
    },
    write: () => {},
  });
});

test("Snapshot inventory bounds and ordering stay in native projection", () => {
  const payload = [{ snapshot_id: "native-order" }];
  runTerminal(["snapshot", "list", "--all", "--json"], {
    runtime: {
      taskWorkflowSnapshotList: () => payload,
      projectSnapshotList(value, all) { assert.equal(value, payload); assert.equal(all, true); return value; },
    },
    write: () => {},
  });
});

test("Line inventory keeps native current-Line context for text and only payload for JSON", () => {
  const inventory = { payload: [{ line_name: "main" }], current_line: "main" };
  runTerminal(["branch", "list", "--archived", "--remote", "secondary"], {
    cwd: "/repository",
    runtime: {
      lineInventory(root, options) {
        assert.equal(root, "/repository");
        assert.deepEqual(options, { includeAll: undefined, archived: true, remoteName: "secondary" });
        return inventory;
      },
      formatLineList(value, options) {
        assert.equal(value, inventory);
        assert.equal(options.archived, true);
        return "native current-Line marker and inventory";
      },
    },
    write: () => {},
  });
  let output = "";
  runTerminal(["line", "list", "--json"], {
    runtime: { lineInventory: () => inventory },
    write: (text) => { output += text; },
  });
  assert.deepEqual(JSON.parse(output), inventory.payload);
});

test("Line mutation and remote selectors become concrete domain parameters", () => {
  const cases = [
    [["create", "feature", "--from-snapshot", "release-tag", "--switch"], "taskWorkflowLineCreate",
      ["feature", { fromSnapshot: "release-tag", switch: true }]],
    [["switch", "feature", "--restore", "--force"], "taskWorkflowLineSwitch",
      ["feature", { restore: true, force: true }]],
    [["show", "--remote", "secondary"], "lineShow", [undefined, "secondary"]],
    [["archive", "feature", "--remote", "secondary"], "taskWorkflowLineArchive", ["feature", "secondary"]],
    [["rename", "old", "new", "--remote", "secondary"], "taskWorkflowLineRename", ["old", "new", "secondary"]],
    [["delete", "feature", "--yes", "--remote", "secondary"], "taskWorkflowLineDelete",
      ["feature", { remoteName: "secondary", confirm: true }]],
    [["merge", "feature", "--message", "merge text"], "lineMerge",
      [{ source: "feature", message: "merge text", continueMerge: undefined, abortMerge: undefined }]],
    [["merge", "--continue", "--message", "resolved"], "lineMerge",
      [{ source: undefined, message: "resolved", continueMerge: true, abortMerge: undefined }]],
    [["merge", "--abort"], "lineMerge",
      [{ source: undefined, message: undefined, continueMerge: undefined, abortMerge: true }]],
  ];
  for (const [args, method, expected] of cases) {
    let called = false;
    runTerminal(["line", ...args, "--json"], {
      cwd: "/repository",
      runtime: { [method](...parameters) {
        called = true;
        assert.deepEqual(parameters, ["/repository", ...expected]);
        return {};
      } },
      write: () => {},
    });
    assert.equal(called, true, method);
  }
});

test("Line parser guards fail before reaching Rust mutations", () => {
  for (const args of [
    ["list", "--all", "--archived"],
    ["switch", "feature", "--force"],
    ["delete", "feature"],
    ["merge", "feature", "--continue"],
    ["merge", "feature", "--abort"],
    ["merge", "--continue", "--abort"],
    ["merge", "--abort", "--message", "message"],
    ["cleanup", "--limit", "18446744073709551616"],
  ]) assert.throws(() => runTerminal(["line", ...args]), (error) => terminalExitCode(error) === 2);
});

test("Line cleanup preserves the full native-size selection and native expansion text", () => {
  const payload = { native_protection_groups: true };
  runTerminal(["line", "cleanup", "--kind", "review", "--limit", "9007199254740993", "--include-protected", "--all", "--yes"], {
    cwd: "/repository",
    runtime: {
      taskWorkflowLineCleanup(root, options) {
        assert.equal(root, "/repository");
        assert.deepEqual(options, {
          idleFor: "7d", cleanupKind: "review", limit: 9007199254740993n,
          includeProtected: true, apply: true,
        });
        return payload;
      },
      formatLineCleanup(value, idleFor, options) {
        assert.equal(value, payload);
        assert.equal(idleFor, "7d");
        assert.deepEqual(options, {
          cleanupKind: "review", limit: 9007199254740993n, includeProtected: true, all: true,
        });
        return "native protection and expansion text";
      },
    },
    write: () => {},
  });
  runTerminal(["line", "cleanup", "--idle-for", "12h", "--limit", "0", "--json"], {
    runtime: { taskWorkflowLineCleanup(_root, options) {
      // Clap accepts usize zero; the Rust service owns its validation.
      assert.equal(options.limit, 0);
      assert.equal(options.idleFor, "12h");
      return payload;
    } },
    write: () => {},
  });
});

test("frozen configuration and provenance enums are terminal syntax errors", () => {
  for (const args of [
    ["config", "set", "--agent-teams", "yes"],
    ["config", "set", "--plan-style", "verbose"],
    ["config", "set", "--sprint", "enabled"],
    ["config", "set", "--default-author-mode", "ai"],
    ["config", "set", "--task-review", "optional"],
    ["config", "unset", "agent-teams"],
    ["config", "unset", "sprint"],
    ["workflow", "ready", "LNT-7", "--apply", "--author-mode", "ai"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  let selected;
  runTerminal(["config", "unset", "task-worktree-main-seed-ram-max-bytes", "--json"], {
    runtime: { unsetConfiguration(_root, key) { selected = key; return {}; } },
    write: () => {},
  });
  assert.equal(selected, "task-worktree-main-seed-ram-max-bytes");
});

test("Task progress is enabled only for terminal stdout with JSON disabled", () => {
  for (const command of ["start", "quick"]) {
    for (const stdoutIsTTY of [false, true]) {
      for (const json of [false, true]) {
        let received;
        const method = command === "start" ? "taskStart" : "taskQuick";
        runTerminal(["task", command, "--title", "work", "--intent", "repair", ...(json ? ["--json"] : [])], {
          stdoutIsTTY,
          runtime: {
            [method](_root, request) { received = request; return {}; },
            projectTaskStart: (value) => value,
            formatTaskStart: () => "native start",
            formatTaskQuick: () => "native quick",
          },
          write: () => {},
        });
        assert.equal(received.humanProgress, stdoutIsTTY && !json);
        assert.ok(Object.values(received).every((value) => typeof value !== "function"));
      }
    }
  }
  assert.throws(() => runTerminal(["task", "quick", "--intent", "repair", "--human-progress"]),
    (error) => terminalExitCode(error) === 2);
});

test("repository handlers select concrete operations and native text with the original remote", () => {
  const cases = [
    ["show", "repositoryShow", "formatRepositoryShow", ["secondary"], ["secondary"]],
    ["ci-capabilities", "repositoryCiCapabilities", "formatRepositoryCiCapabilities", ["secondary"], ["secondary"]],
    ["retire", "repositoryRetire", "formatRepositoryRetire", ["secondary", undefined], []],
    ["restore", "repositoryRestore", "formatRepositoryRestore", ["secondary"], []],
  ];
  for (const [command, method, formatter, argumentsAfterRoot, argumentsAfterPayload] of cases) {
    const payload = { native_repository: command };
    let output = "";
    runTerminal(["repo", command, "--remote", "secondary"], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...argumentsAfterRoot]); return payload; },
        [formatter](...parameters) { assert.deepEqual(parameters, [payload, ...argumentsAfterPayload]); return "native repository text"; },
      },
      write: (text) => { output += text; },
    });
    assert.equal(output, "native repository text\n");
  }
  runTerminal(["repo", "retire", "--abort", "--json"], {
    runtime: { repositoryRetire(_root, remote, abort) {
      assert.equal(remote, undefined);
      assert.equal(abort, true);
      return {};
    } },
    write: () => {},
  });
});

test("repository Job selection preserves native defaults and exact unsigned indexes", () => {
  for (const [args, expected] of [
    [[], { remoteName: undefined, workerJobIndex: undefined, state: undefined, limit: undefined }],
    [["--worker-job-index", "4294967295"], { remoteName: undefined, workerJobIndex: 4294967295, state: undefined, limit: undefined }],
    [["--remote", "secondary", "--state", "queued", "--limit", "1000"],
      { remoteName: "secondary", workerJobIndex: undefined, state: "queued", limit: 1000 }],
  ]) {
    let originalSelection;
    runTerminal(["repo", "jobs", ...args], {
      runtime: {
        repositoryJobs(_root, selection) { originalSelection = selection; assert.deepEqual(selection, expected); return {}; },
        formatRepositoryJobs(_payload, selection) { assert.equal(selection, originalSelection); return "native Job inventory"; },
      },
      write: () => {},
    });
  }
});

test("repository Job syntax rejects conflicts, invalid state and invalid bounds before native calls", () => {
  for (const args of [
    ["--worker-job-index", "1", "--state", "running"],
    ["--worker-job-index", "1", "--limit", "50"],
    ["--worker-job-index", "4294967296"],
    ["--worker-job-index=-1"],
    ["--state", "pending"],
    ["--limit", "0"],
    ["--limit", "1001"],
    ["--limit", "4294967296"],
    ["--limit", "1.5"],
  ]) assert.throws(() => runTerminal(["repo", "jobs", ...args]), (error) => terminalExitCode(error) === 2);
});

test("Change commands use native reference selection and workspace operations", () => {
  const cases = [
    [["create", "LNT-7", "--title", "Repair review", "--base-line", "stable", "--fork-snapshot", "SNP-EXACT", "--local"],
      "changeCreate", ["LNT-7", "Repair review", { local: true, remoteName: undefined, baseLine: "stable", forkSnapshot: "SNP-EXACT" }], "formatChangeCreate"],
    [["show", "LNT-7", "--remote", "secondary"], "changeShow",
      ["LNT-7", { local: undefined, remoteName: "secondary" }], "formatChangeShow"],
    [["revert", "LNT-7/C-01", "--force", "--dry-run", "--local"], "changeRevert",
      ["LNT-7/C-01", { local: true, remoteName: undefined, force: true, dryRun: true }], "formatChangeRevert"],
    [["replay", "LNT-7", "--local"], "changeReplay",
      ["LNT-7", undefined, { local: true, remoteName: undefined, force: undefined, dryRun: undefined }], "formatChangeReplay"],
    [["replay", "LNT-7/C-01", "--onto", "stable", "--force", "--remote", "secondary"], "changeReplay",
      ["LNT-7/C-01", "stable", { local: undefined, remoteName: "secondary", force: true, dryRun: undefined }], "formatChangeReplay"],
    [["close", "LNT-7", "--local"], "changeClose", ["LNT-7", { local: true, remoteName: undefined }], "formatChangeClose"],
    [["publish", "LNT-7", "--remote", "secondary"], "changePublish", ["LNT-7", "secondary"], "formatChangePublish"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_result: method };
    let output = "";
    runTerminal(["change", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return `native ${method}`; },
      },
      write: (text) => { output += text; },
    });
    assert.equal(output, `native ${method}\n`);
  }
});

test("Change inventory passes the complete raw list to native presentation", () => {
  const raw = [{ status: "active" }, { status: "closed" }];
  for (const json of [false, true]) {
    runTerminal(["change", "list", "--all", "--remote", "secondary", ...(json ? ["--json"] : [])], {
      runtime: {
        taskWorkflowChangeList(_root, options) {
          assert.deepEqual(options, { local: undefined, remoteName: "secondary" });
          return raw;
        },
        projectChangeList(payload, all) { assert.equal(payload, raw); assert.equal(all, true); return raw; },
        formatChangeList(payload, options) {
          assert.equal(payload, raw);
          assert.deepEqual(options, { local: undefined, remoteName: "secondary", all: true });
          return "native inventory";
        },
      },
      write: () => {},
    });
  }
});

test("Patchset publication uses the guarded operation and retained queries use native formatting", () => {
  const cases = [
    [["publish", "\u0085 LNT-7 \u0085", "--summary", "review summary", "--author-mode", "ai_with_human_review"],
      "patchsetPublish", ["LNT-7", "review summary", { authorMode: "ai_with_human_review", remoteName: "secondary" }], "formatPatchsetPublish"],
    [["list", " LT-0001-C01 "], "patchsetList", ["LT-0001-C01", "secondary"], "formatPatchsetList"],
    [["show", " LNT-7/P-01 "], "taskWorkflowPatchsetShow", ["LNT-7/P-01", { remoteName: "secondary" }], "formatPatchsetShow"],
    [["select", "LNT-7/P-01"], "taskWorkflowPatchsetSelect", ["LNT-7/P-01", { remoteName: "secondary" }], "formatPatchsetSelect"],
    [["ci-status", "LNT-7/P-01"], "taskWorkflowPatchsetCiStatus", ["LNT-7/P-01", { remoteName: "secondary" }], "formatPatchsetCiStatus"],
    [["rerun-ci", "LNT-7/P-01"], "taskWorkflowPatchsetRerunCi", ["LNT-7/P-01", { remoteName: "secondary" }], "formatPatchsetRerunCi"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_result: method };
    runTerminal(["patchset", ...args, "--remote", "secondary"], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native Patchset text"; },
      },
      write: () => {},
    });
  }
});

test("Change and Patchset parser guards retain required values and syntax status", () => {
  for (const args of [
    ["change", "create", "LNT-7"],
    ["change", "show", "LNT-7", "--local", "--remote", "secondary"],
    ["patchset", "publish", "LNT-7"],
    ["patchset", "publish", "LNT-7", "--summary", "text", "--author-mode", "ai"],
    ["patchset", "list", " 123 "],
    ["patchset", "list", "\u0085"],
    ...["show", "select", "ci-status", "rerun-ci"].flatMap((command) => [
      ["patchset", command, "123"], ["patchset", command, " "],
    ]),
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  runTerminal(["patchset", "list", "\ufeff123", "--json"], {
    runtime: { patchsetList(_root, reference) {
      // Rust trim does not strip a byte-order mark. Identity validation is native.
      assert.equal(reference, "\ufeff123");
      return [];
    } },
    write: () => {},
  });
});

test("Snapshot CLI uses native selector resolution, defaults, byte bounds and text", () => {
  const cases = [
    [["diff", "old-tag", "new-tag", "--include-text", "--max-bytes", "9007199254740993"],
      "snapshotDiff", ["old-tag", "new-tag", { includeText: true, maxBytes: 9007199254740993n }], "formatSnapshotDiff"],
    [["diff", "old-tag", "new-tag"], "snapshotDiff",
      ["old-tag", "new-tag", { includeText: undefined, maxBytes: undefined }], "formatSnapshotDiff"],
    [["revert", "current-tag", "--force", "--dry-run"], "snapshotRevert",
      ["current-tag", { force: true, dryRun: true }], "formatSnapshotRevert"],
    [["replay", "source-tag", "--dry-run"], "snapshotReplay",
      ["source-tag", undefined, { force: undefined, dryRun: true }], "formatSnapshotReplay"],
    [["replay", "source-tag", "--onto", "stable", "--force"], "snapshotReplay",
      ["source-tag", "stable", { force: true, dryRun: undefined }], "formatSnapshotReplay"],
    [["restore-lines", "SNP-EXACT", "path with spaces.txt", "--line", "9007199254740993", "--yes"],
      "taskWorkflowSnapshotRestoreLines", ["SNP-EXACT", "path with spaces.txt", {
        line: 9007199254740993n, startLine: undefined, endLine: undefined, yes: true,
      }], "formatSnapshotRestoreLines"],
    [["restore-lines", "SNP-EXACT", "file.txt", "--start", "9", "--end", "3"],
      "taskWorkflowSnapshotRestoreLines", ["SNP-EXACT", "file.txt", {
        line: undefined, startLine: 9, endLine: 3, yes: undefined,
      }], "formatSnapshotRestoreLines"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_snapshot_result: true };
    runTerminal(["snapshot", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native Snapshot text"; },
      },
      write: () => {},
    });
  }
});

test("Snapshot mutation syntax is checked before native calls", () => {
  for (const args of [
    ["diff", "old", "new", "--max-bytes", "1"],
    ["diff", "old", "new", "--include-text", "--max-bytes", "0"],
    ["diff", "old", "new", "--include-text", "--max-bytes", "18446744073709551616"],
    ["restore-lines", "SNP-X", "file.txt", "--line", "1", "--start", "1", "--end", "2"],
    ["restore-lines", "SNP-X", "file.txt", "--start", "1"],
    ["restore-lines", "SNP-X", "file.txt", "--end", "2"],
    ["restore-lines", "SNP-X", "file.txt", "--line", "0"],
  ]) assert.throws(() => runTerminal(["snapshot", ...args]), (error) => terminalExitCode(error) === 2);
});

test("Snapshot diff lookup failures use the ordinary domain error status", () => {
  const nativeError = new Error("native Snapshot diff lookup failure");
  assert.throws(() => runTerminal(["snapshot", "diff", "old", "new"], {
    runtime: { snapshotDiff() { throw nativeError; } },
  }), (error) => error === nativeError);
  let output = "";
  assert.equal(reportTerminalError(nativeError, {
    runtime: { formatDomainError: (message) => `Error: ${message}`, domainErrorExitCode: () => 1 },
    writeError: (text) => { output += text; },
  }), 1);
  assert.equal(output, "Error: native Snapshot diff lookup failure\n");
});

test("Auth operations retain native identity fallback, repository selection and text", () => {
  for (const [command, method, formatter] of [
    ["whoami", "taskWorkflowAuthWhoami", "formatAuthWhoami"],
    ["bindings", "taskWorkflowAuthBindings", "formatAuthBindings"],
  ]) {
    const payload = { native_auth_result: true };
    runTerminal(["auth", command, "--remote", "secondary", "--repo", "7"], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) {
          assert.deepEqual(parameters, ["/repository", { remoteName: "secondary", repoName: "7" }]);
          return payload;
        },
        [formatter](value) { assert.equal(value, payload); return "native Auth text"; },
      },
      write: () => {},
    });
  }
  runTerminal(["auth", "whoami", "--json"], {
    runtime: { taskWorkflowAuthWhoami(_root, options) {
      assert.deepEqual(options, { remoteName: undefined, repoName: undefined });
      return {};
    } },
    write: () => {},
  });
});

test("Auth grants preserve repeated roles as an array without frontend validation or splitting", () => {
  runTerminal(["auth", "grant", "--actor", "reviewer@example.test", "--role", "code_review", "--role", "task_approve,custom", "--remote", "secondary", "--json"], {
    cwd: "/repository",
    runtime: { taskWorkflowAuthGrant(...parameters) {
      assert.deepEqual(parameters, ["/repository", "reviewer@example.test", ["code_review", "task_approve,custom"], {
        remoteName: "secondary", repoName: undefined,
      }]);
      return {};
    } },
    write: () => {},
  });
  runTerminal(["auth", "grant", "--actor", "reviewer@example.test"], {
    runtime: {
      taskWorkflowAuthGrant(_root, _actor, roles) { assert.deepEqual(roles, []); return {}; },
      formatAuthGrant: () => "native grant text",
    },
    write: () => {},
  });
  for (const args of [
    ["auth", "grant", "--role", "code_review"],
    ["auth", "grant", "--actor", "one", "--actor", "two"],
    ["auth", "whoami", "--remote", "one", "--remote", "two"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
});

test("hidden Auth commands stay callable without appearing in root help", () => {
  let rootHelp = "";
  runTerminal(["--help"], { write: (text) => { rootHelp += text; } });
  assert.doesNotMatch(rootHelp, /auth whoami|auth bindings|auth grant/);
  let authHelp = "";
  runTerminal(["auth", "--help"], { write: (text) => { authHelp += text; } });
  assert.match(authHelp, /^  whoami\s*$/m);
});

test("Stash commands use typed native operations and preserve all option defaults", () => {
  const cases = [
    [["save"], "stashSave", [{ message: undefined, keepWorkspace: undefined }], "formatStashSave"],
    [["save", "--message", "Saved work", "--keep-workspace"], "stashSave",
      [{ message: "Saved work", keepWorkspace: true }], "formatStashSave"],
    [["list"], "taskWorkflowStashList", [], "formatStashList"],
    [["show", "stash-exact"], "taskWorkflowStashShow", ["stash-exact"], "formatStashShow"],
    [["apply", "stash-exact"], "stashApply", ["stash-exact", undefined], "formatStashApply"],
    [["apply", "stash-exact", "--force"], "stashApply", ["stash-exact", true], "formatStashApply"],
    [["pop", "stash-exact", "--force"], "stashPop", ["stash-exact", true], "formatStashPop"],
    [["drop", "stash-exact"], "stashDrop", ["stash-exact"], "formatStashDrop"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_stash_result: true };
    runTerminal(["stash", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native Stash text"; },
      },
      write: () => {},
    });
  }
  for (const args of [
    ["stash", "show"], ["stash", "apply"], ["stash", "save", "--force"],
    ["stash", "save", "-m", "text"], ["stash", "list", "--remote", "secondary"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
});

test("native Stash conflict guidance crosses the frontend unchanged", () => {
  const error = new Error("Workspace has unsaved changes. Re-run with --force to replace those workspace changes.");
  assert.throws(() => runTerminal(["stash", "apply", "stash-exact", "--json"], {
    runtime: { stashApply() { throw error; } },
  }), (actual) => actual === error);
  let output = "";
  assert.equal(reportTerminalError(error, {
    runtime: { formatDomainError: (message) => `Error: ${message}`, domainErrorExitCode: () => 1 },
    writeError: (text) => { output += text; },
  }), 1);
  assert.equal(output, `Error: ${error.message}\n`);
});

test("Tag commands preserve native head selection, identity and formatting", () => {
  const cases = [
    [["create", "v1", "--message", "Release message"], "tagCreate",
      ["v1", "Release message", undefined], "formatTagCreate"],
    [["create", "v1", "--message", "Release message", "--snapshot", "SNP-EXACT"], "tagCreate",
      ["v1", "Release message", "SNP-EXACT"], "formatTagCreate"],
    [["list"], "tagList", [], "formatTagList"],
    [["show", "v1"], "tagShow", ["v1"], "formatTagShow"],
    [["delete", "v1"], "tagDelete", ["v1"], "formatTagDelete"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_tag_result: true };
    runTerminal(["tag", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native Tag text"; },
      },
      write: () => {},
    });
  }
  for (const args of [
    ["tag", "create", "v1"],
    ["tag", "create", "v1", "--message", "text", "--force"],
    ["tag", "delete", "v1", "--force"],
    ["tag", "show"],
    ["tag", "list", "--remote", "secondary"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
});

test("nested Team review commands preserve reviewer groups, exact options and native text", () => {
  const cases = [
    [["request", "LNT-7", "--group", "backend", "--group", "security", "--patchset", " LNT-7/P-01 ", "--note", "Review note"],
      "reviewTeamRequest", ["LNT-7", ["backend", "security"], { patchsetId: "LNT-7/P-01", note: "Review note", remoteName: "secondary" }], "formatReviewTeamRequest"],
    ...[
      ["approve", "reviewTeamApprove", "formatReviewTeamApprove"],
      ["request-changes", "reviewTeamRequestChanges", "formatReviewTeamRequestChanges"],
      ["comment", "reviewTeamComment", "formatReviewTeamComment"],
      ["defer", "reviewTeamDefer", "formatReviewTeamDefer"],
    ].map(([command, method, formatter]) => [
      [command, "LNT-7", "--patchset", "LNT-7/P-01", "--reviewer", "Independent Reviewer", "--message", "Review message"],
      method, ["LNT-7", {
        patchsetId: "LNT-7/P-01", reviewer: "Independent Reviewer", message: "Review message", remoteName: "secondary",
      }], formatter,
    ]),
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { full_native_review_evidence: true };
    runTerminal(["review", "team", ...args, "--remote", "secondary"], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native Team review"; },
      },
      write: () => {},
    });
  }
});

test("Task review forwards selectors to Rust and preserves full operation envelopes", () => {
  const cases = [
    ["approve", "reviewTaskApprove", "formatReviewTaskApprove"],
    ["request-changes", "reviewTaskRequestChanges", "formatReviewTaskRequestChanges"],
    ["comment", "reviewTaskComment", "formatReviewTaskComment"],
    ["defer", "reviewTaskDefer", "formatReviewTaskDefer"],
  ];
  for (const [command, method, formatter] of cases) {
    for (const packageReview of [false, true]) {
      for (const json of [false, true]) {
        const selector = packageReview ? "@/absolute/unread identity.json" : " LNT-7/P-01 ";
        const normalized = packageReview ? selector : "LNT-7/P-01";
        const operation = { payload: { native_review: true }, package_review: packageReview };
        let output = "";
        assert.equal(runTerminal(["review", "task", command, "LNT-7", "--patchset", selector, "--message", "Review message", "--remote", "secondary", ...(json ? ["--json"] : [])], {
          cwd: "/repository",
          runtime: {
            [method](...parameters) {
              assert.deepEqual(parameters, command === "approve"
                ? ["/repository", "LNT-7", normalized, "Review message", "secondary"]
                : ["/repository", "LNT-7", { patchsetId: normalized, message: "Review message", remoteName: "secondary" }]);
              return operation;
            },
            projectReviewTaskOperation(value) { assert.equal(value, operation); return { native_json_projection: true }; },
            [formatter](value) { assert.equal(value, operation); return "native ordinary or package title"; },
          },
          write: (text) => { output += text; },
        }), 0);
        if (json) assert.deepEqual(JSON.parse(output), { native_json_projection: true });
        else assert.equal(output, "native ordinary or package title\n");
      }
    }
  }
  runTerminal(["review", "task", "defer", "LNT-7", "--json"], {
    runtime: {
      reviewTaskDefer(_root, reference, options) {
        assert.equal(reference, "LNT-7");
        assert.deepEqual(options, { patchsetId: undefined, message: undefined, remoteName: undefined });
        return { payload: {}, package_review: false };
      },
      projectReviewTaskOperation: (operation) => operation.payload,
    },
    write: () => {},
  });
});

test("Review show keeps native full text and compact JSON projections distinct", () => {
  const payload = { requests: [{ full_native_evidence: true }] };
  for (const json of [false, true]) {
    let output = "";
    runTerminal(["review", "show", "LNT-7", "--remote", "secondary", ...(json ? ["--json"] : [])], {
      runtime: {
        reviewShow(_root, reference, remote) { assert.equal(reference, "LNT-7"); assert.equal(remote, "secondary"); return payload; },
        projectReviewShow(value) { assert.equal(value, payload); return { native_compact: true }; },
        formatReviewShow(value) { assert.equal(value, payload); return "native full evidence"; },
      },
      write: (text) => { output += text; },
    });
    if (json) assert.deepEqual(JSON.parse(output), { native_compact: true });
    else assert.equal(output, "native full evidence\n");
  }
});

test("Code review uses native submission and template operations", () => {
  runTerminal(["review", "code", "submit", "LNT-7", "--patchset", " LNT-7/P-01 ", "--message", "Code review", "--reviewer", "Independent Reviewer", "--remote", "secondary"], {
    cwd: "/repository",
    runtime: {
      reviewCodeSubmit(...parameters) {
        assert.deepEqual(parameters, ["/repository", "LNT-7", "LNT-7/P-01", "Code review", {
          reviewer: "Independent Reviewer", remoteName: "secondary",
        }]);
        return {};
      },
      formatReviewCodeSubmit: () => "native Code review",
    },
    write: () => {},
  });
  for (const style of [undefined, "inline", "numbered"]) {
    runTerminal(["review", "code", "template", ...(style ? ["--style", style] : [])], {
      runtime: {
        taskWorkflowReviewCodeTemplate(options) { assert.deepEqual(options, { style }); return {}; },
        formatReviewCodeTemplate: () => "native review template",
      },
      write: () => {},
    });
  }
});

test("Review scalar validation errors use parser status while operation and loader failures do not", () => {
  const failure = Object.assign(new Error("native scalar validation rejected"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["review", "task", "approve", "LNT-7", "--patchset", "@relative.json", "--message", "Review"], {
    runtime: { validateReviewPatchsetSelector() { throw failure; } },
  }), (error) => terminalExitCode(error) === 2 && error.message === failure.message);
  assert.throws(() => runTerminal(["review", "task", "comment", "LNT-7", "--patchset", "@/unread.json"], {
    runtime: { reviewTaskComment() { throw failure; } },
  }), (error) => error === failure && terminalExitCode(error) === 1);
  const unavailable = new NativeResolutionError("native addon unavailable");
  assert.throws(() => runTerminal(["patchset", "show", "LNT-7/P-01"], {
    runtime: { validatePublicPatchsetArgument() { throw unavailable; } },
  }), (error) => error === unavailable && terminalExitCode(error) === 1);
});

test("nested Review syntax retains required and repeatable flags", () => {
  for (const args of [
    ["review", "team", "request", "LNT-7"],
    ["review", "team", "approve", "LNT-7", "--blocking"],
    ["review", "task", "approve", "LNT-7", "--message", "Review"],
    ["review", "task", "approve", "LNT-7", "--patchset", "LNT-7/P-01"],
    ["review", "task", "comment", "LNT-7", "--patchset", "123"],
    ["review", "code", "submit", "LNT-7", "--message", "Review"],
    ["review", "code", "template", "--style", "unknown"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  let help = "";
  runTerminal(["help", "review", "team"], { write: (text) => { help += text; } });
  assert.match(help, /Usage: ait review team <COMMAND>/);
  assert.match(help, /^  request-changes\s*$/m);
});

test("Attestation CLI passes only its public fields to the complete native primitive", () => {
  runTerminal(["attest", "put", " LNT-7/P-01 ", "--tests", "pass", "--lint", "pending", "--security", "unknown", "--license", "fail", "--author-mode", "native-mode-value", "--model", "native-model", "--remote", "secondary"], {
    cwd: "/repository",
    runtime: {
      taskWorkflowAttestPut(...parameters) {
        assert.deepEqual(parameters, ["/repository", {
          patchsetId: "LNT-7/P-01", tests: "pass", lint: "pending", security: "unknown", license: "fail",
          authorMode: "native-mode-value", model: "native-model", remoteName: "secondary",
        }]);
        return {};
      },
      formatAttestPut: () => "native Attestation text",
    },
    write: () => {},
  });
});

test("Attestation and Policy CLI handlers use parsed identities and native text", () => {
  for (const [args, method, expected, formatter] of [
    [["attest", "show", " LNT-7/P-01 "], "taskWorkflowAttestShow", ["LNT-7/P-01", { remoteName: "secondary" }], "formatAttestShow"],
    [["policy", "eval", "LNT-7/P-01"], "taskWorkflowPolicyEval", ["LNT-7/P-01", { remoteName: "secondary" }], "formatPolicyEval"],
    [["policy", "waive", "LNT-7/P-01", "--rule", "native-rule", "--reason", "Explicit reason", "--expires-at", "native-expiry"],
      "taskWorkflowPolicyWaive", ["LNT-7/P-01", "native-rule", "Explicit reason", { expiresAt: "native-expiry", remoteName: "secondary" }], "formatPolicyWaive"],
  ]) {
    const payload = { native_result: true };
    runTerminal([...args, "--remote", "secondary"], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native governance text"; },
      },
      write: () => {},
    });
  }
});

test("Policy show delegates JSON compaction and full check text to native presentation", () => {
  const payload = { checks: [{ full_evidence: true }] };
  for (const json of [false, true]) {
    let output = "";
    runTerminal(["policy", "show", " LNT-7/P-01 ", "--remote", "secondary", ...(json ? ["--json"] : [])], {
      runtime: {
        taskWorkflowPolicyShow(_root, reference, options) {
          assert.equal(reference, "LNT-7/P-01");
          assert.deepEqual(options, { remoteName: "secondary" });
          return payload;
        },
        projectPolicyShow(value) { assert.equal(value, payload); return { native_compact: true }; },
        formatPolicyShow(value) { assert.equal(value, payload); return "native full checks"; },
      },
      write: (text) => { output += text; },
    });
    if (json) assert.deepEqual(JSON.parse(output), { native_compact: true });
    else assert.equal(output, "native full checks\n");
  }
});

test("governance syntax and native semantic validation retain different statuses", () => {
  for (const args of [
    ["attest", "put", "123"],
    ["attest", "put", "LNT-7/P-01", "--repo", "7"],
    ["attest", "put", "LNT-7/P-01", "--change", "LNT-7/C-01"],
    ["policy", "show", "123"],
    ["policy", "waive", "LNT-7/P-01", "--rule", "native-rule"],
    ["policy", "waive", "LNT-7/P-01", "--reason", "Explicit reason"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  const nativeError = Object.assign(new Error("native compact Attestation rejects unsupported manual fields"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["attest", "put", "LNT-7/P-01", "--lint", "pass", "--author-mode", "native-mode-value"], {
    runtime: { taskWorkflowAttestPut() { throw nativeError; } },
  }), (error) => error === nativeError && terminalExitCode(error) === 1);
});

test("Diff writes native renderer bytes without appending a console newline", () => {
  const payload = { complete_native_diff: true };
  for (const [args, renderer] of [
    [[], "renderWorkspaceDiffText"],
    [["--stat"], "renderWorkspaceDiffStat"],
    [["--name-only"], "renderWorkspaceDiffNames"],
  ]) {
    for (const rendered of ["", "native Diff\n", "native Diff\n\n"]) {
      let output = "";
      let writes = 0;
      runTerminal(["diff", ...args, "src/a.ts", "folder with spaces", "--", "--leading-path"], {
        cwd: "/repository",
        runtime: {
          workspaceDiff(root, paths) {
            assert.equal(root, "/repository");
            assert.deepEqual(paths, ["src/a.ts", "folder with spaces", "--leading-path"]);
            return payload;
          },
          [renderer](value) { assert.equal(value, payload); return rendered; },
        },
        write: (text) => { output += text; writes++; },
      });
      assert.equal(output, rendered);
      assert.equal(writes, rendered.length > 0 ? 1 : 0);
    }
  }
});

test("Diff JSON retains the complete operation payload and ordinary JSON newline", () => {
  const payload = { native_complete_json: true };
  let output = "";
  runTerminal(["diff", "--json"], {
    runtime: { workspaceDiff(_root, paths) { assert.deepEqual(paths, []); return payload; } },
    write: (text) => { output += text; },
  });
  assert.deepEqual(JSON.parse(output), payload);
  assert.equal(output.endsWith("\n"), true);
  for (const args of [["--json", "--stat"], ["--json", "--name-only"], ["--stat", "--name-only"]]) {
    assert.throws(() => runTerminal(["diff", ...args]), (error) => terminalExitCode(error) === 2);
  }
  let help = "";
  runTerminal(["diff", "--help"], { write: (text) => { help += text; } });
  assert.match(help, /\[PATH\]\.\.\./);
});

test("Blame forwards native source selectors, exact ranges and unchanged paths", () => {
  const defaults = {
    line: undefined, startLine: undefined, endLine: undefined, snapshotId: undefined,
    viaParentSnapshotId: undefined, patchsetId: undefined, remoteName: undefined,
    planId: undefined, planRef: undefined,
  };
  const cases = [
    [[], {}],
    [["--snapshot", "SNP-EXACT", "--via-parent", "SNP-PARENT", "--line", "9007199254740993"],
      { snapshotId: "SNP-EXACT", viaParentSnapshotId: "SNP-PARENT", line: 9007199254740993n }],
    [["--patchset", " LNT-7/P-01 ", "--remote", "secondary", "--start", "7", "--end", "3"],
      { patchsetId: "LNT-7/P-01", remoteName: "secondary", startLine: 7, endLine: 3 }],
    [["--plan-id", "PR-7"], { planId: "PR-7" }],
    [["--plan-ref", "exact/plan-ref"], { planRef: "exact/plan-ref" }],
  ];
  for (const [args, expected] of cases) {
    const payload = { full_native_hunks: true };
    runTerminal(["blame", "path with spaces.txt", ...args], {
      cwd: "/repository",
      runtime: {
        taskWorkflowBlame(...parameters) {
          assert.deepEqual(parameters, ["/repository", "path with spaces.txt", { ...defaults, ...expected }]);
          return payload;
        },
        formatBlame(value) { assert.equal(value, payload); return "native target/range/hunks/warnings"; },
      },
      write: () => {},
    });
  }
});

test("Blame syntax enforces only the frozen argument relationships and bounds", () => {
  for (const args of [
    ["--line", "1", "--start", "1", "--end", "2"],
    ["--start", "1"], ["--end", "2"],
    ["--line", "0"], ["--line", "18446744073709551616"],
    ["--snapshot", "SNP-X", "--patchset", "LNT-7/P-01"],
    ["--snapshot", "SNP-X", "--plan-id", "PR-7"],
    ["--patchset", "LNT-7/P-01", "--plan-ref", "plan/ref"],
    ["--plan-id", "PR-7", "--plan-ref", "plan/ref"],
    ["--remote", "secondary"], ["--via-parent", "SNP-PARENT"],
    ["--patchset", "123"],
  ]) assert.throws(() => runTerminal(["blame", "file.txt", ...args]), (error) => terminalExitCode(error) === 2);
});

test("Pull and Push use locked native operations with unchanged selection and flags", () => {
  const cases = [
    [["pull"], "pullRepository", {
      remoteName: undefined, lineName: undefined, merge: undefined, restore: undefined, force: undefined,
    }, "formatPull"],
    [["pull", "--remote", "secondary", "--line", "feature/sync", "--restore", "--merge"], "pullRepository", {
      remoteName: "secondary", lineName: "feature/sync", merge: true, restore: true, force: undefined,
    }, "formatPull"],
    [["pull", "--restore", "--force"], "pullRepository", {
      remoteName: undefined, lineName: undefined, merge: undefined, restore: true, force: true,
    }, "formatPull"],
    [["push"], "pushRepository", { remoteName: undefined, lineName: undefined }, "formatPush"],
    [["push", "--remote", "secondary", "--line", "feature/sync"], "pushRepository", {
      remoteName: "secondary", lineName: "feature/sync",
    }, "formatPush"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_sync_result: true, phase_timings_ms: { native: 1 }, remote_sync_metrics: { native: 2 } };
    runTerminal(args, {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native transfer/merge/restore text"; },
      },
      write: () => {},
    });
  }
});

test("remote sync JSON uses native projection while SDK payload data remains unchanged", () => {
  for (const [command, method] of [["pull", "pullRepository"], ["push", "pushRepository"]]) {
    const payload = { native_sync_result: true, phase_timings_ms: { native: 1 }, remote_sync_metrics: { native: 2 } };
    let output = "";
    runTerminal([command, "--json"], {
      runtime: {
        [method]: () => payload,
        projectRemoteSync(value) { assert.equal(value, payload); return { native_cli_projection: true }; },
      },
      write: (text) => { output += text; },
    });
    assert.deepEqual(JSON.parse(output), { native_cli_projection: true });
    assert.deepEqual(payload.phase_timings_ms, { native: 1 });
    assert.deepEqual(payload.remote_sync_metrics, { native: 2 });
  }
});

test("Pull syntax constraints do not replace native capability or synchronization errors", () => {
  for (const args of [
    ["pull", "--merge"], ["pull", "--force"], ["pull", "--restore", "--merge", "--force"],
    ["push", "--force"], ["push", "unexpected"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  const nativeError = Object.assign(new Error("native remote capability rejects this synchronization"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["pull", "--restore", "--merge", "--json"], {
    runtime: { pullRepository() { throw nativeError; } },
  }), (error) => error === nativeError && terminalExitCode(error) === 1);
});

test("Worktree handlers use complete native operations and original options", () => {
  const cases = [
    [["status", "owned", "--snapshot", "SNP-X", "--line", "stable", "--verbose"], "taskWorkflowWorktreeStatus",
      [{ name: "owned", snapshotId: "SNP-X", lineName: "stable" }], "formatWorktreeStatus", [true]],
    [["show", "owned"], "worktreeShow", ["owned"], "formatWorktreeShow", []],
    [["restore", "owned", "--snapshot", "SNP-X", "--line", "stable", "--path", "src/a,b.txt", "--path", "folder with spaces", "--force", "--dry-run"], "worktreeRestore",
      [{ name: "owned", snapshotId: "SNP-X", lineName: "stable", paths: ["src/a,b.txt", "folder with spaces"], force: true, dryRun: true }], "formatWorktreeRestore", []],
    [["cleanup-candidates", "--include-protected"], "taskWorkflowWorktreeCleanupCandidates",
      [{ olderThan: "7d", cleanupPolicy: undefined, allowManualOnly: undefined, includeProtected: true }], "formatWorktreeCleanupCandidates", []],
    [["cleanup", "--older-than", "12h", "--policy", "native-policy", "--allow-manual-only", "--limit", "9007199254740993", "--dry-run", "--yes"], "worktreeCleanup",
      [{ olderThan: "12h", cleanupPolicy: "native-policy", allowManualOnly: true, limit: 9007199254740993n, dryRun: true, yes: true }], "formatWorktreeCleanup", []],
    [["prune-stale", "--dry-run", "--yes"], "worktreePruneStale", [{ dryRun: true, yes: true }], "formatWorktreePruneStale", []],
    [["recreate", "owned", "--dry-run"], "worktreeRecreate", [{ name: "owned", dryRun: true }], "formatWorktreeRecreate", []],
    [["recover-task", "LNT-7", "--change", "LNT-7/C-01", "--remote", "secondary", "--dry-run"], "worktreeRecoverTask",
      ["LNT-7", { changeRef: "LNT-7/C-01", remoteName: "secondary", dryRun: true }], "formatWorktreeRecoverTask", []],
    [["restore-owned-head", "owned", "--dry-run"], "worktreeRestoreOwnedHead", [{ name: "owned", dryRun: true }], "formatWorktreeRestoreOwnedHead", []],
    [["rebase", "owned", "--onto", "stable", "--continue", "--abort", "--dry-run"], "worktreeRebase",
      [{ name: "owned", ontoLine: "stable", continueRebase: true, abortRebase: true, dryRun: true }], "formatWorktreeRebase", []],
  ];
  for (const [args, method, expected, formatter, formatArguments] of cases) {
    const payload = { full_native_worktree_result: true };
    runTerminal(["worktree", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](...parameters) { assert.deepEqual(parameters, [payload, ...formatArguments]); return "native Worktree text"; },
      },
      write: () => {},
    });
  }
});

test("Worktree CLI list and doctor request refresh only when explicitly selected", () => {
  for (const [command, method, formatter] of [
    ["list", "taskWorkflowWorktreeList", "formatWorktreeList"],
    ["doctor", "taskWorkflowWorktreeDoctor", "formatWorktreeDoctor"],
  ]) {
    for (const refresh of [false, true]) {
      runTerminal(["worktree", command, ...(refresh ? ["--refresh"] : [])], {
        runtime: {
          [method](_root, options) { assert.deepEqual(options, { refreshStatus: refresh }); return []; },
          [formatter]: () => "native inventory",
        },
        write: () => {},
      });
    }
  }
});

test("Worktree path and open alias delegate shell text and JSON projection", () => {
  const payload = { full_native_path_context: true };
  for (const command of ["path", "open"]) {
    for (const json of [false, true]) {
      let output = "";
      runTerminal(["worktree", command, "owned", "--shell", ...(json ? ["--json"] : [])], {
        runtime: {
          worktreePath(_root, name) { assert.equal(name, "owned"); return payload; },
          projectWorktreePath(value) { assert.equal(value, payload); return { native_path_projection: true }; },
          formatWorktreePath(value, shell) { assert.equal(value, payload); assert.equal(shell, true); return "native shell command"; },
        },
        write: (text) => { output += text; },
      });
      if (json) assert.deepEqual(JSON.parse(output), { native_path_projection: true });
      else assert.equal(output, "native shell command\n");
    }
  }
});

test("Worktree sync returns the original native status for the user's all selection", () => {
  for (const all of [false, true]) {
    const payload = { ok: false };
    assert.equal(runTerminal(["worktree", "sync", ...(all ? ["--all"] : ["owned"]), "--line", "stable", "--force", "--dry-run", "--json"], {
      runtime: {
        worktreeSync(_root, options) {
          assert.deepEqual(options, {
            name: all ? undefined : "owned", allWorktrees: all ? true : undefined,
            lineName: "stable", force: true, dryRun: true,
          });
          return payload;
        },
        worktreeSyncExitCode(value, selectedAll) { assert.equal(value, payload); assert.equal(selectedAll, all); return all ? 2 : 0; },
      },
      write: () => {},
    }), all ? 2 : 0);
  }
});

test("Worktree remove text uses original name count and selection flags", () => {
  for (const [args, names, expectedOptions, expectedPresentation] of [
    [["owned", "--dry-run"], ["owned"],
      { allStale: undefined, deletePath: undefined, force: undefined, dryRun: true, yes: undefined }, [false, true, false]],
    [["same", "same", "--delete-path", "--force", "--yes"], ["same", "same"],
      { allStale: undefined, deletePath: true, force: true, dryRun: undefined, yes: true }, [false, false, true]],
    [["--all-stale", "--dry-run"], [],
      { allStale: true, deletePath: undefined, force: undefined, dryRun: true, yes: undefined }, [true, true, false]],
  ]) {
    const payload = { native_selected_rows: [] };
    runTerminal(["worktree", "remove", ...args], {
      runtime: {
        worktreeRemove(_root, selected, options) { assert.deepEqual(selected, names); assert.deepEqual(options, expectedOptions); return payload; },
        formatWorktreeRemove(...parameters) { assert.deepEqual(parameters, [payload, ...expectedPresentation]); return "native removal text"; },
      },
      write: () => {},
    });
  }
});

test("native Worktree confirmations and operation conflicts remain domain errors", () => {
  const error = Object.assign(new Error("native Worktree confirmation or protection rejected"), { code: "GenericFailure" });
  for (const [args, method] of [
    [["cleanup", "--limit", "0"], "worktreeCleanup"],
    [["prune-stale"], "worktreePruneStale"],
    [["remove"], "worktreeRemove"],
    [["sync", "owned", "--all"], "worktreeSync"],
    [["rebase", "--continue", "--abort"], "worktreeRebase"],
  ]) assert.throws(() => runTerminal(["worktree", ...args], {
    runtime: { [method]() { throw error; } },
  }), (actual) => actual === error && terminalExitCode(actual) === 1);
  assert.throws(() => runTerminal(["worktree", "cleanup", "--limit", "18446744073709551616"]),
    (actual) => terminalExitCode(actual) === 2);
});

test("Git CLI operations preserve paths, immutable selectors and native text", () => {
  const cases = [
    [["migrate", "--staging-root", "/staging with spaces"], "gitMigrate", ["/staging with spaces", false], "formatGitMigrate"],
    [["migrate", "--staging-root", "/staging", "--apply"], "gitMigrate", ["/staging", true], "formatGitMigrate"],
    [["recover", "--apply"], "gitRecover", [true], "formatGitRecover"],
    [["import", "/source repo", "--all-branches-and-tags", "--dry-run"], "gitImport",
      ["/source repo", { allBranchesAndTags: true, dryRun: true }], "formatGitImport"],
    [["export", "/target repo", "--generation", "generation-exact", "--git-object", "object-exact", "--all-lines-and-tags", "--dry-run"], "gitExport",
      ["/target repo", { allLinesAndTags: true, dryRun: true, generation: "generation-exact", gitObject: "object-exact" }], "formatGitExport"],
    [["restore", "--generation", "generation-exact", "--git-object", "object-exact", "--target", "/raw target"], "gitRestore",
      ["generation-exact", "object-exact", "/raw target"], "formatGitRestore"],
    ...["inbound", "outbound", "bidirectional"].map((direction) => [
      ["mirror", "/endpoint repo", "--direction", direction, "--dry-run"], "gitMirror",
      ["/endpoint repo", direction, true], "formatGitMirror",
    ]),
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_git_plan: true };
    runTerminal(["git", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native Git interoperability text"; },
      },
      write: () => {},
    });
  }
});

test("Git reports preserve native success status without interpreting blocked plan fields", () => {
  const payload = { blocked: true, ok: false, dry_run: true, full_native_plan: { retained: true } };
  let output = "";
  assert.equal(runTerminal(["git", "import", "/source repo", "--dry-run", "--json"], {
    runtime: { gitImport: () => payload },
    write: (text) => { output += text; },
  }), 0);
  assert.deepEqual(JSON.parse(output), payload);
  for (const args of [
    ["git", "migrate"],
    ["git", "export", "/target", "--git-object", "object-exact"],
    ["git", "restore", "--generation", "generation-exact", "--git-object", "object-exact"],
    ["git", "restore", "--generation", "generation-exact", "--target", "/target"],
    ["git", "mirror", "/endpoint"],
    ["git", "mirror", "/endpoint", "--direction", "invalid"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
});

test("GC validation emits its native report and returns the validated native status", () => {
  for (const needsAttention of [false, true]) {
    const payload = { needs_attention: needsAttention, native_validation: true };
    const events = [];
    const status = runTerminal(["gc", "validate", "--json"], {
      runtime: {
        gcValidate: () => payload,
        gcValidateExitCode(value) { assert.equal(value, payload); events.push("native-status"); return needsAttention ? 1 : 0; },
      },
      write: (text) => { events.push(JSON.parse(text)); },
    });
    assert.equal(status, needsAttention ? 1 : 0);
    assert.deepEqual(events, ["native-status", payload]);
  }
  const failure = Object.assign(new Error("native needs_attention field validation failed"), { code: "GenericFailure" });
  let output = "";
  assert.throws(() => runTerminal(["gc", "validate", "--json"], {
    runtime: { gcValidate: () => ({}), gcValidateExitCode() { throw failure; } },
    write: (text) => { output += text; },
  }), (error) => error === failure && terminalExitCode(error) === 1);
  assert.equal(output, "");
});

test("GC stats and pruning use native text and preserve preview by default", () => {
  for (const [args, method, expected, formatter] of [
    [["stats"], "gcStats", [], "formatGcStats"],
    [["prune"], "gcPrune", [false], "formatGcPrune"],
    [["prune", "--apply"], "gcPrune", [true], "formatGcPrune"],
  ]) {
    const payload = { native_maintenance: true };
    runTerminal(["gc", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native maintenance text"; },
      },
      write: () => {},
    });
  }
  for (const args of [["gc", "stats", "--all"], ["gc", "prune", "--force"], ["gc", "validate", "--apply"]]) {
    assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  }
});

test("hidden source-cache commands keep explicit paths, native defaults and full-width timestamps", () => {
  const common = ["--namespace-root", "/cache root", "--core-repo-root", "/Core root", "--ext-suffix", ".node"];
  const request = {
    namespaceRoot: "/cache root", coreRepoRoot: "/Core root", extSuffix: ".node",
    rustflags: "", workerId: "shared", coreSourceFingerprint: undefined, serverSourceFingerprint: undefined,
  };
  const cases = [
    [["contract", ...common], "currentSourceNativeCacheContract", [request], "Contract"],
    [["mark-building", ...common, "--source-mtime-ns", "18446744073709551615", "--extra-json", "invalid JSON"],
      "currentSourceCacheMarkBuilding", [request, 18446744073709551615n, "invalid JSON"], "MarkBuilding"],
    [["activate", ...common, "--source-mtime-ns", "0", "--register-lease", "--owner-pid", "4294967295"],
      "currentSourceCacheActivate", [request, 0, { extraJson: "{}", registerLease: true, ownerPid: 4294967295 }], "Activate"],
    [["release-lease", "--lease-path", "/cache root/lease", "--namespace-root", "/cache root"],
      "currentSourceCacheReleaseLease", ["/cache root/lease", "/cache root", false], "ReleaseLease"],
    [["prune", "--namespace-root", "/cache root"], "currentSourceCachePrune", ["/cache root", {
      now: undefined, idleTtlSeconds: 21600, buildStaleSeconds: 900, maxBytes: 2147483648, removeUnleasedReady: false,
    }], "Prune"],
    [["seed-canonical", ...common, "--repo-root", "/repository", "--canonical-repo-root", "/canonical", "--core-source-mtime-ns", "9007199254740993"],
      "currentSourceCacheSeedCanonical", [request, "/repository", "/canonical", 9007199254740993n], "SeedCanonical"],
    [["extension-fresh", "--metadata-path", "/metadata", "--extension-path", "/extension", "--source-mtime-ns", "17", "--source-fingerprint", "exact"],
      "currentSourceCacheExtensionFresh", [{ metadataPath: "/metadata", extensionPath: "/extension", sourceMtimeNs: 17, sourceFingerprint: "exact" }], "ExtensionFresh"],
    [["binary-fresh", "--metadata-path", "/metadata", "--binary-path", "/binary", "--metadata-fingerprint-key", "fingerprint",
      "--metadata-source-mtime-key", "source_mtime", "--metadata-mtime-key", "mtime", "--metadata-sha-key", "sha",
      "--source-mtime-ns", "19", "--source-fingerprint", "exact"], "currentSourceCacheBinaryFresh", [{
      metadataPath: "/metadata", binaryPath: "/binary", metadataFingerprintKey: "fingerprint", metadataSourceMtimeKey: "source_mtime",
      metadataMtimeKey: "mtime", metadataShaKey: "sha", sourceMtimeNs: 19, sourceFingerprint: "exact",
    }], "BinaryFresh"],
    ...["core-fingerprint", "server-fingerprint", "core-mtime", "server-mtime"].map((name) => {
      const suffix = name.split("-").map((word) => word[0].toUpperCase() + word.slice(1)).join("");
      return [[name, "/explicit repo root"], `currentSourceCache${suffix}`, ["/explicit repo root"], suffix];
    }),
  ];
  for (const [args, method, expected, suffix] of cases) {
    const payload = { native_cache_result: true };
    let output = "";
    assert.equal(runTerminal(["current-source-cache", ...args], {
      cwd: "/unrelated cwd",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, expected); return payload; },
        [`formatCurrentSourceCache${suffix}`](value) { assert.equal(value, payload); return "native cache text"; },
      },
      write: (text) => { output += text; },
    }), 0);
    assert.equal(output, "native cache text\n");
  }
  let help = "";
  runTerminal(["--help"], { write: (text) => { help += text; } });
  assert.doesNotMatch(help, /current-source-cache/);
  assert.throws(() => runTerminal(["current-source-cache", "run-cli", "--", "status"]),
    (error) => terminalExitCode(error) === 2);
});

test("cache parser preserves Rust f64 syntax and leaves nonfinite domain validation native", () => {
  for (const [value, expected] of [["1.", 1], [".5", .5], ["1.e3", 1000], ["-1", -1], ["+INF", Infinity],
    ["-infinity", -Infinity], ["NaN", NaN], ["1e309", Infinity]]) {
    runTerminal(["current-source-cache", "prune", "--namespace-root", "/cache", `--now=${value}`, "--json"], {
      runtime: { currentSourceCachePrune(_root, options) { assert.ok(Object.is(options.now, expected)); return {}; } },
      write: () => {},
    });
  }
  for (const value of ["0x10", "1_0", " 1", "", ".", "1e"]) {
    assert.throws(() => runTerminal(["current-source-cache", "prune", "--namespace-root", "/cache", `--now=${value}`]),
      (error) => terminalExitCode(error) === 2);
  }
  const error = Object.assign(new Error("native cache semantic validation"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["current-source-cache", "prune", "--namespace-root", "/cache", "--now=NaN"], {
    runtime: { currentSourceCachePrune() { throw error; } },
  }), (actual) => actual === error && terminalExitCode(actual) === 1);
});

test("cache required arguments, numeric bounds and lease syntax fail before native execution", () => {
  const common = ["--namespace-root", "/cache", "--core-repo-root", "/Core", "--ext-suffix", ".node"];
  for (const args of [
    ["contract", "--namespace-root", "/cache"],
    ["mark-building", ...common],
    ["mark-building", ...common, "--source-mtime-ns", "18446744073709551616"],
    ["activate", ...common, "--source-mtime-ns", "1", "--owner-pid", "42"],
    ["activate", ...common, "--source-mtime-ns", "1", "--register-lease", "--owner-pid", "4294967296"],
    ["prune", "--namespace-root", "/cache", "--max-bytes=-1"],
    ["release-lease", "--lease-path", "/lease"],
    ["binary-fresh", "--metadata-path", "/metadata"],
    ["core-mtime"],
  ]) assert.throws(() => runTerminal(["current-source-cache", ...args]), (error) => terminalExitCode(error) === 2);
});

test("absent native error strings preserve successful init configuration and Plan sync output", () => {
  const payload = { native_success: true };
  for (const [args, method, errorMethod] of [
    [["init", "--json"], "initializeRepository", "configurationError"],
    [["config", "set", "--sprint", "off", "--json"], "configureRepository", "configurationError"],
    [["plan", "sync", "docs/plan.md", "--local", "--json"], "planSync", "planSyncError"],
  ]) {
    let output = "";
    let errors = "";
    assert.equal(runTerminal(args, {
      runtime: {
        [method]: () => payload,
        [errorMethod]: () => undefined,
        formatDomainError() { throw new Error("an absent native error must not be rendered"); },
      },
      write: (text) => { output += text; },
      writeError: (text) => { errors += text; },
    }), 0);
    assert.deepEqual(JSON.parse(output), payload);
    assert.equal(errors, "");
  }
});

test("release CLI passes complete selections to named native operations and native text", () => {
  const cases = [
    [["adapter", "check", "--version", "1.2.3"], "releaseAdapterCheck", ["1.2.3", { lineName: "main", target: undefined }]],
    [["adapter", "build", "--version", "1.2.3", "--line", "release-line", "--target", "x86_64-pc-windows-msvc"],
      "releaseAdapterBuild", ["1.2.3", { lineName: "release-line", target: "x86_64-pc-windows-msvc" }]],
    [["candidate", "create", "--version", "1.2.3"], "releaseCandidateCreate", ["1.2.3", {
      lineName: "main", profile: undefined, channel: undefined, publicSourceRoot: undefined,
    }]],
    [["candidate", "create", "--version", "1.2.3", "--line", "release-line", "--profile", "family", "--channel", "rc", "--public-source-root", "/public source"],
      "releaseCandidateCreate", ["1.2.3", { lineName: "release-line", profile: "family", channel: "rc", publicSourceRoot: "/public source" }]],
    [["check", "REL-1", "--receipts", "/receipt dir", "--public-source-root", "/public source", "--tests-command", "native tests", "--skip-tests-reason", "Native selection"],
      "releaseCheck", ["REL-1", { receipts: "/receipt dir", publicSourceRoot: "/public source", testsCommand: "native tests", skipTestsReason: "Native selection" }]],
    [["build", "REL-1", "--receipts", "/receipt dir", "--public-source-root", "/public source", "--native-matrix-dir", "/matrix", "--native-command-dir", "/command artifacts"],
      "releaseBuild", ["REL-1", { receipts: "/receipt dir", publicSourceRoot: "/public source", nativeMatrixDir: "/matrix", nativeCommandDir: "/command artifacts" }]],
    [["native-source", "REL-1", "--target", "x86_64-pc-windows-msvc", "--source-dir", String.raw`C:\source root`, "--runner", "windows", "--runner-image", "image-exact", "--rust-toolchain", "stable", "--rustc-path", String.raw`C:\Rust\bin\rustc.exe`],
      "releaseNativeSource", [{ releaseId: "REL-1", target: "x86_64-pc-windows-msvc", sourceDir: String.raw`C:\source root`, runner: "windows", runnerImage: "image-exact", rustToolchain: "stable", rustcPath: String.raw`C:\Rust\bin\rustc.exe` }]],
    [["native-bundle", "REL-1", "--native-matrix-dir", "/matrix"], "releaseNativeBundle", ["REL-1", "/matrix"]],
    ...["homebrew", "apt", "winget", "pypi", "npm"].map((channel) => [
      ["package", "REL-1", "--channel", channel, "--public-source-root", "/public source"], "releasePackage", ["REL-1", channel, "/public source"],
    ]),
    [["formula", "REL-1", "--name", "fixture"], "releaseFormula", ["REL-1", "fixture", undefined]],
    [["formula", "REL-1", "--name", "fixture", "--python-formula", "python@3.13"], "releaseFormula", ["REL-1", "fixture", "python@3.13"]],
    [["show", "REL-1", "--remote", "selected", "--public-source-root", "/public source"], "releaseShow", ["REL-1", { remoteName: "selected", publicSourceRoot: "/public source" }]],
    [["publish", "REL-1", "--remote", "selected"], "releasePublish", ["REL-1", "selected"]],
    [["promote", "REL-1", "--channel", "stable", "--public-source-root", "/public source"], "releasePromote", ["REL-1", "stable", "/public source"]],
  ];
  for (const [args, method, expected] of cases) {
    const payload = { native_release: true };
    let output = "";
    assert.equal(runTerminal(["release", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        formatRelease(value) { assert.equal(value, payload); return "native release text\nfull native detail"; },
      },
      write: (text) => { output += text; },
    }), 0);
    assert.equal(output, "native release text\nfull native detail\n");
  }
});

test("release CLI validates frozen required flags and channels before native execution", () => {
  for (const args of [
    ["adapter", "check"], ["candidate", "create"],
    ["candidate", "create", "--version", "1.2.3", "--channel", "dev"],
    ["check"], ["build", "REL-1", "--profile", "family"],
    ["native-source", "REL-1", "--target", "target"], ["native-bundle", "REL-1"],
    ["package", "REL-1"], ["package", "REL-1", "--channel", "github"],
    ["formula", "REL-1"], ["promote", "REL-1"], ["promote", "REL-1", "--channel", "npm"],
  ]) assert.throws(() => runTerminal(["release", ...args]), (error) => terminalExitCode(error) === 2);
  let parentHelp = "";
  runTerminal(["release", "--help"], { write: (text) => { parentHelp += text; } });
  assert.doesNotMatch(parentHelp, /release adapter/);
  let adapterHelp = "";
  runTerminal(["release", "adapter", "--help"], { write: (text) => { adapterHelp += text; } });
  assert.match(adapterHelp, /^  check\s*$/m);
  assert.match(adapterHelp, /^  build\s*$/m);
});

test("release family conflicts remain native domain errors and successful JSON is unprojected", () => {
  const error = Object.assign(new Error("native family/legacy input conflict"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["release", "check", "REL-1", "--receipts", "/receipts", "--tests-command", "native tests"], {
    runtime: { releaseCheck() { throw error; } },
  }), (actual) => actual === error && terminalExitCode(actual) === 1);
  const payload = { ok: false, checks: [{ native: true }], complete_native_detail: { retained: true } };
  let output = "";
  assert.equal(runTerminal(["release", "show", "REL-1", "--json"], {
    runtime: { releaseShow: () => payload }, write: (text) => { output += text; },
  }), 0);
  assert.deepEqual(JSON.parse(output), payload);
});

test("External CLI preserves selection syntax, literal paths and native operation text", () => {
  const cases = [
    [["status"], "externalStatus", [], "formatExternalStatus"],
    [["update"], "externalUpdate", [{ name: undefined, snapshot: undefined, latest: false, locked: false, validate: false, noRecursive: false }], "formatExternalUpdate"],
    [["update", "dep", "--to", "SNP-EXACT", "--validate", "--no-recursive"], "externalUpdate", [{ name: "dep", snapshot: "SNP-EXACT", latest: false, locked: false, validate: true, noRecursive: true }], "formatExternalUpdate"],
    [["update", "dep", "--latest"], "externalUpdate", [{ name: "dep", snapshot: undefined, latest: true, locked: false, validate: false, noRecursive: false }], "formatExternalUpdate"],
    [["update", "--locked", "--validate"], "externalUpdate", [{ name: undefined, snapshot: undefined, latest: false, locked: true, validate: true, noRecursive: false }], "formatExternalUpdate"],
    [["link", "dep", "../linked root"], "externalLink", ["dep", "../linked root"], "formatExternalLink"],
    [["unlink", "dep"], "externalUnlink", ["dep"], "formatExternalUnlink"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_external: true };
    runTerminal(["external", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native External text"; },
      },
      write: () => {},
    });
  }
  for (const args of [
    ["update", "dep"], ["update", "--to", "SNP-EXACT"], ["update", "--latest"],
    ["update", "dep", "--to", "SNP-EXACT", "--latest"], ["update", "dep", "--latest", "--locked"],
    ["update", "--locked", "--to", "SNP-EXACT"], ["link", "dep"], ["unlink"],
  ]) assert.throws(() => runTerminal(["external", ...args]), (error) => terminalExitCode(error) === 2);
});

test("External doctor emits its complete report before returning native diagnostic status", () => {
  for (const failOnBlocking of [false, true]) {
    const payload = { release_ready: false, blocking: [{ native: true }] };
    let output = "";
    assert.equal(runTerminal(["external", "doctor", ...(failOnBlocking ? ["--fail-on-blocking"] : []), "--json"], {
      runtime: { externalDoctor(_root, requested) { assert.equal(requested, failOnBlocking); return { payload, exitCode: failOnBlocking ? 2 : 0 }; } },
      write: (text) => { output += text; },
    }), failOnBlocking ? 2 : 0);
    assert.deepEqual(JSON.parse(output), payload);
  }
});

test("Team CLI preserves native pending envelopes, u32 indexes and quoted text", () => {
  const cases = [
    [["contacts"], "teamContacts", [], "formatTeamContacts", []],
    [["session", "ensure", "--agent", "+4294967295", "--role", "0"], "teamSessionEnsure", [4294967295, 0], "formatTeamSessionEnsure", [4294967295]],
    [["session", "acknowledge", "--activation", "activation-exact"], "teamSessionAcknowledge", ["activation-exact"], "formatTeamSessionAcknowledge", []],
  ];
  for (const [args, method, expected, formatter, formatting] of cases) {
    const payload = { status: "pending", work_started: false, retained_detail: true };
    let output = "";
    assert.equal(runTerminal(["team", ...args], {
      cwd: "/repository",
      runtime: {
        [method](...parameters) { assert.deepEqual(parameters, ["/repository", ...expected]); return { payload, exitCode: 1 }; },
        [formatter](...parameters) { assert.deepEqual(parameters, [payload, ...formatting]); return 'Native status: "pending"'; },
      },
      write: (text) => { output += text; },
    }), 1);
    assert.equal(output, 'Native status: "pending"\n');
    let json = "";
    assert.equal(runTerminal(["team", ...args, "--json"], {
      runtime: { [method]: () => ({ payload, exitCode: 1 }) },
      write: (text) => { json += text; },
    }), 1);
    assert.deepEqual(JSON.parse(json), payload);
  }
  for (const args of [
    ["session", "ensure"], ["session", "ensure", "--agent", "4294967296"],
    ["session", "ensure", "--agent=1.5"], ["session", "ensure", "--agent", "1", "--role=-1"],
    ["session", "acknowledge"],
  ]) assert.throws(() => runTerminal(["team", ...args]), (error) => terminalExitCode(error) === 2);
});

test("hidden Binary DB CLI uses explicit pre-discovery paths and native selector validation", () => {
  const cases = [
    [["upgrade-u64-seconds", "--generation-root", "generation", "--source-time-width", "u32-time-v0", "--activate"],
      "binaryDbUpgradeU64Seconds", [".", "generation", "u32-time-v0", true], "formatBinaryDbUpgradeU64Seconds"],
    [["agent-pool50", "--repo-root", "/uninitialized source", "--generation-root", "/generation"],
      "binaryDbAgentPool50", ["/uninitialized source", "/generation", false], "formatBinaryDbAgentPool50"],
  ];
  for (const [args, method, expected, formatter] of cases) {
    const payload = { native_generation: true };
    runTerminal(["binary-db", ...args], {
      runtime: {
        repositoryAuthorityRoot() { throw new Error("maintenance must not discover normal repository authority"); },
        [method](...parameters) { assert.deepEqual(parameters, expected); return payload; },
        [formatter](value) { assert.equal(value, payload); return "native maintenance text"; },
      },
      write: () => {},
    });
  }
  const failure = Object.assign(new Error("native source selector rejected"), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["binary-db", "upgrade-u64-seconds", "--generation-root", "generation", "--source-time-width", "u32"], {
    runtime: { binaryDbUpgradeU64Seconds() { throw failure; } },
  }), (error) => error === failure && terminalExitCode(error) === 1);
  for (const args of [["agent-pool50"], ["upgrade-u64-seconds"], ["upgrade-u64-seconds", "--generation-root", "generation"], ["agent-pool-50"]]) {
    assert.throws(() => runTerminal(["binary-db", ...args]), (error) => terminalExitCode(error) === 2);
  }
  let rootHelp = "";
  runTerminal(["--help"], { write: (text) => { rootHelp += text; } });
  assert.doesNotMatch(rootHelp, /binary-db/);
  let groupHelp = "";
  runTerminal(["binary-db", "--help"], { write: (text) => { groupHelp += text; } });
  assert.match(groupHelp, /upgrade-u64-seconds/);
  assert.doesNotMatch(groupHelp, /agent-pool50/);
});

test("Snapshot capture and commit use the shared authoring scalar before effects", () => {
  for (const prefix of [["snapshot", "create"], ["commit"]]) {
    let received;
    runTerminal([...prefix, "  LT-0007/C-01  ", "--json"], {
      runtime: {
        parseSnapshotAuthoringReference(value) { assert.equal(value, "  LT-0007/C-01  "); return "LT-0007/C-01"; },
        snapshotCapture(_root, reference) { received = reference; return {}; },
        projectSnapshotCapture: (payload) => payload,
      },
      write: () => {},
    });
    assert.equal(received, "LT-0007/C-01");
    const invalid = Object.assign(new Error("Snapshot creation requires a Task ID."), { code: "GenericFailure" });
    assert.throws(() => runTerminal([...prefix, "C-01"], {
      runtime: {
        parseSnapshotAuthoringReference() { throw invalid; },
        snapshotCapture() { throw new Error("invalid scalar must not reach capture"); },
      },
    }), (error) => terminalExitCode(error) === 2 && error.message === invalid.message);
  }
});

test("remote head recovery preserves pre-discovery operation and frozen jobs syntax", () => {
  for (const [args, expected] of [
    [[], { remoteName: undefined, jobs: 8, apply: false }],
    [["--remote", "selected", "--jobs", "+64", "--apply"], { remoteName: "selected", jobs: 64, apply: true }],
  ]) {
    const payload = { apply: expected.apply, complete_native_history: true };
    let output = "";
    assert.equal(runTerminal(["remote", "recover-head", ...args], {
      cwd: "/damaged repository",
      runtime: {
        repositoryAuthorityRoot() { throw new Error("recovery must not open normal local authority"); },
        remoteRecoverHead(root, options) { assert.equal(root, "/damaged repository"); assert.deepEqual(options, expected); return payload; },
        formatRemoteRecoverHead(value) { assert.equal(value, payload); return "native recovery text"; },
      },
      write: (text) => { output += text; },
    }), 0);
    assert.equal(output, "native recovery text\n");
  }
  for (const jobs of ["0", "65", "-1", "1.5", "18446744073709551616", " 8"]) {
    assert.throws(() => runTerminal(["remote", "recover-head", `--jobs=${jobs}`]),
      (error) => terminalExitCode(error) === 2 && error.message === "jobs must be an integer from 1 through 64");
  }
});

test("every supported help path renders without native loading or repository access", () => {
  const paths = terminalHelpPaths();
  assert.equal(paths.length, 193);
  assert.equal(paths.includes("current-source-cache run-cli"), false);
  const runtime = new Proxy({}, { get() { throw new Error("help must not load native code"); } });
  for (const name of paths) {
    for (const flag of ["-h", "--help"]) {
      let output = "";
      const status = runFrontend([...name.split(" ").filter(Boolean), flag], {
        runtime, write: (text) => { output += text; },
        writeError: () => assert.fail("help must use stdout"),
      });
      assert.equal(status, 0, `${name} ${flag}`);
      assert.equal(output, terminalHelp(name, flag === "--help"));
      assert.match(output, /Usage: ait/);
    }
  }
});

test("help preserves canonical hierarchy, aliases, descriptions and option details", () => {
  function help(args) {
    let text = "";
    assert.equal(runFrontend(args, { write: (value) => { text += value; } }), 0);
    return text;
  }
  const root = help([]);
  const rootCommands = root.split("Commands:\n")[1].split("\nOptions:")[0];
  assert.equal(rootCommands.split("\n").filter((line) => /^  \S/.test(line)).length, 30);
  assert.match(root, /line\s+Manage named Lines.*\[alias: branch\]/);
  for (const hidden of ["binary-db", "current-source-cache", "auth", "change"]) {
    assert.doesNotMatch(rootCommands, new RegExp(`^  ${hidden}\\s`, "m"));
  }
  assert.equal(help(["branch", "--help"]), help(["line", "--help"]));
  assert.equal(help(["help", "branch", "create"]), help(["line", "create", "--help"]));
  assert.equal(help(["help", "worktree", "open"]), help(["worktree", "path", "--help"]));
  assert.equal(help(["worktree", "open", "--help"]), help(["worktree", "path", "--help"]));
  assert.equal(help(["review", "help", "task", "approve"]), help(["review", "task", "approve", "--help"]));
  assert.notEqual(help(["line", "-h"]), help(["line", "--help"]));
  assert.match(help(["task", "start", "--help"]), /--from <MARKDOWN_PATH#ITEM_REF>/);
  assert.match(help(["task", "start", "--help"]), /--intent <INTENT>/);
  assert.match(help(["remote", "recover-head", "--help"]), /\[default: 8\]/);
  assert.match(help(["release", "package", "--help"]), /homebrew, apt, winget, pypi, npm/);
});

test("missing group subcommands emit native short help on stderr with syntax status", () => {
  for (const name of ["task", "review team", "release adapter", "binary-db"]) {
    let stderr = "";
    const status = runFrontend(name.split(" "), {
      write: () => assert.fail("missing subcommand help belongs on stderr"),
      writeError: (text) => { stderr += text; },
    });
    assert.equal(status, 2);
    assert.equal(stderr, terminalHelp(name, false));
  }
});

test("help honors option order, value boundaries and the positional separator", () => {
  for (const args of [
    ["--help", "unexpected"], ["task", "--help", "unexpected"],
    ["task", "show", "--help", "--unknown"], ["task", "show", "-hx"],
  ]) {
    let output = "";
    assert.equal(runFrontend(args, { write: (text) => { output += text; } }), 0);
    assert.match(output, /Usage: ait/);
  }
  for (const args of [
    ["task", "show", "--unknown", "--help"], ["task", "show", "--remote", "--help"],
    ["task", "show", "--help=true"], ["task", "show", "-xh"],
  ]) assert.throws(() => runTerminal(args), (error) => terminalExitCode(error) === 2);
  const calls = [];
  for (const args of [
    ["task", "show", "T", "--remote=--help", "--json"],
    ["task", "show", "--json", "--", "--help"],
  ]) {
    assert.equal(runTerminal(args, {
      runtime: { taskWorkflowTaskShow(...values) { calls.push(values); return {}; } }, write: () => {},
    }), 0);
  }
  assert.equal(calls[0][2].remoteName, "--help");
  assert.equal(calls[1][1], "--help");
});

test("typed scalar diagnostics preserve native syntax text and token order", () => {
  const footer = "\nFor more information, try '--help'.\n";
  const cases = [
    [["remote", "recover-head", "--jobs=0", "--help"],
      "error: invalid value '0' for '--jobs <JOBS>': jobs must be from 1 through 64\n"],
    [["remote", "recover-head", "--jobs=x"],
      "error: invalid value 'x' for '--jobs <JOBS>': jobs must be an integer from 1 through 64\n"],
    [["team", "session", "ensure", "--agent=-1"],
      "error: invalid value '-1' for '--agent <AGENT>': -1 is not in 0..=4294967295\n"],
    [["team", "session", "ensure", "--agent=4294967296"],
      "error: invalid value '4294967296' for '--agent <AGENT>': 4294967296 is not in 0..=4294967295\n"],
    [["team", "session", "ensure", "--agent=18446744073709551615"],
      "error: invalid value '18446744073709551615' for '--agent <AGENT>': number too large to fit in target type\n"],
    [["config", "set", "--task-worktree-main-seed-ram-max-bytes="],
      "error: invalid value '' for '--task-worktree-main-seed-ram-max-bytes <TASK_WORKTREE_MAIN_SEED_RAM_MAX_BYTES>': cannot parse integer from empty string\n"],
    [["blame", "--line=x", "--unknown"],
      "error: invalid value 'x' for '--line <LINE>': line number must be a positive integer\n"],
    [["snapshot", "ancestry", "head", "--limit=0"],
      "error: invalid value '0' for '--limit <COUNT>': value must be 1 or greater\n"],
    [["repo", "jobs", "--state=zzzz", "--help"],
      "error: invalid value 'zzzz' for '--state <STATE>'\n  [possible values: queued, running, succeeded, failed]\n"],
    [["binary-db", "agent-pool50", "--generation-root=", "--help"],
      "error: a value is required for '--generation-root <GENERATION_ROOT>' but none was supplied\n"],
    [["plan", "sync", "", "--help"],
      "error: a value is required for '<TARGET>' but none was supplied\n"],
    [["current-source-cache", "prune", "--now=bad", "--help"],
      "error: invalid value 'bad' for '--now <NOW>': invalid float literal\n"],
  ];
  for (const [args, expected] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      assert.equal(reportTerminalError(error, { writeError: (text) => { output += text; } }), 2);
      assert.equal(output, expected + footer, args.join(" "));
      return true;
    });
  }
});

test("terminal values are normalized once and explicit zero retains native u32 syntax", () => {
  let scalarCalls = 0;
  let patchset;
  runTerminal(["patchset", "show", "  T-1/P-01  ", "--json"], {
    runtime: {
      validatePublicPatchsetArgument(value) { scalarCalls++; return value.trim(); },
      taskWorkflowPatchsetShow(_root, value) { patchset = value; return {}; },
    }, write: () => {},
  });
  assert.equal(scalarCalls, 1);
  assert.equal(patchset, "T-1/P-01");
  let agent;
  runTerminal(["team", "session", "ensure", "--agent=-0", "--json"], {
    runtime: { teamSessionEnsure(_root, value) { agent = value; return { payload: {}, exitCode: 0 }; } },
    write: () => {},
  });
  assert.equal(agent, 0);
  let remote;
  runTerminal(["task", "show", "T", "--remote=", "--json"], {
    runtime: { taskWorkflowTaskShow(_root, _task, options) { remote = options.remoteName; return {}; } },
    write: () => {},
  });
  assert.equal(remote, "");
});

test("unknown arguments, duplicates and flag values preserve native syntax diagnostics", () => {
  const cases = [
    [["unknown"], "error: unrecognized subcommand 'unknown'\n\nUsage: ait <COMMAND>\n"],
    [["task", "unknown"], "error: unrecognized subcommand 'unknown'\n\nUsage: ait task <COMMAND>\n"],
    [["status", "--bogus"], "error: unexpected argument '--bogus' found\n\nUsage: ait status [OPTIONS]\n"],
    [["task", "show", "T", "--bogus"], "error: unexpected argument '--bogus' found\n\n  tip: to pass '--bogus' as a value, use '-- --bogus'\n\nUsage: ait task show <TASK_ID>\n"],
    [["status", "extra"], "error: unexpected argument 'extra' found\n\nUsage: ait status [OPTIONS]\n"],
    [["task", "show", "T", "--remote", "one", "--remote", "two"],
      "error: the argument '--remote <REMOTE>' cannot be used multiple times\n\nUsage: ait task show [OPTIONS] <TASK_ID>\n"],
    [["task", "show", "T", "--remote"], "error: a value is required for '--remote <REMOTE>' but none was supplied\n"],
    [["status", "--json=false"], "error: unexpected value 'false' for '--json' found; no more were expected\n\nUsage: ait status --json\n"],
  ];
  for (const [args, prefix] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      reportTerminalError(error, { writeError: (text) => { output += text; } });
      assert.equal(output, `${prefix}\nFor more information, try '--help'.\n`);
      return true;
    });
  }
});

test("short message options strip exactly one inline equals sign", () => {
  for (const [flag, expected] of [["-m=message", "message"], ["-m==message", "=message"], ["-m=", ""], ["-m=--help", "--help"]]) {
    let message;
    runTerminal(["commit", "T-1", flag, "--json"], {
      runtime: {
        snapshotCapture(_root, _reference, options) { message = options.message; return {}; },
        projectSnapshotCapture(payload) { return payload; },
      }, write: () => {},
    });
    assert.equal(message, expected);
  }
});

test("the JavaScript executable renders help and scalar errors outside a repository", () => {
  const entry = fileURLToPath(new URL("../bin/ait.mjs", import.meta.url));
  const help = spawnSync(process.execPath, [entry, "task", "start", "--help"], {
    cwd: os.tmpdir(), encoding: "utf8", windowsHide: true,
  });
  assert.ifError(help.error);
  assert.equal(help.signal, null);
  assert.equal(help.status, 0, help.stderr);
  assert.equal(help.stderr, "");
  assert.equal(help.stdout, terminalHelp("task start"));
  const invalid = spawnSync(process.execPath, [entry, "remote", "recover-head", "--jobs=0"], {
    cwd: os.tmpdir(), encoding: "utf8", windowsHide: true,
  });
  assert.ifError(invalid.error);
  assert.equal(invalid.signal, null);
  assert.equal(invalid.status, 2);
  assert.equal(invalid.stdout, "");
  assert.equal(invalid.stderr, "error: invalid value '0' for '--jobs <JOBS>': jobs must be from 1 through 64\n\nFor more information, try '--help'.\n");
});

test("required flags, conditional inputs and dependency errors preserve native diagnostic order", () => {
  const cases = [
    [["line", "switch", "--force"],
      "  --restore\n  <NAME>", "ait line switch --restore --force <NAME>"],
    [["line", "delete"], "  --yes\n  <NAME>", "ait line delete --yes <NAME>"],
    [["review", "team", "request", "T"], "  --group <REVIEWER_GROUPS>", "ait review team request --group <REVIEWER_GROUPS> <TASK_ID>"],
    [["release", "candidate", "create", "--public-source-root", "dir"],
      "  --version <VERSION>", "ait release candidate create --version <VERSION>"],
    [["snapshot", "restore-lines", "S", "file"],
      "  <--line <LINE>|--start <START_LINE>>", "ait snapshot restore-lines <SNAPSHOT_ID> <PATH> (--line <N> | --start <N> --end <N>) [--yes] [--json]"],
    [["task", "start", "--full"], "  --intent <INTENT>\n  --json\n  --title <TITLE>",
      "ait task start --intent <INTENT> (--from <MARKDOWN_PATH#ITEM_REF> | --title <TITLE>) [--edit-root <ABSOLUTE_PATH>] [--local | --remote <REMOTE>] [--json [--full]]"],
  ];
  for (const [args, missing, usage] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      reportTerminalError(error, { writeError: (text) => { output += text; } });
      assert.equal(output, `error: the following required arguments were not provided:\n${missing}\n\nUsage: ${usage}\n\nFor more information, try '--help'.\n`);
      return true;
    });
  }
  for (const args of [["task", "start", "--full", "--help"], ["pull", "--force", "--merge", "--help"]]) {
    assert.equal(runTerminal(args, { write: () => {} }), 0);
  }
});

test("conflict diagnostics follow input order and retain surviving dependencies", () => {
  const cases = [
    [["task", "show", "T", "--remote", "one", "--local"],
      "the argument '--remote <REMOTE>' cannot be used with '--local'", "ait task show --remote <REMOTE> <TASK_ID>"],
    [["pull", "--force", "--merge"],
      "the argument '--force' cannot be used with '--merge'", "ait pull --restore --force"],
    [["diff", "--stat", "--json", "--name-only"],
      "the argument '--stat' cannot be used with:\n  --json\n  --name-only", "ait diff --stat [PATH]..."],
    [["line", "merge", "source", "--continue"],
      "the argument '[SOURCE]' cannot be used with '--continue'", "ait line merge <SOURCE>"],
  ];
  for (const [args, message, usage] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      reportTerminalError(error, { writeError: (text) => { output += text; } });
      assert.equal(output, `error: ${message}\n\nUsage: ${usage}\n\nFor more information, try '--help'.\n`);
      return true;
    });
  }
});

test("syntax suggestions retain native similarity, group context and JSON suppression", () => {
  const cases = [
    [["stats"], "error: unrecognized subcommand 'stats'\n\n  tip: some similar subcommands exist: 'attest', 'stash', 'status'\n\nUsage: ait <COMMAND>"],
    [["task", "shw"], "error: unrecognized subcommand 'shw'\n\n  tip: a similar subcommand exists: 'show'\n\nUsage: ait task <COMMAND>"],
    [["status", "--jsn"], "error: unexpected argument '--jsn' found\n\n  tip: a similar argument exists: '--json'\n\nUsage: ait status --json"],
    [["--json", "status"], "error: unexpected argument '--json' found\n\n  tip: 'status --json' exists\n\nUsage: ait <COMMAND>"],
    [["task", "--remtoe", "x", "show", "T"], "error: unexpected argument '--remtoe' found\n\n  tip: 'show --remote' exists\n\nUsage: ait task <COMMAND>"],
    [["repo", "jobs", "--state=runnin"], "error: invalid value 'runnin' for '--state <STATE>'\n  [possible values: queued, running, succeeded, failed]\n\n  tip: a similar value exists: 'running'"],
    [["repo", "jobs", "--state="], "error: a value is required for '--state <STATE>' but none was supplied\n  [possible values: queued, running, succeeded, failed]"],
    [["workflow", "ready", "T", "--josn", "--json"], "error: unexpected argument '--josn' found\n\nUsage: ait workflow ready --json <TASK_ID>"],
  ];
  for (const [args, expected] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      reportTerminalError(error, { writeError: (text) => { output += text; } });
      assert.equal(output, `${expected}\n\nFor more information, try '--help'.\n`);
      return true;
    });
  }
});

test("malformed boolean values preserve resolved argument groups and required choices", () => {
  const cases = [
    [["line", "rename", "old", "new", "--json=oops"],
      "unexpected value 'oops' for '--json' found; no more were expected\n\nUsage: ait line rename <OLD|NEW|--remote <REMOTE>|--json>"],
    [["binary-db", "upgrade-u64-seconds", "--generation-root", "dir", "--source-time-width", "value", "--help=oops"],
      "unexpected value 'oops' for '--help' found; no more were expected\n\nUsage: ait binary-db upgrade-u64-seconds --help <--repo-root <REPO_ROOT>|--generation-root <GENERATION_ROOT>|--source-time-width <SOURCE_TIME_WIDTH>|--activate|--json>"],
    [["git", "mirror", "endpoint", "--direction"],
      "a value is required for '--direction <DIRECTION>' but none was supplied\n  [possible values: inbound, outbound, bidirectional]"],
    [["task", "--help=oops"], "unexpected value 'oops' for '--help' found; no more were expected\n\nUsage: ait task --help <COMMAND>"],
    [["help", "task", "unknown"], "unrecognized subcommand 'unknown'\n\nUsage: ait task <COMMAND>"],
    [["workflow", "guide", "--json=oops"], "unexpected argument '--json' found\n\n  tip: to pass '--json' as a value, use '-- --json'\n\nUsage: ait workflow guide [TOPIC]"],
  ];
  for (const [args, expected] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      reportTerminalError(error, { writeError: (text) => { output += text; } });
      assert.equal(output, `error: ${expected}\n\nFor more information, try '--help'.\n`);
      return true;
    });
  }
  let output = "";
  assert.equal(runTerminal(["task", "-hfoo"], { write: (text) => { output += text; } }), 0);
  assert.equal(output, terminalHelp("task", false));
});

test("guide and review template CLI preserve native repository admission", () => {
  for (const [args, operation, format] of [
    [["workflow", "guide"], "workflowGuide", "formatWorkflowGuide"],
    [["review", "code", "template"], "taskWorkflowReviewCodeTemplate", "formatReviewCodeTemplate"],
  ]) {
    const events = [];
    runTerminal(args, {
      cwd: "/fixture",
      runtime: {
        repositoryAuthorityRoot(root) { events.push(["repository", root]); return "/canonical"; },
        [operation]() { events.push(["operation"]); return {}; },
        [format]() { return "native text"; },
      }, write: () => {},
    });
    assert.deepEqual(events, [["repository", "/fixture"], ["operation"]]);
    const failure = Object.assign(new Error("native repository missing"), { code: "GenericFailure" });
    assert.throws(() => runTerminal(args, {
      runtime: {
        repositoryAuthorityRoot() { throw failure; },
        [operation]() { assert.fail("operation cannot precede native repository discovery"); },
      },
    }), (error) => error === failure);
  }
});

test("built-in help and group separators preserve their native grammar", () => {
  for (const [args, usage] of [
    [["help", "help"], "ait help"], [["help", "task", "help"], "ait task help"],
    [["task", "help", "help"], "ait task help"],
  ]) {
    let output = "";
    assert.equal(runTerminal(args, { write: (text) => { output += text; } }), 0);
    assert.equal(output, `Print this message or the help of the given subcommand(s)\n\nUsage: ${usage} [COMMAND]...\n\nArguments:\n  [COMMAND]...  Print help for the subcommand(s)\n`);
  }
  const cases = [
    [["help", "help", "task"], "error: unrecognized subcommand 'task'\n\nUsage: ait help [COMMAND]...\n"],
    [["task", "--", "show"], "error: unexpected argument 'show' found\n\n  tip: subcommand 'show' exists; to use it, remove the '--' before it\n\nUsage: ait task <COMMAND>\n\nFor more information, try '--help'.\n"],
    [["--", "--help"], "error: unrecognized subcommand '--help'\n\n  tip: a similar subcommand exists: 'help'\n\nUsage: ait <COMMAND>\n\nFor more information, try '--help'.\n"],
    [["doctor", "memory-roo"], "error: unrecognized subcommand 'memory-roo'\n\n  tip: some similar subcommands exist: 'runtime-root', 'memory-root'\n\nUsage: ait doctor <COMMAND>\n\nFor more information, try '--help'.\n"],
  ];
  for (const [args, expected] of cases) {
    let output = "";
    assert.throws(() => runTerminal(args), (error) => {
      reportTerminalError(error, { writeError: (text) => { output += text; } });
      assert.equal(output, expected);
      return true;
    });
  }
  let stderr = "";
  assert.equal(runTerminal(["task", "--"], { writeError: (text) => { stderr += text; } }), 2);
  assert.equal(stderr, terminalHelp("task", false));
  assert.throws(() => runTerminal(["workflow", "guide", "--json"], {
    runtime: { repositoryAuthorityRoot() { assert.fail("invalid syntax must precede discovery"); } },
  }), (error) => terminalExitCode(error) === 2);
});

test("reconcile retains exact native-size limits and native domain errors", () => {
  for (const [value, expected] of [[undefined, undefined], ["0", 0], ["4294967297", 4294967297],
    ["9007199254740993", 9007199254740993n], ["18446744073709551615", 18446744073709551615n]]) {
    let options;
    runTerminal(["workflow", "reconcile", ...(value === undefined ? [] : ["--limit", value]), "--json"], {
      runtime: { workflowReconcile(_root, request) { options = request; return {}; } }, write: () => {},
    });
    assert.equal(options.limit, expected);
  }
  const failure = Object.assign(new Error("--limit must be between 1 and 10000."), { code: "GenericFailure" });
  assert.throws(() => runTerminal(["workflow", "reconcile", "--limit", "4294967297"], {
    runtime: { workflowReconcile(_root, options) { assert.equal(options.limit, 4294967297); throw failure; } },
  }), (error) => error === failure && terminalExitCode(error) === 1);
});
