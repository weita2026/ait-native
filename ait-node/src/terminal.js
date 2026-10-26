import { parseArgs } from "node:util";
import { NativeBridgeError } from "./errors.js";
import { terminalHelp, terminalHelpPaths } from "./terminal-help.js";

class TerminalUsageError extends Error {
  constructor(message) {
    super(message);
    this.name = "TerminalUsageError";
  }
}

const BOOLEAN = Object.freeze({ type: "boolean" });
const STRING = Object.freeze({ type: "string" });
const SCOPE = Object.freeze({ local: BOOLEAN, remote: STRING });
const TASK_START_OPTIONS = Object.freeze({
  ...SCOPE, title: STRING, intent: STRING, "edit-root": STRING, full: BOOLEAN,
});
const CAPTURE = {
  options: { message: { ...STRING, short: "m" }, resume: STRING, full: BOOLEAN },
  positional: ["TASK_ID"],
  invoke: (runtime, root, values, [reference]) => {
    if (values.message !== undefined && values.resume !== undefined) {
      throw new TerminalUsageError("--message conflicts with --resume");
    }
    return runtime.snapshotCapture(root, reference, {
      message: values.message, resumeSnapshotId: values.resume,
    });
  },
  project: (runtime, payload, values) => runtime.projectSnapshotCapture(payload, values.full ?? false),
  format: (runtime, payload) => runtime.formatSnapshotCapture(payload),
};
const CONFIG_FIELDS = Object.freeze({
  "agent-teams": "agent_teams",
  "plan-language": "plan_language",
  "plan-style": "plan_style",
  sprint: "sprint",
  "default-author-mode": "default_author_mode",
  "default-model": "default_model",
  "task-review": "task_review",
  "task-worktree-alias-root": "task_worktree_alias_root",
  "task-worktree-main-seed-ram-max-bytes": "task_worktree_main_seed_ram_max_bytes",
  "id-namespace-prefix": "id_namespace_prefix",
  "user-name": "user_name",
  "user-email": "user_email",
  "post-finish-hooks": "post_finish_hooks",
});
const AUTHOR_MODES = Object.freeze([
  "human_only", "human_with_ai_assist", "ai_with_human_review", "ai_only_experimental",
]);
const CONFIG_ENUMS = Object.freeze({
  "agent-teams": ["enabled", "disabled"],
  "plan-style": ["concise", "detailed"],
  sprint: ["on", "off"],
  "default-author-mode": AUTHOR_MODES,
  "task-review": ["required", "automatic"],
});
const CONFIG_UNSET_KEYS = Object.freeze(Object.keys(CONFIG_FIELDS)
  .filter((key) => !["agent-teams", "sprint"].includes(key)));
const REVIEW_TEAM_OPTIONS = Object.freeze({ patchset: STRING, reviewer: STRING, message: STRING, remote: STRING });
const REVIEW_TASK_OPTIONS = Object.freeze({ patchset: STRING, message: STRING, remote: STRING });
const SOURCE_CACHE_OPTIONS = Object.freeze({
  "namespace-root": STRING, "core-repo-root": STRING, "ext-suffix": STRING,
  rustflags: STRING, "worker-id": STRING,
  "core-source-fingerprint": STRING, "server-source-fingerprint": STRING,
});
const COMMANDS = Object.freeze({
  init: {
    options: { "policy-profile": STRING, "repair-existing": BOOLEAN },
    positional: [],
    invoke: (runtime, root, values) => runtime.initializeRepository(root, {
      policyProfile: values["policy-profile"],
      repairExisting: values["repair-existing"],
    }),
    afterOutputError: (runtime, payload) => runtime.configurationError(payload),
    format: (runtime, payload) => runtime.formatInitializeRepository(payload),
  },
  status: {
    options: { full: BOOLEAN }, positional: [],
    invoke: (runtime, root) => runtime.taskWorkflowStatus(root),
    project: (runtime, payload, values) => runtime.projectStatus(payload, values.full ?? false),
    format: (runtime, payload) => runtime.formatStatus(payload),
  },
  pull: {
    options: { remote: STRING, line: STRING, merge: BOOLEAN, restore: BOOLEAN, force: BOOLEAN },
    positional: [],
    validate: (values) => {
      if (values.merge && !values.restore) throw new TerminalUsageError("--merge requires --restore");
      if (values.force && !values.restore) throw new TerminalUsageError("--force requires --restore");
      if (values.force && values.merge) throw new TerminalUsageError("--force conflicts with --merge");
    },
    invoke: (runtime, root, values) => runtime.pullRepository(root, {
      remoteName: values.remote, lineName: values.line,
      merge: values.merge, restore: values.restore, force: values.force,
    }),
    project: (runtime, payload) => runtime.projectRemoteSync(payload),
    format: (runtime, payload) => runtime.formatPull(payload),
  },
  push: {
    options: { remote: STRING, line: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.pushRepository(root, {
      remoteName: values.remote, lineName: values.line,
    }),
    project: (runtime, payload) => runtime.projectRemoteSync(payload),
    format: (runtime, payload) => runtime.formatPush(payload),
  },
  diff: {
    options: { stat: BOOLEAN, "name-only": BOOLEAN },
    positional: ["PATH"], optionalPositionals: 1, variadic: true,
    verbatimText: true,
    validate: (values) => {
      if ([values.json, values.stat, values["name-only"]].filter(Boolean).length > 1) {
        throw new TerminalUsageError("--json, --stat and --name-only conflict");
      }
    },
    invoke: (runtime, root, _values, paths) => runtime.workspaceDiff(root, paths),
    format: (runtime, payload, values) => values["name-only"]
      ? runtime.renderWorkspaceDiffNames(payload)
      : values.stat ? runtime.renderWorkspaceDiffStat(payload) : runtime.renderWorkspaceDiffText(payload),
  },
  blame: {
    options: {
      line: STRING, start: STRING, end: STRING, snapshot: STRING,
      "via-parent": STRING, patchset: STRING, remote: STRING, "plan-id": STRING, "plan-ref": STRING,
    },
    positional: ["PATH"],
    validate: (values) => {
      if (values.line !== undefined && (values.start !== undefined || values.end !== undefined)) {
        throw new TerminalUsageError("--line conflicts with --start and --end");
      }
      if ((values.start === undefined) !== (values.end === undefined)) {
        throw new TerminalUsageError("--start and --end require each other");
      }
      if (["snapshot", "patchset", "plan-id", "plan-ref"].filter((name) => values[name] !== undefined).length > 1) {
        throw new TerminalUsageError("--snapshot, --patchset, --plan-id and --plan-ref conflict");
      }
      if (values["via-parent"] !== undefined && values.snapshot === undefined) {
        throw new TerminalUsageError("--via-parent requires --snapshot");
      }
      if (values.remote !== undefined && values.patchset === undefined) {
        throw new TerminalUsageError("--remote requires --patchset");
      }
    },
    invoke: (runtime, root, values, [path]) => runtime.taskWorkflowBlame(root, path, {
      line: positiveNativeSize(values.line, "--line"),
      startLine: positiveNativeSize(values.start, "--start"),
      endLine: positiveNativeSize(values.end, "--end"), snapshotId: values.snapshot,
      viaParentSnapshotId: values["via-parent"], patchsetId: values.patchset,
      remoteName: values.remote, planId: values["plan-id"], planRef: values["plan-ref"],
    }),
    format: (runtime, payload) => runtime.formatBlame(payload),
  },
  "config show": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.taskWorkflowConfigShow(root),
    format: (runtime, payload) => runtime.formatConfigurationShow(payload),
  },
  "config set": {
    options: Object.fromEntries(Object.keys(CONFIG_FIELDS).map((name) => [name, STRING])),
    positional: [],
    validate: (values) => {
      for (const [name, choices] of Object.entries(CONFIG_ENUMS)) {
        validateChoice(values[name], `--${name}`, choices);
      }
    },
    invoke: (runtime, root, values, _positionals, context) => {
      context.request = Object.fromEntries(Object.entries(CONFIG_FIELDS)
        .filter(([name]) => values[name] !== undefined)
        .map(([name, field]) => [field, name === "task-worktree-main-seed-ram-max-bytes"
          ? signed64(values[name], `--${name}`) : values[name]]));
      return runtime.configureRepository(root, context.request);
    },
    afterOutputError: (runtime, payload) => runtime.configurationError(payload),
    format: (runtime, payload, _values, _positionals, context) =>
      runtime.formatConfiguration(payload, context.request),
  },
  "config unset": {
    options: {}, positional: ["KEY"],
    validate: (_values, [key]) => validateChoice(key, "KEY", CONFIG_UNSET_KEYS),
    invoke: (runtime, root, _values, [key]) => runtime.unsetConfiguration(root, key),
    format: (runtime, payload, _values, [key]) => runtime.formatConfigurationUnset(payload, key),
  },
  "remote add": {
    options: { default: BOOLEAN }, positional: ["NAME", "URL"],
    invoke: (runtime, root, values, [name, url]) => runtime.taskWorkflowRemoteAdd(root, {
      name, url, make_default: values.default ?? false,
    }),
    format: (runtime, payload) => runtime.formatRemoteAdd(payload),
  },
  "remote recover-head": {
    options: { remote: STRING, jobs: STRING, apply: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.remoteRecoverHead(root, {
      remoteName: values.remote, jobs: parseRemoteRecoveryJobs(values.jobs ?? "8"), apply: values.apply ?? false,
    }),
    format: (runtime, payload) => runtime.formatRemoteRecoverHead(payload),
  },
  "remote list": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.taskWorkflowRemoteList(root),
    format: (runtime, payload) => runtime.formatRemoteList(payload),
  },
  "release adapter check": {
    hidden: true, hiddenParent: "release",
    options: { version: STRING, line: STRING, target: STRING }, positional: [],
    validate: (values) => requireOption(values, "version"),
    invoke: (runtime, root, values) => runtime.releaseAdapterCheck(root, values.version, {
      lineName: values.line ?? "main", target: values.target,
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release adapter build": {
    hidden: true, hiddenParent: "release",
    options: { version: STRING, line: STRING, target: STRING }, positional: [],
    validate: (values) => requireOption(values, "version"),
    invoke: (runtime, root, values) => runtime.releaseAdapterBuild(root, values.version, {
      lineName: values.line ?? "main", target: values.target,
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release candidate create": {
    options: { version: STRING, line: STRING, profile: STRING, channel: STRING, "public-source-root": STRING }, positional: [],
    validate: (values) => { requireOption(values, "version"); validateChoice(values.channel, "--channel", ["rc", "stable"]); },
    invoke: (runtime, root, values) => runtime.releaseCandidateCreate(root, values.version, {
      lineName: values.line ?? "main", profile: values.profile, channel: values.channel,
      publicSourceRoot: values["public-source-root"],
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release check": {
    options: { receipts: STRING, "public-source-root": STRING, "tests-command": STRING, "skip-tests-reason": STRING },
    positional: ["RELEASE_ID"],
    invoke: (runtime, root, values, [releaseId]) => runtime.releaseCheck(root, releaseId, {
      receipts: values.receipts, publicSourceRoot: values["public-source-root"],
      testsCommand: values["tests-command"], skipTestsReason: values["skip-tests-reason"],
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release build": {
    options: { receipts: STRING, "public-source-root": STRING, "native-matrix-dir": STRING, "native-command-dir": STRING },
    positional: ["RELEASE_ID"],
    invoke: (runtime, root, values, [releaseId]) => runtime.releaseBuild(root, releaseId, {
      receipts: values.receipts, publicSourceRoot: values["public-source-root"],
      nativeMatrixDir: values["native-matrix-dir"], nativeCommandDir: values["native-command-dir"],
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release native-source": {
    options: { target: STRING, "source-dir": STRING, runner: STRING, "runner-image": STRING, "rust-toolchain": STRING, "rustc-path": STRING },
    positional: ["RELEASE_ID"],
    validate: (values) => {
      for (const name of ["target", "source-dir", "runner", "runner-image", "rust-toolchain", "rustc-path"]) requireOption(values, name);
    },
    invoke: (runtime, root, values, [releaseId]) => runtime.releaseNativeSource(root, {
      releaseId, target: values.target, sourceDir: values["source-dir"], runner: values.runner,
      runnerImage: values["runner-image"], rustToolchain: values["rust-toolchain"], rustcPath: values["rustc-path"],
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release native-bundle": {
    options: { "native-matrix-dir": STRING }, positional: ["RELEASE_ID"],
    validate: (values) => requireOption(values, "native-matrix-dir"),
    invoke: (runtime, root, values, [releaseId]) => runtime.releaseNativeBundle(root, releaseId, values["native-matrix-dir"]),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release package": {
    options: { channel: STRING, "public-source-root": STRING }, positional: ["RELEASE_ID"],
    validate: (values) => {
      requireOption(values, "channel"); validateChoice(values.channel, "--channel", ["homebrew", "apt", "winget", "pypi", "npm"]);
    },
    invoke: (runtime, root, values, [releaseId]) => runtime.releasePackage(root, releaseId, values.channel, values["public-source-root"]),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release formula": {
    options: { name: STRING, "python-formula": STRING }, positional: ["RELEASE_ID"],
    validate: (values) => requireOption(values, "name"),
    invoke: (runtime, root, values, [releaseId]) => runtime.releaseFormula(root, releaseId, values.name, values["python-formula"]),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release show": {
    options: { remote: STRING, "public-source-root": STRING }, positional: ["RELEASE_ID"],
    invoke: (runtime, root, values, [releaseId]) => runtime.releaseShow(root, releaseId, {
      remoteName: values.remote, publicSourceRoot: values["public-source-root"],
    }),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release publish": {
    options: { remote: STRING }, positional: ["RELEASE_ID"],
    invoke: (runtime, root, values, [releaseId]) => runtime.releasePublish(root, releaseId, values.remote),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "release promote": {
    options: { channel: STRING, "public-source-root": STRING }, positional: ["RELEASE_ID"],
    validate: (values) => { requireOption(values, "channel"); validateChoice(values.channel, "--channel", ["rc", "stable"]); },
    invoke: (runtime, root, values, [releaseId]) => runtime.releasePromote(root, releaseId, values.channel, values["public-source-root"]),
    format: (runtime, payload) => runtime.formatRelease(payload),
  },
  "external status": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.externalStatus(root),
    format: (runtime, payload) => runtime.formatExternalStatus(payload),
  },
  "external doctor": {
    options: { "fail-on-blocking": BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.externalDoctor(root, values["fail-on-blocking"] ?? false),
    payload: (result) => result.payload,
    format: (runtime, payload) => runtime.formatExternalDoctor(payload),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "external update": {
    options: { to: STRING, latest: BOOLEAN, locked: BOOLEAN, validate: BOOLEAN, "no-recursive": BOOLEAN },
    positional: ["NAME"], optionalPositionals: 1,
    validate: (values, [name]) => {
      if (values.to !== undefined && values.latest) throw new TerminalUsageError("--to conflicts with --latest");
      if (values.locked && (name !== undefined || values.to !== undefined || values.latest)) {
        throw new TerminalUsageError("--locked conflicts with NAME, --to and --latest");
      }
      if (name !== undefined && values.to === undefined && !values.latest) {
        throw new TerminalUsageError("NAME requires --to or --latest");
      }
      if (name === undefined && (values.to !== undefined || values.latest)) {
        throw new TerminalUsageError("--to and --latest require NAME");
      }
    },
    invoke: (runtime, root, values, [name]) => runtime.externalUpdate(root, {
      name, snapshot: values.to, latest: values.latest ?? false, locked: values.locked ?? false,
      validate: values.validate ?? false, noRecursive: values["no-recursive"] ?? false,
    }),
    format: (runtime, payload) => runtime.formatExternalUpdate(payload),
  },
  "external link": {
    options: {}, positional: ["NAME", "PATH"],
    invoke: (runtime, root, _values, [name, path]) => runtime.externalLink(root, name, path),
    format: (runtime, payload) => runtime.formatExternalLink(payload),
  },
  "external unlink": {
    options: {}, positional: ["NAME"],
    invoke: (runtime, root, _values, [name]) => runtime.externalUnlink(root, name),
    format: (runtime, payload) => runtime.formatExternalUnlink(payload),
  },
  "team contacts": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.teamContacts(root),
    payload: (result) => result.payload,
    format: (runtime, payload) => runtime.formatTeamContacts(payload),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "team session ensure": {
    options: { agent: STRING, role: STRING }, positional: [],
    validate: (values) => requireOption(values, "agent"),
    invoke: (runtime, root, values, _positionals, context) => {
      context.agent = unsigned32(values.agent, "--agent");
      return runtime.teamSessionEnsure(root, context.agent, unsigned32(values.role, "--role"));
    },
    payload: (result) => result.payload,
    format: (runtime, payload, _values, _positionals, context) => runtime.formatTeamSessionEnsure(payload, context.agent),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "team session acknowledge": {
    options: { activation: STRING }, positional: [],
    validate: (values) => requireOption(values, "activation"),
    invoke: (runtime, root, values) => runtime.teamSessionAcknowledge(root, values.activation),
    payload: (result) => result.payload,
    format: (runtime, payload) => runtime.formatTeamSessionAcknowledge(payload),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "binary-db upgrade-u64-seconds": {
    hidden: true,
    options: { "repo-root": STRING, "generation-root": STRING, "source-time-width": STRING, activate: BOOLEAN }, positional: [],
    validate: (values) => { requireOption(values, "generation-root"); requireOption(values, "source-time-width"); },
    invoke: (runtime, _root, values) => runtime.binaryDbUpgradeU64Seconds(values["repo-root"] ?? ".",
      values["generation-root"], values["source-time-width"], values.activate ?? false),
    format: (runtime, payload) => runtime.formatBinaryDbUpgradeU64Seconds(payload),
  },
  "binary-db agent-pool50": {
    hidden: true, hiddenParent: "binary-db",
    options: { "repo-root": STRING, "generation-root": STRING, activate: BOOLEAN }, positional: [],
    validate: (values) => requireOption(values, "generation-root"),
    invoke: (runtime, _root, values) => runtime.binaryDbAgentPool50(values["repo-root"] ?? ".", values["generation-root"], values.activate ?? false),
    format: (runtime, payload) => runtime.formatBinaryDbAgentPool50(payload),
  },
  "repo show": {
    options: { remote: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.repositoryShow(root, values.remote),
    format: (runtime, payload, values) => runtime.formatRepositoryShow(payload, values.remote),
  },
  "repo jobs": {
    options: { remote: STRING, "worker-job-index": STRING, state: STRING, limit: STRING },
    positional: [],
    validate: (values) => {
      validateChoice(values.state, "--state", ["queued", "running", "succeeded", "failed"]);
      if (values["worker-job-index"] !== undefined &&
          (values.state !== undefined || values.limit !== undefined)) {
        throw new TerminalUsageError("--worker-job-index conflicts with --state and --limit");
      }
    },
    invoke: (runtime, root, values, _positionals, context) => {
      const limit = unsigned32(values.limit, "--limit");
      if (limit !== undefined && (limit < 1 || limit > 1000)) {
        throw new TerminalUsageError("--limit must be from 1 through 1000");
      }
      context.selection = {
        remoteName: values.remote,
        workerJobIndex: unsigned32(values["worker-job-index"], "--worker-job-index"),
        state: values.state, limit,
      };
      return runtime.repositoryJobs(root, context.selection);
    },
    format: (runtime, payload, _values, _positionals, context) =>
      runtime.formatRepositoryJobs(payload, context.selection),
  },
  "repo ci-capabilities": {
    options: { remote: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.repositoryCiCapabilities(root, values.remote),
    format: (runtime, payload, values) => runtime.formatRepositoryCiCapabilities(payload, values.remote),
  },
  "repo retire": {
    options: { remote: STRING, abort: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.repositoryRetire(root, values.remote, values.abort),
    format: (runtime, payload) => runtime.formatRepositoryRetire(payload),
  },
  "repo restore": {
    options: { remote: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.repositoryRestore(root, values.remote),
    format: (runtime, payload) => runtime.formatRepositoryRestore(payload),
  },
  "auth whoami": {
    hidden: true,
    options: { remote: STRING, repo: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowAuthWhoami(root, {
      remoteName: values.remote, repoName: values.repo,
    }),
    format: (runtime, payload) => runtime.formatAuthWhoami(payload),
  },
  "auth bindings": {
    hidden: true,
    options: { remote: STRING, repo: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowAuthBindings(root, {
      remoteName: values.remote, repoName: values.repo,
    }),
    format: (runtime, payload) => runtime.formatAuthBindings(payload),
  },
  "auth grant": {
    hidden: true,
    options: { remote: STRING, repo: STRING, actor: STRING, role: { ...STRING, multiple: true } },
    positional: [],
    validate: (values) => requireOption(values, "actor"),
    invoke: (runtime, root, values) => runtime.taskWorkflowAuthGrant(root, values.actor, values.role ?? [], {
      remoteName: values.remote, repoName: values.repo,
    }),
    format: (runtime, payload) => runtime.formatAuthGrant(payload),
  },
  "queue summary": {
    options: { remote: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.queueSummary(root, values.remote),
    format: (runtime, payload) => runtime.formatQueueSummary(payload),
  },
  "doctor memory-root": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.doctorMemoryRoot(root),
    format: (runtime, payload) => runtime.formatDoctorMemoryRoot(payload),
  },
  "doctor runtime-root": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.taskWorkflowDoctorRuntimeRoot(root),
    format: (runtime, payload) => runtime.formatDoctorRuntimeRoot(payload),
  },
  "doctor plan-authority": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.doctorPlanAuthority(root),
    format: (runtime, payload) => runtime.formatDoctorPlanAuthority(payload),
  },
  "git migrate": {
    options: { "staging-root": STRING, apply: BOOLEAN }, positional: [],
    validate: (values) => requireOption(values, "staging-root"),
    invoke: (runtime, root, values) => runtime.gitMigrate(root, values["staging-root"], values.apply ?? false),
    format: (runtime, payload) => runtime.formatGitMigrate(payload),
  },
  "git recover": {
    options: { apply: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.gitRecover(root, values.apply ?? false),
    format: (runtime, payload) => runtime.formatGitRecover(payload),
  },
  "git import": {
    options: { "all-branches-and-tags": BOOLEAN, "dry-run": BOOLEAN }, positional: ["SOURCE"],
    invoke: (runtime, root, values, [source]) => runtime.gitImport(root, source, {
      allBranchesAndTags: values["all-branches-and-tags"], dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatGitImport(payload),
  },
  "git export": {
    options: { generation: STRING, "git-object": STRING, "all-lines-and-tags": BOOLEAN, "dry-run": BOOLEAN },
    positional: ["TARGET"],
    validate: (values) => {
      if (values["git-object"] !== undefined && values.generation === undefined) {
        throw new TerminalUsageError("--git-object requires --generation");
      }
    },
    invoke: (runtime, root, values, [target]) => runtime.gitExport(root, target, {
      allLinesAndTags: values["all-lines-and-tags"], dryRun: values["dry-run"],
      generation: values.generation, gitObject: values["git-object"],
    }),
    format: (runtime, payload) => runtime.formatGitExport(payload),
  },
  "git restore": {
    options: { generation: STRING, "git-object": STRING, target: STRING }, positional: [],
    validate: (values) => {
      requireOption(values, "generation"); requireOption(values, "git-object"); requireOption(values, "target");
    },
    invoke: (runtime, root, values) => runtime.gitRestore(root, values.generation, values["git-object"], values.target),
    format: (runtime, payload) => runtime.formatGitRestore(payload),
  },
  "git mirror": {
    options: { direction: STRING, "dry-run": BOOLEAN }, positional: ["ENDPOINT"],
    validate: (values) => {
      requireOption(values, "direction");
      validateChoice(values.direction, "--direction", ["inbound", "outbound", "bidirectional"]);
    },
    invoke: (runtime, root, values, [endpoint]) => runtime.gitMirror(root, endpoint, values.direction, values["dry-run"] ?? false),
    format: (runtime, payload) => runtime.formatGitMirror(payload),
  },
  "current-source-cache contract": {
    hidden: true, options: SOURCE_CACHE_OPTIONS, positional: [],
    validate: requireSourceCacheOptions,
    invoke: (runtime, _root, values) => runtime.currentSourceNativeCacheContract(sourceCacheRequest(values)),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheContract(payload),
  },
  "current-source-cache mark-building": {
    hidden: true,
    options: { ...SOURCE_CACHE_OPTIONS, "source-mtime-ns": STRING, "extra-json": STRING }, positional: [],
    validate: (values) => { requireSourceCacheOptions(values); requireOption(values, "source-mtime-ns"); },
    invoke: (runtime, _root, values) => runtime.currentSourceCacheMarkBuilding(
      sourceCacheRequest(values), nativeSize(values["source-mtime-ns"], "--source-mtime-ns"), values["extra-json"] ?? "{}"),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheMarkBuilding(payload),
  },
  "current-source-cache activate": {
    hidden: true,
    options: { ...SOURCE_CACHE_OPTIONS, "source-mtime-ns": STRING, "extra-json": STRING,
      "register-lease": BOOLEAN, "owner-pid": STRING }, positional: [],
    validate: (values) => {
      requireSourceCacheOptions(values); requireOption(values, "source-mtime-ns");
      if (values["owner-pid"] !== undefined && !values["register-lease"]) {
        throw new TerminalUsageError("--owner-pid requires --register-lease");
      }
    },
    invoke: (runtime, _root, values) => runtime.currentSourceCacheActivate(
      sourceCacheRequest(values), nativeSize(values["source-mtime-ns"], "--source-mtime-ns"), {
        extraJson: values["extra-json"] ?? "{}", registerLease: values["register-lease"] ?? false,
        ownerPid: unsigned32(values["owner-pid"], "--owner-pid"),
      }),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheActivate(payload),
  },
  "current-source-cache release-lease": {
    hidden: true,
    options: { "lease-path": STRING, "namespace-root": STRING, "remove-unleased-ready": BOOLEAN }, positional: [],
    validate: (values) => { requireOption(values, "lease-path"); requireOption(values, "namespace-root"); },
    invoke: (runtime, _root, values) => runtime.currentSourceCacheReleaseLease(
      values["lease-path"], values["namespace-root"], values["remove-unleased-ready"] ?? false),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheReleaseLease(payload),
  },
  "current-source-cache prune": {
    hidden: true,
    options: { "namespace-root": STRING, now: STRING, "idle-ttl-seconds": STRING,
      "build-stale-seconds": STRING, "max-bytes": STRING, "remove-unleased-ready": BOOLEAN }, positional: [],
    validate: (values) => requireOption(values, "namespace-root"),
    invoke: (runtime, _root, values) => runtime.currentSourceCachePrune(values["namespace-root"], {
      now: nativeFloat(values.now, "--now"),
      idleTtlSeconds: nativeSize(values["idle-ttl-seconds"] ?? "21600", "--idle-ttl-seconds"),
      buildStaleSeconds: nativeSize(values["build-stale-seconds"] ?? "900", "--build-stale-seconds"),
      maxBytes: nativeSize(values["max-bytes"] ?? "2147483648", "--max-bytes"),
      removeUnleasedReady: values["remove-unleased-ready"] ?? false,
    }),
    format: (runtime, payload) => runtime.formatCurrentSourceCachePrune(payload),
  },
  "current-source-cache seed-canonical": {
    hidden: true,
    options: { ...SOURCE_CACHE_OPTIONS, "repo-root": STRING, "canonical-repo-root": STRING,
      "core-source-mtime-ns": STRING }, positional: [],
    validate: (values) => {
      requireSourceCacheOptions(values);
      for (const name of ["repo-root", "canonical-repo-root", "core-source-mtime-ns"]) requireOption(values, name);
    },
    invoke: (runtime, _root, values) => runtime.currentSourceCacheSeedCanonical(
      sourceCacheRequest(values), values["repo-root"], values["canonical-repo-root"],
      nativeSize(values["core-source-mtime-ns"], "--core-source-mtime-ns")),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheSeedCanonical(payload),
  },
  "current-source-cache extension-fresh": {
    hidden: true,
    options: { "metadata-path": STRING, "extension-path": STRING, "source-mtime-ns": STRING,
      "source-fingerprint": STRING }, positional: [],
    validate: (values) => {
      for (const name of ["metadata-path", "extension-path", "source-mtime-ns", "source-fingerprint"]) requireOption(values, name);
    },
    invoke: (runtime, _root, values) => runtime.currentSourceCacheExtensionFresh({
      metadataPath: values["metadata-path"], extensionPath: values["extension-path"],
      sourceMtimeNs: nativeSize(values["source-mtime-ns"], "--source-mtime-ns"),
      sourceFingerprint: values["source-fingerprint"],
    }),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheExtensionFresh(payload),
  },
  "current-source-cache binary-fresh": {
    hidden: true,
    options: { "metadata-path": STRING, "binary-path": STRING, "metadata-fingerprint-key": STRING,
      "metadata-source-mtime-key": STRING, "metadata-mtime-key": STRING, "metadata-sha-key": STRING,
      "source-mtime-ns": STRING, "source-fingerprint": STRING }, positional: [],
    validate: (values) => {
      for (const name of ["metadata-path", "binary-path", "metadata-fingerprint-key", "metadata-source-mtime-key",
        "metadata-mtime-key", "metadata-sha-key", "source-mtime-ns", "source-fingerprint"]) requireOption(values, name);
    },
    invoke: (runtime, _root, values) => runtime.currentSourceCacheBinaryFresh({
      metadataPath: values["metadata-path"], binaryPath: values["binary-path"],
      metadataFingerprintKey: values["metadata-fingerprint-key"],
      metadataSourceMtimeKey: values["metadata-source-mtime-key"],
      metadataMtimeKey: values["metadata-mtime-key"], metadataShaKey: values["metadata-sha-key"],
      sourceMtimeNs: nativeSize(values["source-mtime-ns"], "--source-mtime-ns"),
      sourceFingerprint: values["source-fingerprint"],
    }),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheBinaryFresh(payload),
  },
  "current-source-cache core-fingerprint": {
    hidden: true, options: {}, positional: ["REPO_ROOT"],
    invoke: (runtime, _root, _values, [repoRoot]) => runtime.currentSourceCacheCoreFingerprint(repoRoot),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheCoreFingerprint(payload),
  },
  "current-source-cache server-fingerprint": {
    hidden: true, options: {}, positional: ["REPO_ROOT"],
    invoke: (runtime, _root, _values, [repoRoot]) => runtime.currentSourceCacheServerFingerprint(repoRoot),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheServerFingerprint(payload),
  },
  "current-source-cache core-mtime": {
    hidden: true, options: {}, positional: ["REPO_ROOT"],
    invoke: (runtime, _root, _values, [repoRoot]) => runtime.currentSourceCacheCoreMtime(repoRoot),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheCoreMtime(payload),
  },
  "current-source-cache server-mtime": {
    hidden: true, options: {}, positional: ["REPO_ROOT"],
    invoke: (runtime, _root, _values, [repoRoot]) => runtime.currentSourceCacheServerMtime(repoRoot),
    format: (runtime, payload) => runtime.formatCurrentSourceCacheServerMtime(payload),
  },
  "gc stats": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.gcStats(root),
    format: (runtime, payload) => runtime.formatGcStats(payload),
  },
  "gc validate": {
    options: {}, positional: [],
    invoke: (runtime, root, _values, _positionals, context) => {
      const payload = runtime.gcValidate(root);
      context.validationExitCode = runtime.gcValidateExitCode(payload);
      return payload;
    },
    format: (runtime, payload) => runtime.formatGcValidate(payload),
    exitCode: (_runtime, _payload, _operation, _values, context) => context.validationExitCode,
  },
  "gc prune": {
    options: { apply: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.gcPrune(root, values.apply ?? false),
    format: (runtime, payload) => runtime.formatGcPrune(payload),
  },
  "line list": {
    options: { all: BOOLEAN, archived: BOOLEAN, remote: STRING }, positional: [],
    validate: (values) => {
      if (values.all && values.archived) throw new TerminalUsageError("--all conflicts with --archived");
    },
    invoke: (runtime, root, values) => runtime.lineInventory(root, lineListOptions(values)),
    payload: (inventory) => inventory.payload,
    format: (runtime, _payload, values, _positionals, _context, inventory) =>
      runtime.formatLineList(inventory, lineListOptions(values)),
  },
  "line create": {
    options: { "from-snapshot": STRING, switch: BOOLEAN }, positional: ["NAME"],
    invoke: (runtime, root, values, [name]) => runtime.taskWorkflowLineCreate(root, name, {
      fromSnapshot: values["from-snapshot"], switch: values.switch,
    }),
    format: (runtime, payload) => runtime.formatLineCreate(payload),
  },
  "line switch": {
    options: { restore: BOOLEAN, force: BOOLEAN }, positional: ["NAME"],
    validate: (values) => {
      if (values.force && !values.restore) throw new TerminalUsageError("--force requires --restore");
    },
    invoke: (runtime, root, values, [name]) => runtime.taskWorkflowLineSwitch(root, name, {
      restore: values.restore, force: values.force,
    }),
    format: (runtime, payload) => runtime.formatLineSwitch(payload),
  },
  "line show": {
    options: { remote: STRING }, positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.lineShow(root, name, values.remote),
    format: (runtime, payload) => runtime.formatLineShow(payload),
  },
  "line archive": {
    options: { remote: STRING }, positional: ["NAME"],
    invoke: (runtime, root, values, [name]) => runtime.taskWorkflowLineArchive(root, name, values.remote),
    format: (runtime, payload) => runtime.formatLineArchive(payload),
  },
  "line rename": {
    options: { remote: STRING }, positional: ["OLD", "NEW"],
    invoke: (runtime, root, values, [oldName, newName]) =>
      runtime.taskWorkflowLineRename(root, oldName, newName, values.remote),
    format: (runtime, payload) => runtime.formatLineRename(payload),
  },
  "line delete": {
    options: { remote: STRING, yes: BOOLEAN }, positional: ["NAME"],
    validate: (values) => {
      if (!values.yes) throw new TerminalUsageError("--yes is required");
    },
    invoke: (runtime, root, values, [name]) => runtime.taskWorkflowLineDelete(root, name, {
      remoteName: values.remote, confirm: values.yes,
    }),
    format: (runtime, payload) => runtime.formatLineDelete(payload),
  },
  "line merge": {
    options: { message: STRING, continue: BOOLEAN, abort: BOOLEAN },
    positional: ["SOURCE"], optionalPositionals: 1,
    invoke: (runtime, root, values, [source]) => {
      if (source !== undefined && (values.continue || values.abort)) {
        throw new TerminalUsageError("SOURCE conflicts with --continue and --abort");
      }
      if (values.abort && (values.continue || values.message !== undefined)) {
        throw new TerminalUsageError("--abort conflicts with --continue and --message");
      }
      return runtime.lineMerge(root, {
        source, message: values.message, continueMerge: values.continue, abortMerge: values.abort,
      });
    },
    format: (runtime, payload) => runtime.formatLineMerge(payload),
  },
  "line cleanup": {
    options: {
      "idle-for": STRING, kind: STRING, limit: STRING, "include-protected": BOOLEAN,
      all: BOOLEAN, yes: BOOLEAN,
    },
    positional: [],
    invoke: (runtime, root, values, _positionals, context) => {
      context.idleFor = values["idle-for"] ?? "7d";
      context.selection = {
        cleanupKind: values.kind, limit: nativeSize(values.limit, "--limit"),
        includeProtected: values["include-protected"],
      };
      return runtime.taskWorkflowLineCleanup(root, {
        ...context.selection, idleFor: context.idleFor, apply: values.yes,
      });
    },
    format: (runtime, payload, values, _positionals, context) =>
      runtime.formatLineCleanup(payload, context.idleFor, { ...context.selection, all: values.all }),
  },
  "plan list": {
    options: { ...SCOPE, all: BOOLEAN }, positional: [],
    validate: rejectAllJson,
    invoke: (runtime, root, values) => runtime.planList(root, scope(values)),
    format: (runtime, payload, values) => runtime.formatPlanList(payload, {
      ...scope(values), all: values.all,
    }),
  },
  "plan show": {
    options: { ...SCOPE, revision: STRING }, positional: ["PLAN"],
    invoke: (runtime, root, values, [planId]) => runtime.planShow(root, planId, planQuery(values)),
    format: (runtime, payload) => runtime.formatPlanShow(payload),
  },
  "plan revisions": {
    options: { ...SCOPE, all: BOOLEAN }, positional: ["PLAN"],
    validate: rejectAllJson,
    invoke: (runtime, root, values, [planId]) => runtime.planRevisions(root, planId, scope(values)),
    format: (runtime, payload, values, [planId]) => runtime.formatPlanRevisions(payload, planId, {
      ...scope(values), all: values.all,
    }),
  },
  "plan items": {
    options: { ...SCOPE, revision: STRING }, positional: ["PLAN"],
    invoke: (runtime, root, values, [planId]) => runtime.planItems(root, planId, planQuery(values)),
    format: (runtime, payload) => runtime.formatPlanItems(payload),
  },
  "plan inspect": {
    options: { ...SCOPE, revision: STRING }, positional: ["PLAN"],
    invoke: (runtime, root, values, [planId]) => runtime.planInspect(root, planId, planQuery(values)),
    format: (runtime, payload) => runtime.formatPlanInspect(payload),
  },
  "plan candidates": {
    options: { ...SCOPE, all: BOOLEAN, contains: STRING }, positional: [],
    invoke: (runtime, root, values) => runtime.planCandidates(root, {
      ...scope(values), includeAll: values.all, contains: values.contains,
    }),
    format: (runtime, payload) => runtime.formatPlanCandidates(payload),
  },
  "plan sync": {
    options: { ...SCOPE, "plan-ref": STRING, prune: BOOLEAN, reconcile: BOOLEAN },
    positional: ["TARGET"],
    invoke: (runtime, root, values, [target]) => runtime.planSync(root, target, {
      ...scope(values), planRef: values["plan-ref"], prune: values.prune, reconcile: values.reconcile,
    }),
    afterOutputError: (runtime, payload) => runtime.planSyncError(payload),
    format: (runtime, payload) => runtime.formatPlanSync(payload),
  },
  "workflow ready": {
    retainAuthorityRoot: true,
    options: {
      apply: BOOLEAN, remote: STRING, "expected-identity": STRING, full: BOOLEAN,
      "snapshot-message": STRING, summary: STRING, tests: STRING, lint: STRING,
      security: STRING, license: STRING, "author-mode": STRING, model: STRING,
      "run-ci-once": BOOLEAN,
    },
    positional: ["TASK_ID"],
    validate: (values) => {
      requireApply(values, [
        "snapshot-message", "summary", "tests", "lint", "security", "license",
        "author-mode", "model", "run-ci-once",
      ]);
      validateChoice(values["author-mode"], "--author-mode", AUTHOR_MODES);
    },
    invoke: (runtime, root, values, [taskId]) => runtime.workflowReady(root, taskId, {
      apply: values.apply, remoteName: values.remote,
      expectedIdentityPath: values["expected-identity"],
      snapshotMessage: values["snapshot-message"], summary: values.summary,
      tests: values.tests, lint: values.lint, security: values.security,
      license: values.license, authorMode: values["author-mode"], model: values.model,
      runCiOnce: values["run-ci-once"],
    }),
    payload: (result) => result.payload,
    project: (runtime, payload, values, [taskId], root) => runtime.projectWorkflowReady(root, payload, taskId, {
      remoteName: values.remote, full: values.full ?? false,
    }),
    format: (runtime, payload, _values, [taskId]) => runtime.formatWorkflowReady(payload, taskId),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "workflow finish": {
    retainAuthorityRoot: true,
    options: {
      apply: BOOLEAN, remote: STRING, "expected-identity": STRING, full: BOOLEAN,
      reviewer: STRING, "review-message": STRING,
    },
    positional: ["TASK_ID"],
    validate: (values) => requireApply(values, ["reviewer", "review-message"]),
    invoke: (runtime, root, values, [taskId]) => runtime.workflowFinish(root, taskId, {
      apply: values.apply, remoteName: values.remote,
      expectedIdentityPath: values["expected-identity"],
      reviewer: values.reviewer, reviewMessage: values["review-message"],
    }),
    payload: (result) => result.payload,
    project: (runtime, payload, values, [taskId], root) => runtime.projectWorkflowFinish(root, payload, taskId, {
      remoteName: values.remote, full: values.full ?? false,
    }),
    format: (runtime, payload, _values, [taskId]) => runtime.formatWorkflowFinish(payload, taskId),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "workflow reconcile": {
    options: {
      remote: STRING, task: STRING, "dry-run": BOOLEAN, apply: BOOLEAN,
      "safe-only": BOOLEAN, scheduled: BOOLEAN, limit: STRING,
    },
    positional: [],
    invoke: (runtime, root, values) => {
      if (values["dry-run"] && values.apply) {
        throw new TerminalUsageError("--dry-run conflicts with --apply");
      }
      if (values.scheduled && !values.apply) {
        throw new TerminalUsageError("--scheduled requires --apply");
      }
      return runtime.workflowReconcile(root, {
        remoteName: values.remote, taskId: values.task, apply: values.apply,
        safeOnly: values["safe-only"], scheduled: values.scheduled,
        limit: nativeSize(values.limit, "--limit"),
      });
    },
    format: (runtime, payload) => runtime.formatWorkflowReconcile(payload),
  },
  "workflow guide": {
    options: {}, positional: ["TOPIC"], optionalPositionals: 1, discoverRepository: true,
    invoke: (runtime, _root, _values, [topic]) => runtime.workflowGuide(topic),
    format: (runtime, payload) => runtime.formatWorkflowGuide(payload),
  },
  "task list": {
    options: { ...SCOPE, all: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowTaskList(root, {
      ...scope(values), all: true,
    }),
    project: (runtime, payload, values) => runtime.projectTaskList(payload, values.all ?? false),
    format: (runtime, payload, values) => runtime.formatTaskList(payload, {
      ...scope(values), all: values.all ?? false,
    }),
  },
  "task start": {
    options: { ...TASK_START_OPTIONS, from: STRING }, positional: [],
    invoke: (runtime, root, values, _positionals, context) =>
      runtime.taskStart(root, taskRequest(values, false, context.stdoutIsTTY)),
    project: (runtime, payload, values) => runtime.projectTaskStart(payload, values.full ?? false),
    format: (runtime, payload) => runtime.formatTaskStart(payload),
  },
  "task quick": {
    options: TASK_START_OPTIONS, positional: [],
    invoke: (runtime, root, values, _positionals, context) =>
      runtime.taskQuick(root, taskRequest(values, true, context.stdoutIsTTY)),
    project: (runtime, payload, values) => runtime.projectTaskStart(payload, values.full ?? false),
    format: (runtime, payload) => runtime.formatTaskQuick(payload),
  },
  "task audit": {
    options: SCOPE, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [taskId]) => runtime.taskAudit(root, taskId, scope(values)),
    format: (runtime, payload, _values, [taskId]) => runtime.formatTaskAudit(payload, taskId),
  },
  "task abandon": {
    options: SCOPE, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [taskId]) => runtime.taskAbandon(root, taskId, scope(values)),
    payload: (result) => result.payload,
    format: (runtime, payload, _values, [taskId]) => runtime.formatTaskAbandon(payload, taskId),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "task finish": {
    options: { ...SCOPE, message: STRING, "sync-plan-markdown": BOOLEAN, full: BOOLEAN },
    positional: ["TASK_ID"],
    invoke: (runtime, root, values, [taskId]) => runtime.taskFinish(root, taskId, {
      ...scope(values), message: values.message, syncPlanMarkdown: values["sync-plan-markdown"],
    }),
    project: (runtime, payload, values) => runtime.projectTaskFinish(payload, values.full ?? false),
    format: (runtime, payload, _values, [requested]) => runtime.formatTaskFinish(payload, requested),
    exitCode: (runtime, payload) => runtime.taskFinishExitCode(payload),
  },
  "task show": {
    options: SCOPE, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [taskId]) =>
      runtime.taskWorkflowTaskShow(root, taskId, scope(values)),
    format: (runtime, payload) => runtime.formatTaskShow(payload),
  },
  "change create": {
    options: { ...SCOPE, title: STRING, "base-line": STRING, "fork-snapshot": STRING },
    positional: ["TASK_ID"],
    validate: (values) => requireOption(values, "title"),
    invoke: (runtime, root, values, [taskId]) => runtime.changeCreate(root, taskId, values.title, {
      ...scope(values), baseLine: values["base-line"], forkSnapshot: values["fork-snapshot"],
    }),
    format: (runtime, payload) => runtime.formatChangeCreate(payload),
  },
  "change list": {
    options: { ...SCOPE, all: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowChangeList(root, scope(values)),
    project: (runtime, payload, values) => runtime.projectChangeList(payload, values.all ?? false),
    format: (runtime, payload, values) => runtime.formatChangeList(payload, {
      ...scope(values), all: values.all ?? false,
    }),
  },
  "change show": {
    options: SCOPE, positional: ["TASK_ID[/C-##]"],
    invoke: (runtime, root, values, [reference]) => runtime.changeShow(root, reference, scope(values)),
    format: (runtime, payload) => runtime.formatChangeShow(payload),
  },
  "change revert": {
    options: { ...SCOPE, force: BOOLEAN, "dry-run": BOOLEAN }, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.changeRevert(root, reference, {
      ...scope(values), force: values.force, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatChangeRevert(payload),
  },
  "change replay": {
    options: { ...SCOPE, onto: STRING, force: BOOLEAN, "dry-run": BOOLEAN }, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.changeReplay(root, reference, values.onto, {
      ...scope(values), force: values.force, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatChangeReplay(payload),
  },
  "change close": {
    options: SCOPE, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.changeClose(root, reference, scope(values)),
    format: (runtime, payload) => runtime.formatChangeClose(payload),
  },
  "change publish": {
    options: { remote: STRING }, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.changePublish(root, reference, values.remote),
    format: (runtime, payload) => runtime.formatChangePublish(payload),
  },
  "patchset publish": {
    options: { remote: STRING, summary: STRING, "author-mode": STRING }, positional: ["TASK_ID"],
    validate: (values) => {
      requireOption(values, "summary");
      validateChoice(values["author-mode"], "--author-mode", AUTHOR_MODES);
    },
    invoke: (runtime, root, values, [reference]) => runtime.patchsetPublish(root, reference, values.summary, {
      authorMode: values["author-mode"], remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatPatchsetPublish(payload),
  },
  "patchset list": {
    options: { remote: STRING }, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.patchsetList(root, reference, values.remote),
    format: (runtime, payload) => runtime.formatPatchsetList(payload),
  },
  "patchset show": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPatchsetShow(root, reference, {
      remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatPatchsetShow(payload),
  },
  "patchset select": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPatchsetSelect(root, reference, {
      remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatPatchsetSelect(payload),
  },
  "patchset ci-status": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPatchsetCiStatus(root, reference, {
      remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatPatchsetCiStatus(payload),
  },
  "patchset rerun-ci": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPatchsetRerunCi(root, reference, {
      remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatPatchsetRerunCi(payload),
  },
  "worktree status": {
    options: { snapshot: STRING, line: STRING, verbose: BOOLEAN },
    positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.taskWorkflowWorktreeStatus(root, {
      name, snapshotId: values.snapshot, lineName: values.line,
    }),
    format: (runtime, payload, values) => runtime.formatWorktreeStatus(payload, values.verbose ?? false),
  },
  "worktree show": {
    options: {}, positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, _values, [name]) => runtime.worktreeShow(root, name),
    format: (runtime, payload) => runtime.formatWorktreeShow(payload),
  },
  "worktree path": {
    options: { shell: BOOLEAN }, positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, _values, [name]) => runtime.worktreePath(root, name),
    project: (runtime, payload) => runtime.projectWorktreePath(payload),
    format: (runtime, payload, values) => runtime.formatWorktreePath(payload, values.shell ?? false),
  },
  "worktree restore": {
    options: { snapshot: STRING, line: STRING, path: { ...STRING, multiple: true }, force: BOOLEAN, "dry-run": BOOLEAN },
    positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.worktreeRestore(root, {
      name, snapshotId: values.snapshot, lineName: values.line, paths: values.path ?? [],
      force: values.force, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatWorktreeRestore(payload),
  },
  "worktree list": {
    options: { refresh: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowWorktreeList(root, { refreshStatus: values.refresh ?? false }),
    format: (runtime, payload) => runtime.formatWorktreeList(payload),
  },
  "worktree doctor": {
    options: { refresh: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowWorktreeDoctor(root, { refreshStatus: values.refresh ?? false }),
    format: (runtime, payload) => runtime.formatWorktreeDoctor(payload),
  },
  "worktree cleanup-candidates": {
    options: { "older-than": STRING, policy: STRING, "allow-manual-only": BOOLEAN, "include-protected": BOOLEAN },
    positional: [],
    invoke: (runtime, root, values) => runtime.taskWorkflowWorktreeCleanupCandidates(root, {
      olderThan: values["older-than"] ?? "7d", cleanupPolicy: values.policy,
      allowManualOnly: values["allow-manual-only"], includeProtected: values["include-protected"],
    }),
    format: (runtime, payload) => runtime.formatWorktreeCleanupCandidates(payload),
  },
  "worktree cleanup": {
    options: { "older-than": STRING, policy: STRING, "allow-manual-only": BOOLEAN, limit: STRING, "dry-run": BOOLEAN, yes: BOOLEAN },
    positional: [],
    invoke: (runtime, root, values) => runtime.worktreeCleanup(root, {
      olderThan: values["older-than"] ?? "7d", cleanupPolicy: values.policy,
      allowManualOnly: values["allow-manual-only"], limit: nativeSize(values.limit, "--limit"),
      dryRun: values["dry-run"], yes: values.yes,
    }),
    format: (runtime, payload) => runtime.formatWorktreeCleanup(payload),
  },
  "worktree prune-stale": {
    options: { "dry-run": BOOLEAN, yes: BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.worktreePruneStale(root, { dryRun: values["dry-run"], yes: values.yes }),
    format: (runtime, payload) => runtime.formatWorktreePruneStale(payload),
  },
  "worktree sync": {
    options: { all: BOOLEAN, line: STRING, force: BOOLEAN, "dry-run": BOOLEAN },
    positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.worktreeSync(root, {
      name, allWorktrees: values.all, lineName: values.line, force: values.force, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatWorktreeSync(payload),
    exitCode: (runtime, payload, _operation, values) => runtime.worktreeSyncExitCode(payload, values.all ?? false),
  },
  "worktree recreate": {
    options: { "dry-run": BOOLEAN }, positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.worktreeRecreate(root, { name, dryRun: values["dry-run"] }),
    format: (runtime, payload) => runtime.formatWorktreeRecreate(payload),
  },
  "worktree recover-task": {
    options: { change: STRING, remote: STRING, "dry-run": BOOLEAN }, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [taskId]) => runtime.worktreeRecoverTask(root, taskId, {
      changeRef: values.change, remoteName: values.remote, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatWorktreeRecoverTask(payload),
  },
  "worktree restore-owned-head": {
    options: { "dry-run": BOOLEAN }, positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.worktreeRestoreOwnedHead(root, { name, dryRun: values["dry-run"] }),
    format: (runtime, payload) => runtime.formatWorktreeRestoreOwnedHead(payload),
  },
  "worktree rebase": {
    options: { onto: STRING, continue: BOOLEAN, abort: BOOLEAN, "dry-run": BOOLEAN },
    positional: ["NAME"], optionalPositionals: 1,
    invoke: (runtime, root, values, [name]) => runtime.worktreeRebase(root, {
      name, ontoLine: values.onto, continueRebase: values.continue,
      abortRebase: values.abort, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatWorktreeRebase(payload),
  },
  "worktree remove": {
    options: { "all-stale": BOOLEAN, "delete-path": BOOLEAN, force: BOOLEAN, "dry-run": BOOLEAN, yes: BOOLEAN },
    positional: ["NAME"], optionalPositionals: 1, variadic: true,
    invoke: (runtime, root, values, names) => runtime.worktreeRemove(root, names, {
      allStale: values["all-stale"], deletePath: values["delete-path"], force: values.force,
      dryRun: values["dry-run"], yes: values.yes,
    }),
    format: (runtime, payload, values, names) => runtime.formatWorktreeRemove(payload,
      values["all-stale"] ?? false, values["dry-run"] ?? false, names.length > 1),
  },
  "review show": {
    options: { remote: STRING }, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewShow(root, reference, values.remote),
    project: (runtime, payload) => runtime.projectReviewShow(payload),
    format: (runtime, payload) => runtime.formatReviewShow(payload),
  },
  "review team request": {
    options: { patchset: STRING, note: STRING, remote: STRING, group: { ...STRING, multiple: true } },
    positional: ["TASK_ID"],
    validate: (values) => requireOption(values, "group"),
    invoke: (runtime, root, values, [reference]) => runtime.reviewTeamRequest(root, reference, values.group, {
      patchsetId: values.patchset, note: values.note, remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatReviewTeamRequest(payload),
  },
  "review team approve": {
    options: REVIEW_TEAM_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTeamApprove(root, reference, reviewTeamOptions(runtime, values)),
    format: (runtime, payload) => runtime.formatReviewTeamApprove(payload),
  },
  "review team request-changes": {
    options: REVIEW_TEAM_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTeamRequestChanges(root, reference, reviewTeamOptions(runtime, values)),
    format: (runtime, payload) => runtime.formatReviewTeamRequestChanges(payload),
  },
  "review team comment": {
    options: REVIEW_TEAM_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTeamComment(root, reference, reviewTeamOptions(runtime, values)),
    format: (runtime, payload) => runtime.formatReviewTeamComment(payload),
  },
  "review team defer": {
    options: REVIEW_TEAM_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTeamDefer(root, reference, reviewTeamOptions(runtime, values)),
    format: (runtime, payload) => runtime.formatReviewTeamDefer(payload),
  },
  "review task approve": {
    options: REVIEW_TASK_OPTIONS, positional: ["TASK_ID"],
    validate: (values) => { requireOption(values, "patchset"); requireOption(values, "message"); },
    invoke: (runtime, root, values, [reference]) => runtime.reviewTaskApprove(root, reference,
      values.patchset, values.message, values.remote),
    project: (runtime, operation) => runtime.projectReviewTaskOperation(operation),
    format: (runtime, operation) => runtime.formatReviewTaskApprove(operation),
  },
  "review task request-changes": {
    options: REVIEW_TASK_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTaskRequestChanges(root, reference, reviewTaskOptions(runtime, values)),
    project: (runtime, operation) => runtime.projectReviewTaskOperation(operation),
    format: (runtime, operation) => runtime.formatReviewTaskRequestChanges(operation),
  },
  "review task comment": {
    options: REVIEW_TASK_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTaskComment(root, reference, reviewTaskOptions(runtime, values)),
    project: (runtime, operation) => runtime.projectReviewTaskOperation(operation),
    format: (runtime, operation) => runtime.formatReviewTaskComment(operation),
  },
  "review task defer": {
    options: REVIEW_TASK_OPTIONS, positional: ["TASK_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.reviewTaskDefer(root, reference, reviewTaskOptions(runtime, values)),
    project: (runtime, operation) => runtime.projectReviewTaskOperation(operation),
    format: (runtime, operation) => runtime.formatReviewTaskDefer(operation),
  },
  "review code submit": {
    options: { patchset: STRING, message: STRING, reviewer: STRING, remote: STRING }, positional: ["TASK_ID"],
    validate: (values) => { requireOption(values, "patchset"); requireOption(values, "message"); },
    invoke: (runtime, root, values, [reference]) => runtime.reviewCodeSubmit(root, reference,
      values.patchset, values.message, {
        reviewer: values.reviewer, remoteName: values.remote,
      }),
    format: (runtime, payload) => runtime.formatReviewCodeSubmit(payload),
  },
  "review code template": {
    options: { style: STRING }, positional: [], discoverRepository: true,
    validate: (values) => validateChoice(values.style, "--style", ["inline", "numbered"]),
    invoke: (runtime, _root, values) => runtime.taskWorkflowReviewCodeTemplate({ style: values.style }),
    format: (runtime, payload) => runtime.formatReviewCodeTemplate(payload),
  },
  "attest put": {
    options: {
      tests: STRING, lint: STRING, security: STRING, license: STRING,
      "author-mode": STRING, model: STRING, remote: STRING,
    },
    positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowAttestPut(root, {
      patchsetId: reference,
      tests: values.tests, lint: values.lint, security: values.security, license: values.license,
      authorMode: values["author-mode"], model: values.model, remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatAttestPut(payload),
  },
  "attest show": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowAttestShow(root, reference, {
      remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatAttestShow(payload),
  },
  "policy eval": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPolicyEval(root, reference, {
      remoteName: values.remote,
    }),
    format: (runtime, payload) => runtime.formatPolicyEval(payload),
  },
  "policy show": {
    options: { remote: STRING }, positional: ["PATCHSET_ID"],
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPolicyShow(root, reference, {
      remoteName: values.remote,
    }),
    project: (runtime, payload) => runtime.projectPolicyShow(payload),
    format: (runtime, payload) => runtime.formatPolicyShow(payload),
  },
  "policy waive": {
    options: { rule: STRING, reason: STRING, "expires-at": STRING, remote: STRING },
    positional: ["PATCHSET_ID"],
    validate: (values) => { requireOption(values, "rule"); requireOption(values, "reason"); },
    invoke: (runtime, root, values, [reference]) => runtime.taskWorkflowPolicyWaive(root,
      reference, values.rule, values.reason, {
        expiresAt: values["expires-at"], remoteName: values.remote,
      }),
    format: (runtime, payload) => runtime.formatPolicyWaive(payload),
  },
  "tag create": {
    options: { snapshot: STRING, message: STRING }, positional: ["NAME"],
    validate: (values) => requireOption(values, "message"),
    invoke: (runtime, root, values, [name]) => runtime.tagCreate(root, name, values.message, values.snapshot),
    format: (runtime, payload) => runtime.formatTagCreate(payload),
  },
  "tag list": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.tagList(root),
    format: (runtime, payload) => runtime.formatTagList(payload),
  },
  "tag show": {
    options: {}, positional: ["NAME"],
    invoke: (runtime, root, _values, [name]) => runtime.tagShow(root, name),
    format: (runtime, payload) => runtime.formatTagShow(payload),
  },
  "tag delete": {
    options: {}, positional: ["NAME"],
    invoke: (runtime, root, _values, [name]) => runtime.tagDelete(root, name),
    format: (runtime, payload) => runtime.formatTagDelete(payload),
  },
  "stash save": {
    options: { message: STRING, "keep-workspace": BOOLEAN }, positional: [],
    invoke: (runtime, root, values) => runtime.stashSave(root, {
      message: values.message, keepWorkspace: values["keep-workspace"],
    }),
    format: (runtime, payload) => runtime.formatStashSave(payload),
  },
  "stash list": {
    options: {}, positional: [],
    invoke: (runtime, root) => runtime.taskWorkflowStashList(root),
    format: (runtime, payload) => runtime.formatStashList(payload),
  },
  "stash show": {
    options: {}, positional: ["STASH_ID"],
    invoke: (runtime, root, _values, [stashId]) => runtime.taskWorkflowStashShow(root, stashId),
    format: (runtime, payload) => runtime.formatStashShow(payload),
  },
  "stash apply": {
    options: { force: BOOLEAN }, positional: ["STASH_ID"],
    invoke: (runtime, root, values, [stashId]) => runtime.stashApply(root, stashId, values.force),
    format: (runtime, payload) => runtime.formatStashApply(payload),
  },
  "stash pop": {
    options: { force: BOOLEAN }, positional: ["STASH_ID"],
    invoke: (runtime, root, values, [stashId]) => runtime.stashPop(root, stashId, values.force),
    format: (runtime, payload) => runtime.formatStashPop(payload),
  },
  "stash drop": {
    options: {}, positional: ["STASH_ID"],
    invoke: (runtime, root, _values, [stashId]) => runtime.stashDrop(root, stashId),
    format: (runtime, payload) => runtime.formatStashDrop(payload),
  },
  "snapshot list": {
    options: { all: BOOLEAN }, positional: [],
    invoke: (runtime, root) => runtime.taskWorkflowSnapshotList(root),
    project: (runtime, payload, values) => runtime.projectSnapshotList(payload, values.all ?? false),
    format: (runtime, payload, values) => runtime.formatSnapshotList(payload, values.all ?? false),
  },
  "snapshot diff": {
    options: { "include-text": BOOLEAN, "max-bytes": STRING },
    positional: ["OLD_SNAPSHOT_OR_TAG", "NEW_SNAPSHOT_OR_TAG"],
    invoke: (runtime, root, values, [older, newer]) => {
      if (values["max-bytes"] !== undefined && !values["include-text"]) {
        throw new TerminalUsageError("--max-bytes requires --include-text");
      }
      return runtime.snapshotDiff(root, older, newer, {
        includeText: values["include-text"], maxBytes: positiveNativeSize(values["max-bytes"], "--max-bytes"),
      });
    },
    format: (runtime, payload) => runtime.formatSnapshotDiff(payload),
  },
  "snapshot revert": {
    options: { force: BOOLEAN, "dry-run": BOOLEAN }, positional: ["SNAPSHOT_OR_TAG"],
    invoke: (runtime, root, values, [selector]) => runtime.snapshotRevert(root, selector, {
      force: values.force, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatSnapshotRevert(payload),
  },
  "snapshot replay": {
    options: { onto: STRING, force: BOOLEAN, "dry-run": BOOLEAN }, positional: ["SNAPSHOT_OR_TAG"],
    invoke: (runtime, root, values, [selector]) => runtime.snapshotReplay(root, selector, values.onto, {
      force: values.force, dryRun: values["dry-run"],
    }),
    format: (runtime, payload) => runtime.formatSnapshotReplay(payload),
  },
  "snapshot restore-lines": {
    options: { line: STRING, start: STRING, end: STRING, yes: BOOLEAN },
    positional: ["SNAPSHOT_ID", "PATH"],
    invoke: (runtime, root, values, [snapshotId, path]) => {
      if (values.line !== undefined && (values.start !== undefined || values.end !== undefined)) {
        throw new TerminalUsageError("--line conflicts with --start and --end");
      }
      if ((values.start === undefined) !== (values.end === undefined)) {
        throw new TerminalUsageError("--start and --end require each other");
      }
      return runtime.taskWorkflowSnapshotRestoreLines(root, snapshotId, path, {
        line: positiveNativeSize(values.line, "--line"),
        startLine: positiveNativeSize(values.start, "--start"),
        endLine: positiveNativeSize(values.end, "--end"), yes: values.yes,
      });
    },
    format: (runtime, payload) => runtime.formatSnapshotRestoreLines(payload),
  },
  "snapshot ancestry": {
    options: {
      ancestors: BOOLEAN, descendants: BOOLEAN, "first-parent": BOOLEAN,
      "max-depth": STRING, limit: STRING, all: BOOLEAN,
    },
    positional: ["SNAPSHOT_OR_TAG"],
    queryError: true,
    invoke: (runtime, root, values, [selector]) => {
      if (values.ancestors && values.descendants) {
        throw new TerminalUsageError("--ancestors conflicts with --descendants");
      }
      return runtime.snapshotAncestry(root, selector, {
        descendants: values.descendants, firstParent: values["first-parent"],
        maxDepth: positiveNativeSize(values["max-depth"], "--max-depth"),
        limit: positiveNativeSize(values.limit, "--limit"),
      });
    },
    format: (runtime, payload, values) => runtime.formatSnapshotAncestry(payload, values.all ?? false),
  },
  "snapshot is-ancestor": {
    options: {}, positional: ["OLDER_SNAPSHOT_OR_TAG", "NEWER_SNAPSHOT_OR_TAG"],
    queryError: true,
    invoke: (runtime, root, _values, [older, newer]) => runtime.snapshotIsAncestor(root, older, newer),
    payload: (result) => result.payload,
    format: (runtime, payload) => runtime.formatSnapshotIsAncestor(payload),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "snapshot merge-base": {
    options: { all: BOOLEAN }, positional: ["LEFT_SNAPSHOT_OR_TAG", "RIGHT_SNAPSHOT_OR_TAG"],
    queryError: true,
    invoke: (runtime, root, values, [left, right]) => runtime.snapshotMergeBase(root, left, right, values.all ?? false),
    payload: (result) => result.payload,
    format: (runtime, payload) => runtime.formatSnapshotMergeBase(payload),
    exitCode: (_runtime, _payload, result) => result.exitCode,
  },
  "snapshot show": {
    options: { files: BOOLEAN }, positional: ["SNAPSHOT_OR_TAG"],
    invoke: (runtime, root, values, [selector]) => runtime.inspectSnapshot(root, selector, !values.json),
    payload: (inspection) => inspection.payload,
    format: (runtime, _payload, values, _positionals, _context, inspection) =>
      runtime.formatSnapshotShow(inspection, values.files ?? false),
  },
  "snapshot create": CAPTURE,
  commit: CAPTURE,
});

// Frozen scalar syntax belongs to the frontend; values never dispatch a Rust command.
const SCALAR_ARGUMENTS = Object.freeze({
  "binary-db upgrade-u64-seconds": {
    "repo-root": {"kind": "path", "label": "--repo-root <REPO_ROOT>"},
    "generation-root": {"kind": "path", "label": "--generation-root <GENERATION_ROOT>"},
  },
  "binary-db agent-pool50": {
    "repo-root": {"kind": "path", "label": "--repo-root <REPO_ROOT>"},
    "generation-root": {"kind": "path", "label": "--generation-root <GENERATION_ROOT>"},
  },
  "line cleanup": {
    "limit": {"kind": "usize", "label": "--limit <LIMIT>"},
  },
  "git migrate": {
    "staging-root": {"kind": "path", "label": "--staging-root <PATH>"},
  },
  "git restore": {
    "target": {"kind": "path", "label": "--target <TARGET>"},
  },
  "git mirror": {
    "direction": {"kind": "choice", "label": "--direction <DIRECTION>", "choices": ["inbound", "outbound", "bidirectional"]},
  },
  "blame": {
    "line": {"kind": "line", "label": "--line <LINE>"},
    "start": {"kind": "line", "label": "--start <START_LINE>"},
    "end": {"kind": "line", "label": "--end <END_LINE>"},
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "remote recover-head": {
    "jobs": {"kind": "jobs", "label": "--jobs <JOBS>"},
  },
  "current-source-cache contract": {
    "namespace-root": {"kind": "path", "label": "--namespace-root <NAMESPACE_ROOT>"},
    "core-repo-root": {"kind": "path", "label": "--core-repo-root <CORE_REPO_ROOT>"},
  },
  "current-source-cache mark-building": {
    "namespace-root": {"kind": "path", "label": "--namespace-root <NAMESPACE_ROOT>"},
    "core-repo-root": {"kind": "path", "label": "--core-repo-root <CORE_REPO_ROOT>"},
    "source-mtime-ns": {"kind": "u64", "label": "--source-mtime-ns <SOURCE_MTIME_NS>"},
  },
  "current-source-cache activate": {
    "namespace-root": {"kind": "path", "label": "--namespace-root <NAMESPACE_ROOT>"},
    "core-repo-root": {"kind": "path", "label": "--core-repo-root <CORE_REPO_ROOT>"},
    "source-mtime-ns": {"kind": "u64", "label": "--source-mtime-ns <SOURCE_MTIME_NS>"},
    "owner-pid": {"kind": "u32", "label": "--owner-pid <OWNER_PID>"},
  },
  "current-source-cache release-lease": {
    "lease-path": {"kind": "path", "label": "--lease-path <LEASE_PATH>"},
    "namespace-root": {"kind": "path", "label": "--namespace-root <NAMESPACE_ROOT>"},
  },
  "current-source-cache prune": {
    "namespace-root": {"kind": "path", "label": "--namespace-root <NAMESPACE_ROOT>"},
    "now": {"kind": "f64", "label": "--now <NOW>"},
    "idle-ttl-seconds": {"kind": "u64", "label": "--idle-ttl-seconds <IDLE_TTL_SECONDS>"},
    "build-stale-seconds": {"kind": "u64", "label": "--build-stale-seconds <BUILD_STALE_SECONDS>"},
    "max-bytes": {"kind": "u64", "label": "--max-bytes <MAX_BYTES>"},
  },
  "current-source-cache seed-canonical": {
    "namespace-root": {"kind": "path", "label": "--namespace-root <NAMESPACE_ROOT>"},
    "core-repo-root": {"kind": "path", "label": "--core-repo-root <CORE_REPO_ROOT>"},
    "repo-root": {"kind": "path", "label": "--repo-root <REPO_ROOT>"},
    "canonical-repo-root": {"kind": "path", "label": "--canonical-repo-root <CANONICAL_REPO_ROOT>"},
    "core-source-mtime-ns": {"kind": "u64", "label": "--core-source-mtime-ns <CORE_SOURCE_MTIME_NS>"},
  },
  "current-source-cache extension-fresh": {
    "metadata-path": {"kind": "path", "label": "--metadata-path <METADATA_PATH>"},
    "extension-path": {"kind": "path", "label": "--extension-path <EXTENSION_PATH>"},
    "source-mtime-ns": {"kind": "u64", "label": "--source-mtime-ns <SOURCE_MTIME_NS>"},
  },
  "current-source-cache binary-fresh": {
    "metadata-path": {"kind": "path", "label": "--metadata-path <METADATA_PATH>"},
    "binary-path": {"kind": "path", "label": "--binary-path <BINARY_PATH>"},
    "source-mtime-ns": {"kind": "u64", "label": "--source-mtime-ns <SOURCE_MTIME_NS>"},
  },
  "current-source-cache core-fingerprint": {
    "#0": {"kind": "path", "label": "<REPO_ROOT>"},
  },
  "current-source-cache server-fingerprint": {
    "#0": {"kind": "path", "label": "<REPO_ROOT>"},
  },
  "current-source-cache core-mtime": {
    "#0": {"kind": "path", "label": "<REPO_ROOT>"},
  },
  "current-source-cache server-mtime": {
    "#0": {"kind": "path", "label": "<REPO_ROOT>"},
  },
  "release candidate create": {
    "channel": {"kind": "choice", "label": "--channel <CHANNEL>", "choices": ["rc", "stable"]},
    "public-source-root": {"kind": "path", "label": "--public-source-root <DIR>"},
  },
  "release check": {
    "receipts": {"kind": "path", "label": "--receipts <DIR>"},
    "public-source-root": {"kind": "path", "label": "--public-source-root <DIR>"},
  },
  "release build": {
    "receipts": {"kind": "path", "label": "--receipts <DIR>"},
    "public-source-root": {"kind": "path", "label": "--public-source-root <DIR>"},
    "native-matrix-dir": {"kind": "path", "label": "--native-matrix-dir <DIR>"},
    "native-command-dir": {"kind": "path", "label": "--native-command-dir <DIR>"},
  },
  "release native-source": {
    "source-dir": {"kind": "path", "label": "--source-dir <DIR>"},
    "rustc-path": {"kind": "path", "label": "--rustc-path <PATH>"},
  },
  "release native-bundle": {
    "native-matrix-dir": {"kind": "path", "label": "--native-matrix-dir <DIR>"},
  },
  "release package": {
    "channel": {"kind": "choice", "label": "--channel <CHANNEL>", "choices": ["homebrew", "apt", "winget", "pypi", "npm"]},
    "public-source-root": {"kind": "path", "label": "--public-source-root <DIR>"},
  },
  "release show": {
    "public-source-root": {"kind": "path", "label": "--public-source-root <DIR>"},
  },
  "release promote": {
    "channel": {"kind": "choice", "label": "--channel <CHANNEL>", "choices": ["rc", "stable"]},
    "public-source-root": {"kind": "path", "label": "--public-source-root <DIR>"},
  },
  "repo jobs": {
    "worker-job-index": {"kind": "u32", "label": "--worker-job-index <INDEX>"},
    "state": {"kind": "choice", "label": "--state <STATE>", "choices": ["queued", "running", "succeeded", "failed"]},
    "limit": {"kind": "jobLimit", "label": "--limit <COUNT>"},
  },
  "config set": {
    "agent-teams": {"kind": "choice", "label": "--agent-teams <AGENT_TEAMS>", "choices": ["enabled", "disabled"]},
    "plan-style": {"kind": "choice", "label": "--plan-style <PLAN_STYLE>", "choices": ["concise", "detailed"]},
    "sprint": {"kind": "choice", "label": "--sprint <SPRINT>", "choices": ["on", "off"]},
    "default-author-mode": {"kind": "choice", "label": "--default-author-mode <DEFAULT_AUTHOR_MODE>", "choices": ["human_only", "human_with_ai_assist", "ai_with_human_review", "ai_only_experimental"]},
    "task-review": {"kind": "choice", "label": "--task-review <TASK_REVIEW>", "choices": ["required", "automatic"]},
    "task-worktree-main-seed-ram-max-bytes": {"kind": "i64", "label": "--task-worktree-main-seed-ram-max-bytes <TASK_WORKTREE_MAIN_SEED_RAM_MAX_BYTES>"},
  },
  "config unset": {
    "#0": {"kind": "choice", "label": "<KEY>", "choices": ["plan-language", "plan-style", "default-author-mode", "default-model", "task-review", "task-worktree-alias-root", "task-worktree-main-seed-ram-max-bytes", "id-namespace-prefix", "user-name", "user-email", "post-finish-hooks"]},
  },
  "team session ensure": {
    "agent": {"kind": "u32", "label": "--agent <AGENT>"},
    "role": {"kind": "u32", "label": "--role <ROLE>"},
  },
  "plan sync": {
    "#0": {"kind": "path", "label": "<TARGET>"},
  },
  "task start": {
    "edit-root": {"kind": "path", "label": "--edit-root <ABSOLUTE_PATH>"},
  },
  "task quick": {
    "edit-root": {"kind": "path", "label": "--edit-root <ABSOLUTE_PATH>"},
  },
  "snapshot create": {
    "#0": {"kind": "authoring", "label": "<TASK_ID>"},
  },
  "snapshot diff": {
    "max-bytes": {"kind": "positive", "label": "--max-bytes <BYTES>"},
  },
  "snapshot restore-lines": {
    "line": {"kind": "line", "label": "--line <LINE>"},
    "start": {"kind": "line", "label": "--start <START_LINE>"},
    "end": {"kind": "line", "label": "--end <END_LINE>"},
  },
  "snapshot ancestry": {
    "max-depth": {"kind": "positive", "label": "--max-depth <DEPTH>"},
    "limit": {"kind": "positive", "label": "--limit <COUNT>"},
  },
  "commit": {
    "#0": {"kind": "authoring", "label": "<TASK_ID>"},
  },
  "patchset publish": {
    "#0": {"kind": "work", "label": "<TASK_ID>"},
    "author-mode": {"kind": "choice", "label": "--author-mode <MODE>", "choices": ["human_only", "human_with_ai_assist", "ai_with_human_review", "ai_only_experimental"]},
  },
  "patchset list": {
    "#0": {"kind": "work", "label": "<TASK_ID>"},
  },
  "patchset show": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "patchset select": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "patchset ci-status": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "patchset rerun-ci": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "review team request": {
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "review team approve": {
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "review team request-changes": {
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "review team comment": {
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "review team defer": {
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "review task approve": {
    "patchset": {"kind": "review", "label": "--patchset <PATCHSET_ID>"},
  },
  "review task request-changes": {
    "patchset": {"kind": "review", "label": "--patchset <PATCHSET_ID>"},
  },
  "review task comment": {
    "patchset": {"kind": "review", "label": "--patchset <PATCHSET_ID>"},
  },
  "review task defer": {
    "patchset": {"kind": "review", "label": "--patchset <PATCHSET_ID>"},
  },
  "review code submit": {
    "patchset": {"kind": "patchset", "label": "--patchset <PATCHSET_ID>"},
  },
  "review code template": {
    "style": {"kind": "choice", "label": "--style <STYLE>", "choices": ["inline", "numbered"]},
  },
  "attest put": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "attest show": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "policy eval": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "policy show": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "policy waive": {
    "#0": {"kind": "patchset", "label": "<PATCHSET_ID>"},
  },
  "worktree cleanup": {
    "limit": {"kind": "usize", "label": "--limit <COUNT>"},
  },
  "workflow reconcile": {
    "limit": {"kind": "usize", "label": "--limit <LIMIT>"},
  },
  "workflow ready": {
    "author-mode": {"kind": "choice", "label": "--author-mode <AUTHOR_MODE>", "choices": ["human_only", "human_with_ai_assist", "ai_with_human_review", "ai_only_experimental"]},
    "expected-identity": {"kind": "path", "label": "--expected-identity <PATH>"},
  },
  "workflow finish": {
    "expected-identity": {"kind": "path", "label": "--expected-identity <PATH>"},
  },
});

// Required arguments and relationships from the same frozen frontend grammar.
const COMMAND_SYNTAX = Object.freeze({
  "init": {"args": [{"key": "policy-profile", "label": "--policy-profile <POLICY_PROFILE>"}, {"key": "repair-existing", "label": "--repair-existing"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@InitArgs", "members": ["policy-profile", "repair-existing", "json"]}]},
  "binary-db upgrade-u64-seconds": {"args": [{"key": "repo-root", "label": "--repo-root <REPO_ROOT>"}, {"key": "generation-root", "label": "--generation-root <GENERATION_ROOT>", "required": true}, {"key": "source-time-width", "label": "--source-time-width <SOURCE_TIME_WIDTH>", "required": true}, {"key": "activate", "label": "--activate"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@BinaryDbUpgradeU64SecondsArgs", "members": ["repo-root", "generation-root", "source-time-width", "activate", "json"]}]},
  "binary-db agent-pool50": {"args": [{"key": "repo-root", "label": "--repo-root <REPO_ROOT>"}, {"key": "generation-root", "label": "--generation-root <GENERATION_ROOT>", "required": true}, {"key": "activate", "label": "--activate"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@BinaryDbAgentPool50Args", "members": ["repo-root", "generation-root", "activate", "json"]}]},
  "line list": {"args": [{"key": "all", "label": "--all", "conflicts": ["archived"]}, {"key": "archived", "label": "--archived", "conflicts": ["all"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineListArgs", "members": ["all", "archived", "remote", "json"]}]},
  "line create": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "from-snapshot", "label": "--from-snapshot <FROM_SNAPSHOT>"}, {"key": "switch", "label": "--switch"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineCreateArgs", "members": ["#0", "from-snapshot", "switch", "json"]}]},
  "line switch": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "restore", "label": "--restore"}, {"key": "force", "label": "--force", "requires": ["restore"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineSwitchArgs", "members": ["#0", "restore", "force", "json"]}]},
  "line show": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineShowArgs", "members": ["#0", "remote", "json"]}]},
  "line archive": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineArchiveArgs", "members": ["#0", "remote", "json"]}]},
  "line rename": {"args": [{"key": "#0", "label": "<OLD>", "required": true}, {"key": "#1", "label": "<NEW>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineRenameArgs", "members": ["#0", "#1", "remote", "json"]}]},
  "line delete": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "yes", "label": "--yes", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineDeleteArgs", "members": ["#0", "remote", "yes", "json"]}]},
  "line merge": {"args": [{"key": "#0", "label": "[SOURCE]", "conflicts": ["continue", "abort"]}, {"key": "message", "label": "--message <MESSAGE>", "conflicts": ["abort"]}, {"key": "continue", "label": "--continue", "conflicts": ["#0", "abort"]}, {"key": "abort", "label": "--abort", "conflicts": ["#0", "message", "continue"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineMergeArgs", "members": ["#0", "message", "continue", "abort", "json"]}]},
  "line cleanup": {"args": [{"key": "idle-for", "label": "--idle-for <IDLE_FOR>"}, {"key": "kind", "label": "--kind <CLEANUP_KIND>"}, {"key": "limit", "label": "--limit <LIMIT>"}, {"key": "include-protected", "label": "--include-protected"}, {"key": "all", "label": "--all"}, {"key": "yes", "label": "--yes"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@LineCleanupArgs", "members": ["idle-for", "kind", "limit", "include-protected", "all", "yes", "json"]}]},
  "git migrate": {"args": [{"key": "staging-root", "label": "--staging-root <PATH>", "required": true}, {"key": "apply", "label": "--apply"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GitMigrateArgs", "members": ["staging-root", "apply", "json"]}]},
  "git recover": {"args": [{"key": "apply", "label": "--apply"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GitRecoverArgs", "members": ["apply", "json"]}]},
  "git import": {"args": [{"key": "#0", "label": "<SOURCE>", "required": true}, {"key": "all-branches-and-tags", "label": "--all-branches-and-tags"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GitImportArgs", "members": ["#0", "all-branches-and-tags", "dry-run", "json"]}]},
  "git export": {"args": [{"key": "#0", "label": "<TARGET>", "required": true}, {"key": "generation", "label": "--generation <GENERATION>"}, {"key": "git-object", "label": "--git-object <GIT_OBJECT>", "requires": ["generation"]}, {"key": "all-lines-and-tags", "label": "--all-lines-and-tags"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GitExportArgs", "members": ["#0", "generation", "git-object", "all-lines-and-tags", "dry-run", "json"]}]},
  "git restore": {"args": [{"key": "generation", "label": "--generation <GENERATION>", "required": true}, {"key": "git-object", "label": "--git-object <GIT_OBJECT>", "required": true}, {"key": "target", "label": "--target <TARGET>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GitRestoreArgs", "members": ["generation", "git-object", "target", "json"]}]},
  "git mirror": {"args": [{"key": "#0", "label": "<ENDPOINT>", "required": true}, {"key": "direction", "label": "--direction <DIRECTION>", "required": true}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GitMirrorArgs", "members": ["#0", "direction", "dry-run", "json"]}]},
  "blame": {"args": [{"key": "#0", "label": "<PATH>", "required": true}, {"key": "line", "label": "--line <LINE>", "conflicts": ["start", "end"]}, {"key": "start", "label": "--start <START_LINE>", "requires": ["end"], "conflicts": ["line"]}, {"key": "end", "label": "--end <END_LINE>", "requires": ["start"], "conflicts": ["line"]}, {"key": "snapshot", "label": "--snapshot <SNAPSHOT_ID>", "conflicts": ["patchset", "plan-id", "plan-ref"]}, {"key": "via-parent", "label": "--via-parent <VIA_PARENT_SNAPSHOT_ID>", "requires": ["snapshot"]}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>", "conflicts": ["snapshot", "plan-id", "plan-ref"]}, {"key": "remote", "label": "--remote <REMOTE_NAME>", "requires": ["patchset"]}, {"key": "plan-id", "label": "--plan-id <PLAN_ID>", "conflicts": ["plan-ref", "snapshot", "patchset"]}, {"key": "plan-ref", "label": "--plan-ref <PLAN_REF>", "conflicts": ["plan-id", "snapshot", "patchset"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@BlameArgs", "members": ["#0", "line", "start", "end", "snapshot", "via-parent", "patchset", "remote", "plan-id", "plan-ref", "json"]}]},
  "doctor memory-root": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@DoctorMemoryRootArgs", "members": ["json"]}]},
  "doctor runtime-root": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@DoctorRuntimeRootArgs", "members": ["json"]}]},
  "doctor plan-authority": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@DoctorPlanAuthorityArgs", "members": ["json"]}]},
  "queue summary": {"args": [{"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@QueueSummaryArgs", "members": ["remote", "json"]}]},
  "remote add": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "#1", "label": "<URL>", "required": true}, {"key": "default", "label": "--default"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RemoteAddArgs", "members": ["#0", "#1", "default", "json"]}]},
  "remote list": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@RemoteListArgs", "members": ["json"]}]},
  "remote recover-head": {"args": [{"key": "remote", "label": "--remote <NAME>"}, {"key": "jobs", "label": "--jobs <JOBS>"}, {"key": "apply", "label": "--apply"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RemoteRecoverHeadArgs", "members": ["remote", "jobs", "apply", "json"]}]},
  "current-source-cache contract": {"args": [{"key": "namespace-root", "label": "--namespace-root <NAMESPACE_ROOT>", "required": true}, {"key": "core-repo-root", "label": "--core-repo-root <CORE_REPO_ROOT>", "required": true}, {"key": "ext-suffix", "label": "--ext-suffix <EXT_SUFFIX>", "required": true}, {"key": "rustflags", "label": "--rustflags <RUSTFLAGS>"}, {"key": "worker-id", "label": "--worker-id <WORKER_ID>"}, {"key": "core-source-fingerprint", "label": "--core-source-fingerprint <CORE_SOURCE_FINGERPRINT>"}, {"key": "server-source-fingerprint", "label": "--server-source-fingerprint <SERVER_SOURCE_FINGERPRINT>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceCacheContractArgs", "members": []}, {"key": "@CurrentSourceCachePathArgs", "members": ["namespace-root", "core-repo-root", "ext-suffix", "rustflags", "worker-id", "core-source-fingerprint", "server-source-fingerprint"]}]},
  "current-source-cache mark-building": {"args": [{"key": "namespace-root", "label": "--namespace-root <NAMESPACE_ROOT>", "required": true}, {"key": "core-repo-root", "label": "--core-repo-root <CORE_REPO_ROOT>", "required": true}, {"key": "ext-suffix", "label": "--ext-suffix <EXT_SUFFIX>", "required": true}, {"key": "rustflags", "label": "--rustflags <RUSTFLAGS>"}, {"key": "worker-id", "label": "--worker-id <WORKER_ID>"}, {"key": "core-source-fingerprint", "label": "--core-source-fingerprint <CORE_SOURCE_FINGERPRINT>"}, {"key": "server-source-fingerprint", "label": "--server-source-fingerprint <SERVER_SOURCE_FINGERPRINT>"}, {"key": "source-mtime-ns", "label": "--source-mtime-ns <SOURCE_MTIME_NS>", "required": true}, {"key": "extra-json", "label": "--extra-json <EXTRA_JSON>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceCacheStateArgs", "members": []}, {"key": "@CurrentSourceCachePathArgs", "members": ["namespace-root", "core-repo-root", "ext-suffix", "rustflags", "worker-id", "core-source-fingerprint", "server-source-fingerprint"]}]},
  "current-source-cache activate": {"args": [{"key": "namespace-root", "label": "--namespace-root <NAMESPACE_ROOT>", "required": true}, {"key": "core-repo-root", "label": "--core-repo-root <CORE_REPO_ROOT>", "required": true}, {"key": "ext-suffix", "label": "--ext-suffix <EXT_SUFFIX>", "required": true}, {"key": "rustflags", "label": "--rustflags <RUSTFLAGS>"}, {"key": "worker-id", "label": "--worker-id <WORKER_ID>"}, {"key": "core-source-fingerprint", "label": "--core-source-fingerprint <CORE_SOURCE_FINGERPRINT>"}, {"key": "server-source-fingerprint", "label": "--server-source-fingerprint <SERVER_SOURCE_FINGERPRINT>"}, {"key": "source-mtime-ns", "label": "--source-mtime-ns <SOURCE_MTIME_NS>", "required": true}, {"key": "extra-json", "label": "--extra-json <EXTRA_JSON>"}, {"key": "register-lease", "label": "--register-lease"}, {"key": "owner-pid", "label": "--owner-pid <OWNER_PID>", "requires": ["register-lease"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceCacheActivateArgs", "members": []}, {"key": "@CurrentSourceCachePathArgs", "members": ["namespace-root", "core-repo-root", "ext-suffix", "rustflags", "worker-id", "core-source-fingerprint", "server-source-fingerprint"]}]},
  "current-source-cache release-lease": {"args": [{"key": "lease-path", "label": "--lease-path <LEASE_PATH>", "required": true}, {"key": "namespace-root", "label": "--namespace-root <NAMESPACE_ROOT>", "required": true}, {"key": "remove-unleased-ready", "label": "--remove-unleased-ready"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceCacheReleaseLeaseArgs", "members": ["lease-path", "namespace-root", "remove-unleased-ready", "json"]}]},
  "current-source-cache prune": {"args": [{"key": "namespace-root", "label": "--namespace-root <NAMESPACE_ROOT>", "required": true}, {"key": "now", "label": "--now <NOW>"}, {"key": "idle-ttl-seconds", "label": "--idle-ttl-seconds <IDLE_TTL_SECONDS>"}, {"key": "build-stale-seconds", "label": "--build-stale-seconds <BUILD_STALE_SECONDS>"}, {"key": "max-bytes", "label": "--max-bytes <MAX_BYTES>"}, {"key": "remove-unleased-ready", "label": "--remove-unleased-ready"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceCachePruneArgs", "members": ["namespace-root", "now", "idle-ttl-seconds", "build-stale-seconds", "max-bytes", "remove-unleased-ready", "json"]}]},
  "current-source-cache seed-canonical": {"args": [{"key": "namespace-root", "label": "--namespace-root <NAMESPACE_ROOT>", "required": true}, {"key": "core-repo-root", "label": "--core-repo-root <CORE_REPO_ROOT>", "required": true}, {"key": "ext-suffix", "label": "--ext-suffix <EXT_SUFFIX>", "required": true}, {"key": "rustflags", "label": "--rustflags <RUSTFLAGS>"}, {"key": "worker-id", "label": "--worker-id <WORKER_ID>"}, {"key": "core-source-fingerprint", "label": "--core-source-fingerprint <CORE_SOURCE_FINGERPRINT>"}, {"key": "server-source-fingerprint", "label": "--server-source-fingerprint <SERVER_SOURCE_FINGERPRINT>"}, {"key": "repo-root", "label": "--repo-root <REPO_ROOT>", "required": true}, {"key": "canonical-repo-root", "label": "--canonical-repo-root <CANONICAL_REPO_ROOT>", "required": true}, {"key": "core-source-mtime-ns", "label": "--core-source-mtime-ns <CORE_SOURCE_MTIME_NS>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceCacheSeedCanonicalArgs", "members": []}, {"key": "@CurrentSourceCachePathArgs", "members": ["namespace-root", "core-repo-root", "ext-suffix", "rustflags", "worker-id", "core-source-fingerprint", "server-source-fingerprint"]}]},
  "current-source-cache extension-fresh": {"args": [{"key": "metadata-path", "label": "--metadata-path <METADATA_PATH>", "required": true}, {"key": "extension-path", "label": "--extension-path <EXTENSION_PATH>", "required": true}, {"key": "source-mtime-ns", "label": "--source-mtime-ns <SOURCE_MTIME_NS>", "required": true}, {"key": "source-fingerprint", "label": "--source-fingerprint <SOURCE_FINGERPRINT>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceExtensionFreshArgs", "members": ["metadata-path", "extension-path", "source-mtime-ns", "source-fingerprint", "json"]}]},
  "current-source-cache binary-fresh": {"args": [{"key": "metadata-path", "label": "--metadata-path <METADATA_PATH>", "required": true}, {"key": "binary-path", "label": "--binary-path <BINARY_PATH>", "required": true}, {"key": "metadata-fingerprint-key", "label": "--metadata-fingerprint-key <METADATA_FINGERPRINT_KEY>", "required": true}, {"key": "metadata-source-mtime-key", "label": "--metadata-source-mtime-key <METADATA_SOURCE_MTIME_KEY>", "required": true}, {"key": "metadata-mtime-key", "label": "--metadata-mtime-key <METADATA_MTIME_KEY>", "required": true}, {"key": "metadata-sha-key", "label": "--metadata-sha-key <METADATA_SHA_KEY>", "required": true}, {"key": "source-mtime-ns", "label": "--source-mtime-ns <SOURCE_MTIME_NS>", "required": true}, {"key": "source-fingerprint", "label": "--source-fingerprint <SOURCE_FINGERPRINT>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceBinaryFreshArgs", "members": ["metadata-path", "binary-path", "metadata-fingerprint-key", "metadata-source-mtime-key", "metadata-mtime-key", "metadata-sha-key", "source-mtime-ns", "source-fingerprint", "json"]}]},
  "current-source-cache core-fingerprint": {"args": [{"key": "#0", "label": "<REPO_ROOT>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceFingerprintArgs", "members": ["#0", "json"]}]},
  "current-source-cache server-fingerprint": {"args": [{"key": "#0", "label": "<REPO_ROOT>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceFingerprintArgs", "members": ["#0", "json"]}]},
  "current-source-cache core-mtime": {"args": [{"key": "#0", "label": "<REPO_ROOT>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceFingerprintArgs", "members": ["#0", "json"]}]},
  "current-source-cache server-mtime": {"args": [{"key": "#0", "label": "<REPO_ROOT>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CurrentSourceFingerprintArgs", "members": ["#0", "json"]}]},
  "release adapter check": {"args": [{"key": "version", "label": "--version <VERSION>", "required": true}, {"key": "line", "label": "--line <LINE_NAME>"}, {"key": "target", "label": "--target <TARGET>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseAdapterArgs", "members": ["version", "line", "target", "json"]}]},
  "release adapter build": {"args": [{"key": "version", "label": "--version <VERSION>", "required": true}, {"key": "line", "label": "--line <LINE_NAME>"}, {"key": "target", "label": "--target <TARGET>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseAdapterArgs", "members": ["version", "line", "target", "json"]}]},
  "release candidate create": {"args": [{"key": "version", "label": "--version <VERSION>", "required": true}, {"key": "line", "label": "--line <LINE_NAME>"}, {"key": "profile", "label": "--profile <PROFILE>"}, {"key": "channel", "label": "--channel <CHANNEL>"}, {"key": "public-source-root", "label": "--public-source-root <DIR>", "hidden": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseCandidateCreateArgs", "members": ["version", "line", "profile", "channel", "public-source-root", "json"]}]},
  "release check": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "receipts", "label": "--receipts <DIR>"}, {"key": "public-source-root", "label": "--public-source-root <DIR>", "hidden": true}, {"key": "tests-command", "label": "--tests-command <TESTS_COMMAND>"}, {"key": "skip-tests-reason", "label": "--skip-tests-reason <SKIP_TESTS_REASON>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseCheckArgs", "members": ["#0", "receipts", "public-source-root", "tests-command", "skip-tests-reason", "json"]}]},
  "release build": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "receipts", "label": "--receipts <DIR>"}, {"key": "public-source-root", "label": "--public-source-root <DIR>", "hidden": true}, {"key": "native-matrix-dir", "label": "--native-matrix-dir <DIR>"}, {"key": "native-command-dir", "label": "--native-command-dir <DIR>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseBuildArgs", "members": ["#0", "receipts", "public-source-root", "native-matrix-dir", "native-command-dir", "json"]}]},
  "release native-source": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "target", "label": "--target <TARGET>", "required": true}, {"key": "source-dir", "label": "--source-dir <DIR>", "required": true}, {"key": "runner", "label": "--runner <RUNNER>", "required": true}, {"key": "runner-image", "label": "--runner-image <RUNNER_IMAGE>", "required": true}, {"key": "rust-toolchain", "label": "--rust-toolchain <RUST_TOOLCHAIN>", "required": true}, {"key": "rustc-path", "label": "--rustc-path <PATH>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseNativeSourceArgs", "members": ["#0", "target", "source-dir", "runner", "runner-image", "rust-toolchain", "rustc-path", "json"]}]},
  "release native-bundle": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "native-matrix-dir", "label": "--native-matrix-dir <DIR>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseNativeBundleArgs", "members": ["#0", "native-matrix-dir", "json"]}]},
  "release package": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "channel", "label": "--channel <CHANNEL>", "required": true}, {"key": "public-source-root", "label": "--public-source-root <DIR>", "hidden": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleasePackageArgs", "members": ["#0", "channel", "public-source-root", "json"]}]},
  "release formula": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "name", "label": "--name <NAME>", "required": true}, {"key": "python-formula", "label": "--python-formula <PYTHON_FORMULA>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseFormulaArgs", "members": ["#0", "name", "python-formula", "json"]}]},
  "release show": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "public-source-root", "label": "--public-source-root <DIR>", "hidden": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleaseShowArgs", "members": ["#0", "remote", "public-source-root", "json"]}]},
  "release publish": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleasePublishArgs", "members": ["#0", "remote", "json"]}]},
  "release promote": {"args": [{"key": "#0", "label": "<RELEASE_ID>", "required": true}, {"key": "channel", "label": "--channel <CHANNEL>", "required": true}, {"key": "public-source-root", "label": "--public-source-root <DIR>", "hidden": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReleasePromoteArgs", "members": ["#0", "channel", "public-source-root", "json"]}]},
  "repo show": {"args": [{"key": "remote", "label": "--remote <NAME>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RemoteJsonArgs", "members": ["remote", "json"]}]},
  "repo retire": {"args": [{"key": "remote", "label": "--remote <NAME>"}, {"key": "abort", "label": "--abort"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RepoRetireArgs", "members": ["remote", "abort", "json"]}]},
  "repo restore": {"args": [{"key": "remote", "label": "--remote <NAME>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RemoteJsonArgs", "members": ["remote", "json"]}]},
  "repo jobs": {"args": [{"key": "remote", "label": "--remote <NAME>"}, {"key": "worker-job-index", "label": "--worker-job-index <INDEX>", "conflicts": ["state", "limit"]}, {"key": "state", "label": "--state <STATE>", "conflicts": ["worker-job-index"]}, {"key": "limit", "label": "--limit <COUNT>", "conflicts": ["worker-job-index"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RepoJobsArgs", "members": ["remote", "worker-job-index", "state", "limit", "json"]}]},
  "repo ci-capabilities": {"args": [{"key": "remote", "label": "--remote <NAME>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@RemoteJsonArgs", "members": ["remote", "json"]}]},
  "auth whoami": {"args": [{"key": "remote", "label": "--remote <REMOTE>"}, {"key": "repo", "label": "--repo <REPO>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@AuthWhoamiArgs", "members": ["remote", "repo", "json"]}]},
  "auth grant": {"args": [{"key": "actor", "label": "--actor <ACTOR>", "required": true}, {"key": "role", "label": "--role <ROLE>"}, {"key": "repo", "label": "--repo <REPO>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@AuthGrantArgs", "members": ["actor", "role", "repo", "remote", "json"]}]},
  "auth bindings": {"args": [{"key": "repo", "label": "--repo <REPO>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@AuthBindingsArgs", "members": ["repo", "remote", "json"]}]},
  "config show": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@ConfigShowArgs", "members": ["json"]}]},
  "config set": {"args": [{"key": "agent-teams", "label": "--agent-teams <AGENT_TEAMS>"}, {"key": "plan-language", "label": "--plan-language <PLAN_LANGUAGE>"}, {"key": "plan-style", "label": "--plan-style <PLAN_STYLE>"}, {"key": "sprint", "label": "--sprint <SPRINT>"}, {"key": "default-author-mode", "label": "--default-author-mode <DEFAULT_AUTHOR_MODE>"}, {"key": "default-model", "label": "--default-model <DEFAULT_MODEL>"}, {"key": "task-review", "label": "--task-review <TASK_REVIEW>"}, {"key": "task-worktree-alias-root", "label": "--task-worktree-alias-root <TASK_WORKTREE_ALIAS_ROOT>"}, {"key": "task-worktree-main-seed-ram-max-bytes", "label": "--task-worktree-main-seed-ram-max-bytes <TASK_WORKTREE_MAIN_SEED_RAM_MAX_BYTES>"}, {"key": "id-namespace-prefix", "label": "--id-namespace-prefix <ID_NAMESPACE_PREFIX>"}, {"key": "user-name", "label": "--user-name <USER_NAME>"}, {"key": "user-email", "label": "--user-email <USER_EMAIL>"}, {"key": "post-finish-hooks", "label": "--post-finish-hooks <JSON>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ConfigSetArgs", "members": ["agent-teams", "plan-language", "plan-style", "sprint", "default-author-mode", "default-model", "task-review", "task-worktree-alias-root", "task-worktree-main-seed-ram-max-bytes", "id-namespace-prefix", "user-name", "user-email", "post-finish-hooks", "json"]}]},
  "config unset": {"args": [{"key": "#0", "label": "<KEY>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ConfigUnsetArgs", "members": ["#0", "json"]}]},
  "team session acknowledge": {"args": [{"key": "activation", "label": "--activation <ACTIVATION>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@Acknowledge", "members": ["activation", "json"]}]},
  "team session ensure": {"args": [{"key": "agent", "label": "--agent <AGENT>", "required": true}, {"key": "role", "label": "--role <ROLE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@Ensure", "members": ["agent", "role", "json"]}]},
  "team contacts": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@Contacts", "members": ["json"]}]},
  "external update": {"args": [{"key": "#0", "label": "[NAME]", "requires": ["@target_selection"], "conflicts": ["locked"]}, {"key": "to", "label": "--to <SNAPSHOT>", "requires": ["#0"], "conflicts": ["latest", "locked"]}, {"key": "latest", "label": "--latest", "requires": ["#0"], "conflicts": ["to", "locked"]}, {"key": "locked", "label": "--locked", "conflicts": ["#0", "to", "latest"]}, {"key": "validate", "label": "--validate"}, {"key": "no-recursive", "label": "--no-recursive"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ExternalUpdateArgs", "members": ["#0", "to", "latest", "locked", "validate", "no-recursive", "json"]}, {"key": "@target_selection", "members": ["to", "latest"]}]},
  "external status": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@ExternalStatusArgs", "members": ["json"]}]},
  "external doctor": {"args": [{"key": "fail-on-blocking", "label": "--fail-on-blocking"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ExternalDoctorArgs", "members": ["fail-on-blocking", "json"]}]},
  "external link": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "#1", "label": "<PATH>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ExternalLinkArgs", "members": ["#0", "#1", "json"]}]},
  "external unlink": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ExternalUnlinkArgs", "members": ["#0", "json"]}]},
  "status": {"args": [{"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}], "groups": [{"key": "@StatusArgs", "members": ["json", "full"]}]},
  "diff": {"args": [{"key": "json", "label": "--json", "conflicts": ["stat", "name-only"]}, {"key": "stat", "label": "--stat", "conflicts": ["json", "name-only"]}, {"key": "name-only", "label": "--name-only", "conflicts": ["json", "stat"]}, {"key": "#0", "label": "[PATH]..."}], "groups": [{"key": "@DiffArgs", "members": ["json", "stat", "name-only", "#0"]}]},
  "pull": {"args": [{"key": "remote", "label": "--remote <REMOTE>"}, {"key": "line", "label": "--line <LINE>"}, {"key": "merge", "label": "--merge", "requires": ["restore"]}, {"key": "restore", "label": "--restore"}, {"key": "force", "label": "--force", "requires": ["restore"], "conflicts": ["merge"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PullArgs", "members": ["remote", "line", "merge", "restore", "force", "json"]}]},
  "push": {"args": [{"key": "remote", "label": "--remote <REMOTE>"}, {"key": "line", "label": "--line <LINE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PushArgs", "members": ["remote", "line", "json"]}]},
  "gc stats": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@GcStatsArgs", "members": ["json"]}]},
  "gc validate": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@GcValidateArgs", "members": ["json"]}]},
  "gc prune": {"args": [{"key": "apply", "label": "--apply"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@GcPruneArgs", "members": ["apply", "json"]}]},
  "stash save": {"args": [{"key": "message", "label": "--message <MESSAGE>"}, {"key": "keep-workspace", "label": "--keep-workspace"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@StashSaveArgs", "members": ["message", "keep-workspace", "json"]}]},
  "stash list": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@StashListArgs", "members": ["json"]}]},
  "stash show": {"args": [{"key": "#0", "label": "<STASH_ID>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@StashIdArgs", "members": ["#0", "json"]}]},
  "stash apply": {"args": [{"key": "#0", "label": "<STASH_ID>", "required": true}, {"key": "force", "label": "--force"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@StashRestoreArgs", "members": ["#0", "force", "json"]}]},
  "stash pop": {"args": [{"key": "#0", "label": "<STASH_ID>", "required": true}, {"key": "force", "label": "--force"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@StashRestoreArgs", "members": ["#0", "force", "json"]}]},
  "stash drop": {"args": [{"key": "#0", "label": "<STASH_ID>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@StashIdArgs", "members": ["#0", "json"]}]},
  "plan list": {"args": [{"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}, {"key": "all", "label": "--all", "conflicts": ["json"]}], "groups": [{"key": "@PlanListArgs", "members": []}, {"key": "@QueryScopeArgs", "members": ["local", "remote", "json"]}]},
  "plan show": {"args": [{"key": "#0", "label": "<PLAN>", "required": true}, {"key": "revision", "label": "--revision <REVISION>"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ShowArgs", "members": []}, {"key": "@QueryScopeArgs", "members": ["local", "remote", "json"]}]},
  "plan revisions": {"args": [{"key": "#0", "label": "<PLAN>", "required": true}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}, {"key": "all", "label": "--all", "conflicts": ["json"]}], "groups": [{"key": "@PlanIdArgs", "members": []}, {"key": "@QueryScopeArgs", "members": ["local", "remote", "json"]}]},
  "plan items": {"args": [{"key": "#0", "label": "<PLAN>", "required": true}, {"key": "revision", "label": "--revision <REVISION>"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ShowArgs", "members": []}, {"key": "@QueryScopeArgs", "members": ["local", "remote", "json"]}]},
  "plan candidates": {"args": [{"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "all", "label": "--all"}, {"key": "contains", "label": "--contains <TERMS>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@CandidatesArgs", "members": ["local", "remote", "all", "contains", "json"]}]},
  "plan inspect": {"args": [{"key": "#0", "label": "<PLAN>", "required": true}, {"key": "revision", "label": "--revision <REVISION>"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ShowArgs", "members": []}, {"key": "@QueryScopeArgs", "members": ["local", "remote", "json"]}]},
  "plan sync": {"args": [{"key": "#0", "label": "<TARGET>", "required": true}, {"key": "plan-ref", "label": "--plan-ref <PLAN_REF>"}, {"key": "prune", "label": "--prune"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <NAME>", "conflicts": ["local"]}, {"key": "reconcile", "label": "--reconcile"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SyncArgs", "members": ["#0", "plan-ref", "prune", "local", "remote", "reconcile", "json"]}]},
  "task start": {"args": [{"key": "title", "label": "--title <TITLE>", "unless": ["from"], "conflicts": ["from"]}, {"key": "intent", "label": "--intent <INTENT>", "required": true}, {"key": "from", "label": "--from <MARKDOWN_PATH#ITEM_REF>", "conflicts": ["title"]}, {"key": "edit-root", "label": "--edit-root <ABSOLUTE_PATH>"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}], "groups": [{"key": "@TaskStartArgs", "members": ["title", "intent", "from", "edit-root", "local", "remote", "json", "full"]}]},
  "task quick": {"args": [{"key": "intent", "label": "--intent <INTENT>", "required": true}, {"key": "title", "label": "--title <TITLE>"}, {"key": "edit-root", "label": "--edit-root <ABSOLUTE_PATH>"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}], "groups": [{"key": "@TaskQuickArgs", "members": ["intent", "title", "edit-root", "local", "remote", "json", "full"]}]},
  "task list": {"args": [{"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "all", "label": "--all"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TaskListArgs", "members": ["local", "remote", "all", "json"]}]},
  "task show": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TaskShowArgs", "members": ["#0", "local", "remote", "json"]}]},
  "task audit": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TaskAuditArgs", "members": ["#0", "local", "remote", "json"]}]},
  "task finish": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "sync-plan-markdown", "label": "--sync-plan-markdown"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}], "groups": [{"key": "@TaskFinishArgs", "members": ["#0", "message", "sync-plan-markdown", "local", "remote", "json", "full"]}]},
  "task abandon": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>", "conflicts": ["local"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TaskAbandonArgs", "members": ["#0", "local", "remote", "json"]}]},
  "change create": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "title", "label": "--title <TITLE>", "required": true}, {"key": "base-line", "label": "--base-line <LINE>"}, {"key": "fork-snapshot", "label": "--fork-snapshot <SNAPSHOT>"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangeCreateArgs", "members": ["#0", "title", "base-line", "fork-snapshot", "local", "remote", "json"]}]},
  "change list": {"args": [{"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "all", "label": "--all"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangeListArgs", "members": ["local", "remote", "all", "json"]}]},
  "change show": {"args": [{"key": "#0", "label": "<TASK_ID[/C-##]>", "required": true}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangeShowArgs", "members": ["#0", "local", "remote", "json"]}]},
  "change revert": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangeRevertArgs", "members": ["#0", "force", "dry-run", "local", "remote", "json"]}]},
  "change replay": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "onto", "label": "--onto <LINE>", "hidden": true}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangeReplayArgs", "members": ["#0", "onto", "force", "dry-run", "local", "remote", "json"]}]},
  "change close": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "local", "label": "--local", "conflicts": ["remote"]}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangeCloseArgs", "members": ["#0", "local", "remote", "json"]}]},
  "change publish": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ChangePublishArgs", "members": ["#0", "remote", "json"]}]},
  "snapshot create": {"args": [{"key": "resume", "label": "--resume <SNAPSHOT_ID>", "conflicts": ["message"]}, {"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}], "groups": [{"key": "@SnapshotCreateArgs", "members": ["resume", "#0", "message", "json", "full"]}]},
  "snapshot list": {"args": [{"key": "all", "label": "--all"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotListArgs", "members": ["all", "json"]}]},
  "snapshot show": {"args": [{"key": "#0", "label": "<SNAPSHOT_OR_TAG>", "required": true}, {"key": "files", "label": "--files"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotShowArgs", "members": ["#0", "files", "json"]}]},
  "snapshot diff": {"args": [{"key": "#0", "label": "<OLD_SNAPSHOT_OR_TAG>", "required": true}, {"key": "#1", "label": "<NEW_SNAPSHOT_OR_TAG>", "required": true}, {"key": "include-text", "label": "--include-text"}, {"key": "max-bytes", "label": "--max-bytes <BYTES>", "requires": ["include-text"]}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotDiffArgs", "members": ["#0", "#1", "include-text", "max-bytes", "json"]}]},
  "snapshot restore-lines": {"args": [{"key": "#0", "label": "<SNAPSHOT_ID>", "required": true}, {"key": "#1", "label": "<PATH>", "required": true}, {"key": "line", "label": "--line <LINE>", "conflicts": ["start", "end"]}, {"key": "start", "label": "--start <START_LINE>", "requires": ["end"], "conflicts": ["line"]}, {"key": "end", "label": "--end <END_LINE>", "requires": ["start"], "conflicts": ["line"]}, {"key": "yes", "label": "--yes"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotRestoreLinesArgs", "members": ["#0", "#1", "line", "start", "end", "yes", "json"]}, {"key": "@line_selection", "members": ["line", "start"], "required": true}]},
  "snapshot revert": {"args": [{"key": "#0", "label": "<SNAPSHOT_OR_TAG>", "required": true}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotRevertArgs", "members": ["#0", "force", "dry-run", "json"]}]},
  "snapshot replay": {"args": [{"key": "#0", "label": "<SNAPSHOT_OR_TAG>", "required": true}, {"key": "onto", "label": "--onto <LINE>", "hidden": true}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotReplayArgs", "members": ["#0", "onto", "force", "dry-run", "json"]}]},
  "snapshot ancestry": {"args": [{"key": "#0", "label": "<SNAPSHOT_OR_TAG>", "required": true}, {"key": "ancestors", "label": "--ancestors", "conflicts": ["descendants"]}, {"key": "descendants", "label": "--descendants", "conflicts": ["ancestors"]}, {"key": "first-parent", "label": "--first-parent"}, {"key": "max-depth", "label": "--max-depth <DEPTH>"}, {"key": "limit", "label": "--limit <COUNT>"}, {"key": "all", "label": "--all"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotAncestryArgs", "members": ["#0", "ancestors", "descendants", "first-parent", "max-depth", "limit", "all", "json"]}]},
  "snapshot is-ancestor": {"args": [{"key": "#0", "label": "<OLDER_SNAPSHOT_OR_TAG>", "required": true}, {"key": "#1", "label": "<NEWER_SNAPSHOT_OR_TAG>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotIsAncestorArgs", "members": ["#0", "#1", "json"]}]},
  "snapshot merge-base": {"args": [{"key": "#0", "label": "<LEFT_SNAPSHOT_OR_TAG>", "required": true}, {"key": "#1", "label": "<RIGHT_SNAPSHOT_OR_TAG>", "required": true}, {"key": "all", "label": "--all"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@SnapshotMergeBaseArgs", "members": ["#0", "#1", "all", "json"]}]},
  "commit": {"args": [{"key": "resume", "label": "--resume <SNAPSHOT_ID>", "conflicts": ["message"]}, {"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}], "groups": [{"key": "@SnapshotCreateArgs", "members": ["resume", "#0", "message", "json", "full"]}]},
  "tag create": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "snapshot", "label": "--snapshot <SNAPSHOT_ID>"}, {"key": "message", "label": "--message <MESSAGE>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TagCreateArgs", "members": ["#0", "snapshot", "message", "json"]}]},
  "tag list": {"args": [{"key": "json", "label": "--json"}], "groups": [{"key": "@TagListArgs", "members": ["json"]}]},
  "tag show": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TagShowArgs", "members": ["#0", "json"]}]},
  "tag delete": {"args": [{"key": "#0", "label": "<NAME>", "required": true}, {"key": "json", "label": "--json"}], "groups": [{"key": "@TagDeleteArgs", "members": ["#0", "json"]}]},
  "patchset publish": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "summary", "label": "--summary <SUMMARY>", "required": true}, {"key": "author-mode", "label": "--author-mode <MODE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PatchsetPublishArgs", "members": ["#0", "summary", "author-mode", "remote", "json"]}]},
  "patchset list": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PatchsetListArgs", "members": ["#0", "remote", "json"]}]},
  "patchset show": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PatchsetShowArgs", "members": ["#0", "remote", "json"]}]},
  "patchset select": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PatchsetSelectArgs", "members": ["#0", "remote", "json"]}]},
  "patchset ci-status": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PatchsetCiStatusArgs", "members": ["#0", "remote", "json"]}]},
  "patchset rerun-ci": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PatchsetRerunCiArgs", "members": ["#0", "remote", "json"]}]},
  "review show": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewShowArgs", "members": ["#0", "remote", "json"]}]},
  "review team request": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "group", "label": "--group <REVIEWER_GROUPS>", "required": true}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "note", "label": "--note <NOTE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewRequestArgs", "members": ["#0", "group", "patchset", "note", "remote", "json"]}]},
  "review team approve": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "reviewer", "label": "--reviewer <REVIEWER>"}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewApproveArgs", "members": ["#0", "reviewer", "patchset", "message", "remote", "json"]}]},
  "review team request-changes": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "reviewer", "label": "--reviewer <REVIEWER>"}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewApproveArgs", "members": ["#0", "reviewer", "patchset", "message", "remote", "json"]}]},
  "review team comment": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "reviewer", "label": "--reviewer <REVIEWER>"}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewApproveArgs", "members": ["#0", "reviewer", "patchset", "message", "remote", "json"]}]},
  "review team defer": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "reviewer", "label": "--reviewer <REVIEWER>"}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewApproveArgs", "members": ["#0", "reviewer", "patchset", "message", "remote", "json"]}]},
  "review task approve": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>", "required": true}, {"key": "message", "label": "--message <MESSAGE>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewTaskApproveArgs", "members": ["#0", "patchset", "message", "remote", "json"]}]},
  "review task request-changes": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewTaskRecordArgs", "members": ["#0", "patchset", "message", "remote", "json"]}]},
  "review task comment": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewTaskRecordArgs", "members": ["#0", "patchset", "message", "remote", "json"]}]},
  "review task defer": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>"}, {"key": "message", "label": "--message <MESSAGE>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewTaskRecordArgs", "members": ["#0", "patchset", "message", "remote", "json"]}]},
  "review code submit": {"args": [{"key": "reviewer", "label": "--reviewer <REVIEWER>"}, {"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "patchset", "label": "--patchset <PATCHSET_ID>", "required": true}, {"key": "message", "label": "--message <MESSAGE>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewCodeSubmitArgs", "members": ["reviewer", "#0", "patchset", "message", "remote", "json"]}]},
  "review code template": {"args": [{"key": "style", "label": "--style <STYLE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@ReviewCodeTemplateArgs", "members": ["style", "json"]}]},
  "attest put": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "tests", "label": "--tests <TESTS>"}, {"key": "lint", "label": "--lint <LINT>"}, {"key": "security", "label": "--security <SECURITY>"}, {"key": "license", "label": "--license <LICENSE>"}, {"key": "author-mode", "label": "--author-mode <AUTHOR_MODE>"}, {"key": "model", "label": "--model <MODEL>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@AttestPutArgs", "members": ["#0", "tests", "lint", "security", "license", "author-mode", "model", "remote", "json"]}]},
  "attest show": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@AttestShowArgs", "members": ["#0", "remote", "json"]}]},
  "policy eval": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PolicyEvalArgs", "members": ["#0", "remote", "json"]}]},
  "policy show": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PolicyShowArgs", "members": ["#0", "remote", "json"]}]},
  "policy waive": {"args": [{"key": "#0", "label": "<PATCHSET_ID>", "required": true}, {"key": "rule", "label": "--rule <RULE_NAME>", "required": true}, {"key": "reason", "label": "--reason <REASON>", "required": true}, {"key": "expires-at", "label": "--expires-at <EXPIRES_AT>"}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@PolicyWaiveArgs", "members": ["#0", "rule", "reason", "expires-at", "remote", "json"]}]},
  "worktree status": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "snapshot", "label": "--snapshot <SNAPSHOT_ID>"}, {"key": "line", "label": "--line <LINE_NAME>"}, {"key": "verbose", "label": "--verbose"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeStatusArgs", "members": ["#0", "snapshot", "line", "verbose", "json"]}]},
  "worktree restore": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "snapshot", "label": "--snapshot <SNAPSHOT_ID>"}, {"key": "line", "label": "--line <LINE_NAME>"}, {"key": "path", "label": "--path <PATH>"}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeRestoreArgs", "members": ["#0", "snapshot", "line", "path", "force", "dry-run", "json"]}]},
  "worktree show": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeShowArgs", "members": ["#0", "json"]}]},
  "worktree path": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "shell", "label": "--shell"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreePathArgs", "members": ["#0", "shell", "json"]}]},
  "worktree doctor": {"args": [{"key": "refresh", "label": "--refresh"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeDoctorArgs", "members": ["refresh", "json"]}]},
  "worktree cleanup-candidates": {"args": [{"key": "older-than", "label": "--older-than <DURATION>"}, {"key": "policy", "label": "--policy <POLICY>"}, {"key": "allow-manual-only", "label": "--allow-manual-only"}, {"key": "include-protected", "label": "--include-protected"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeCleanupCandidatesArgs", "members": ["older-than", "policy", "allow-manual-only", "include-protected", "json"]}]},
  "worktree cleanup": {"args": [{"key": "older-than", "label": "--older-than <DURATION>"}, {"key": "policy", "label": "--policy <POLICY>"}, {"key": "allow-manual-only", "label": "--allow-manual-only"}, {"key": "limit", "label": "--limit <COUNT>"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "yes", "label": "--yes"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeCleanupArgs", "members": ["older-than", "policy", "allow-manual-only", "limit", "dry-run", "yes", "json"]}]},
  "worktree prune-stale": {"args": [{"key": "dry-run", "label": "--dry-run"}, {"key": "yes", "label": "--yes"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreePruneStaleArgs", "members": ["dry-run", "yes", "json"]}]},
  "worktree list": {"args": [{"key": "json", "label": "--json"}, {"key": "refresh", "label": "--refresh"}], "groups": [{"key": "@WorktreeListArgs", "members": ["json", "refresh"]}]},
  "worktree sync": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "all", "label": "--all"}, {"key": "line", "label": "--line <LINE_NAME>"}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeSyncArgs", "members": ["#0", "all", "line", "force", "dry-run", "json"]}]},
  "worktree recreate": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeRecreateArgs", "members": ["#0", "dry-run", "json"]}]},
  "worktree recover-task": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "change", "label": "--change <INTERNAL_WORK_REF>", "hidden": true}, {"key": "remote", "label": "--remote <REMOTE>"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeRecoverTaskArgs", "members": ["#0", "change", "remote", "dry-run", "json"]}]},
  "worktree restore-owned-head": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeRestoreOwnedHeadArgs", "members": ["#0", "dry-run", "json"]}]},
  "worktree rebase": {"args": [{"key": "#0", "label": "[NAME]"}, {"key": "onto", "label": "--onto <LINE_NAME>"}, {"key": "continue", "label": "--continue"}, {"key": "abort", "label": "--abort"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeRebaseArgs", "members": ["#0", "onto", "continue", "abort", "dry-run", "json"]}]},
  "worktree remove": {"args": [{"key": "#0", "label": "[NAME]..."}, {"key": "all-stale", "label": "--all-stale"}, {"key": "delete-path", "label": "--delete-path"}, {"key": "force", "label": "--force"}, {"key": "dry-run", "label": "--dry-run"}, {"key": "yes", "label": "--yes"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorktreeRemoveArgs", "members": ["#0", "all-stale", "delete-path", "force", "dry-run", "yes", "json"]}]},
  "workflow guide": {"args": [{"key": "#0", "label": "[TOPIC]"}], "groups": [{"key": "@WorkflowGuideArgs", "members": ["#0"]}]},
  "workflow reconcile": {"args": [{"key": "remote", "label": "--remote <REMOTE>"}, {"key": "task", "label": "--task <TASK_ID>"}, {"key": "dry-run", "label": "--dry-run", "conflicts": ["apply"]}, {"key": "apply", "label": "--apply", "conflicts": ["dry-run"]}, {"key": "safe-only", "label": "--safe-only"}, {"key": "scheduled", "label": "--scheduled", "requires": ["apply"], "conflicts": ["dry-run"]}, {"key": "limit", "label": "--limit <LIMIT>"}, {"key": "json", "label": "--json"}], "groups": [{"key": "@WorkflowReconcileArgs", "members": ["remote", "task", "dry-run", "apply", "safe-only", "scheduled", "limit", "json"]}]},
  "workflow ready": {"args": [{"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "apply", "label": "--apply"}, {"key": "snapshot-message", "label": "--snapshot-message <SNAPSHOT_MESSAGE>", "requires": ["apply"]}, {"key": "summary", "label": "--summary <SUMMARY>", "requires": ["apply"]}, {"key": "tests", "label": "--tests <TESTS>", "requires": ["apply"]}, {"key": "lint", "label": "--lint <LINT>", "requires": ["apply"]}, {"key": "security", "label": "--security <SECURITY>", "requires": ["apply"]}, {"key": "license", "label": "--license <LICENSE>", "requires": ["apply"]}, {"key": "author-mode", "label": "--author-mode <AUTHOR_MODE>", "requires": ["apply"]}, {"key": "model", "label": "--model <MODEL>", "requires": ["apply"]}, {"key": "run-ci-once", "label": "--run-ci-once", "requires": ["apply"]}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}, {"key": "expected-identity", "label": "--expected-identity <PATH>"}, {"key": "remote", "label": "--remote <REMOTE>"}], "groups": [{"key": "@WorkflowReadyArgs", "members": ["#0", "apply", "snapshot-message", "summary", "tests", "lint", "security", "license", "author-mode", "model", "run-ci-once", "json", "full", "expected-identity", "remote"]}]},
  "workflow finish": {"args": [{"key": "reviewer", "label": "--reviewer <REVIEWER>", "requires": ["apply"]}, {"key": "#0", "label": "<TASK_ID>", "required": true}, {"key": "apply", "label": "--apply"}, {"key": "review-message", "label": "--review-message <REVIEW_MESSAGE>", "requires": ["apply"]}, {"key": "json", "label": "--json"}, {"key": "full", "label": "--full", "requires": ["json"]}, {"key": "expected-identity", "label": "--expected-identity <PATH>"}, {"key": "remote", "label": "--remote <REMOTE>"}], "groups": [{"key": "@WorkflowFinishArgs", "members": ["reviewer", "#0", "apply", "review-message", "json", "full", "expected-identity", "remote"]}]},
});

function scope(values) {
  if (values.local && values.remote !== undefined) {
    throw new TerminalUsageError("--local conflicts with --remote");
  }
  return { local: values.local, remoteName: values.remote };
}

function validateChoice(value, name, choices) {
  if (value !== undefined && !choices.includes(value)) {
    throw new TerminalUsageError(`invalid value ${JSON.stringify(value)} for ${name}; expected ${choices.join(", ")}`);
  }
}

function requireOption(values, name) {
  if (values[name] === undefined) throw new TerminalUsageError(`required option --${name} is missing`);
}

// Match the frozen Clap value parsers, including Rust's White_Space trim.
// Reference resolution and identity validation remain in the native service.
function trimReference(value) {
  return value.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
}

function parseWorkReference(value) {
  const reference = trimReference(value);
  if (reference.length === 0 || /^[0-9]+$/.test(reference)) {
    throw new TerminalUsageError("Task ID must be non-empty and non-numeric");
  }
  return reference;
}

function nativeParserValue(validate) {
  try {
    return validate();
  } catch (error) {
    if (error instanceof NativeBridgeError || error?.code !== "GenericFailure") throw error;
    throw new TerminalUsageError(error.message);
  }
}

function parsePatchsetReference(runtime, value) {
  return value === undefined ? undefined
    : nativeParserValue(() => runtime.validatePublicPatchsetArgument(value));
}

function parseReviewPatchsetSelector(runtime, value) {
  return value === undefined ? undefined
    : nativeParserValue(() => runtime.validateReviewPatchsetSelector(value));
}

function reviewTeamOptions(runtime, values) {
  return {
    patchsetId: values.patchset,
    reviewer: values.reviewer, message: values.message, remoteName: values.remote,
  };
}

function reviewTaskOptions(runtime, values) {
  return {
    patchsetId: values.patchset,
    message: values.message, remoteName: values.remote,
  };
}

function taskRequest(values, quick, stdoutIsTTY) {
  if (values.intent === undefined) {
    throw new TerminalUsageError("required option --intent is missing");
  }
  if (!quick && values.title !== undefined && values.from !== undefined) {
    throw new TerminalUsageError("--title conflicts with --from");
  }
  if (!quick && values.title === undefined && values.from === undefined) {
    throw new TerminalUsageError("task start requires --title or --from");
  }
  const request = {
    ...scope(values), intent: values.intent, title: values.title, editRoot: values["edit-root"],
    humanProgress: stdoutIsTTY === true && !values.json,
  };
  if (!quick) request.source = values.from;
  return request;
}

function planQuery(values) {
  return { ...scope(values), revision: values.revision };
}

function rejectAllJson(values) {
  if (values.all && values.json) throw new TerminalUsageError("--all conflicts with --json");
}

function requireApply(values, names) {
  if (!values.apply) {
    const provided = names.find((name) => values[name] !== undefined);
    if (provided) throw new TerminalUsageError(`--${provided} requires --apply`);
  }
}

function requireSourceCacheOptions(values) {
  for (const name of ["namespace-root", "core-repo-root", "ext-suffix"]) requireOption(values, name);
}

function sourceCacheRequest(values) {
  return {
    namespaceRoot: values["namespace-root"], coreRepoRoot: values["core-repo-root"],
    extSuffix: values["ext-suffix"], rustflags: values.rustflags ?? "", workerId: values["worker-id"] ?? "shared",
    coreSourceFingerprint: values["core-source-fingerprint"], serverSourceFingerprint: values["server-source-fingerprint"],
  };
}

function nativeFloat(value, name) {
  if (value === undefined) return undefined;
  if (/^[+-]?(?:inf(?:inity)?|nan)$/i.test(value)) {
    if (/nan$/i.test(value)) return NaN;
    return value.startsWith("-") ? -Infinity : Infinity;
  }
  if (!/^[+-]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:e[+-]?[0-9]+)?$/i.test(value)) {
    throw new TerminalUsageError(`${name} must be a floating-point number`);
  }
  return Number(value);
}

function parseRemoteRecoveryJobs(value) {
  if (!/^\+?[0-9]+$/.test(value) || BigInt(value) < 1n || BigInt(value) > 64n) {
    throw new TerminalUsageError("jobs must be an integer from 1 through 64");
  }
  return Number(value);
}

function unsigned32(value, name) {
  if (value === undefined) return undefined;
  if (!/^[+-]?\d+$/.test(value) || BigInt(value) < 0n || BigInt(value) > 4294967295n) {
    throw new TerminalUsageError(`${name} must be an unsigned 32-bit integer`);
  }
  return Number(BigInt(value));
}

function signed64(value, name) {
  if (!/^[+-]?\d+$/.test(value)) throw new TerminalUsageError(`${name} must be an integer`);
  const integer = BigInt(value);
  if (integer < -9223372036854775808n || integer > 9223372036854775807n) {
    throw new TerminalUsageError(`${name} is outside the signed 64-bit range`);
  }
  return integer;
}

function nativeSize(value, name, minimum = 0n) {
  if (value === undefined) return undefined;
  if (!/^\+?\d+$/.test(value)) throw new TerminalUsageError(`${name} must be an unsigned integer`);
  const integer = BigInt(value);
  // Every platform in the declared addon matrix has a 64-bit native usize.
  if (integer < minimum || integer > 18446744073709551615n) {
    throw new TerminalUsageError(`${name} is outside the native-size range`);
  }
  return integer <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(integer) : integer;
}

function positiveNativeSize(value, name) {
  return nativeSize(value, name, 1n);
}

function lineListOptions(values) {
  return { includeAll: values.all, archived: values.archived, remoteName: values.remote };
}

const SYNTAX_FOOTER = "\nFor more information, try '--help'.\n";

function usageText(name) {
  return terminalHelp(name, false).match(/^Usage: [^\n]+(?:\n {7}[^\n]+)*/m)[0];
}

function optionLabel(name, key, command) {
  const declared = COMMAND_SYNTAX[name]?.args.find((arg) => arg.key === key);
  if (declared) return declared.label;
  if (SCALAR_ARGUMENTS[name]?.[key]) return SCALAR_ARGUMENTS[name][key].label;
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const label = terminalHelp(name, false).split("\nOptions:\n")[1]
    ?.match(new RegExp(`^\\s+(?:-\\S,\\s+)?(--${escaped}(?: <[^>]+>)?)`, "m"))?.[1];
  return label ?? `--${key}${command.options[key]?.type === "string" ? ` <${key.replaceAll("-", "_").toUpperCase()}>` : ""}`;
}

function selectedUsage(name, command, keys, requiredKeys = [], includeDependencies = true) {
  const usage = usageText(name);
  if (["external update", "task start", "task quick", "snapshot restore-lines"].includes(name)) return usage;
  const syntax = COMMAND_SYNTAX[name];
  const requiredFlags = new Set([...(syntax?.args.filter((arg) => arg.required).map((arg) => arg.key) ?? []), ...requiredKeys]);
  const pending = [...keys, ...requiredFlags];
  const visited = new Set();
  for (let index = 0; includeDependencies && index < pending.length; index++) {
    const key = pending[index];
    if (visited.has(key)) continue;
    visited.add(key);
    for (const dependency of syntax?.args.find((arg) => arg.key === key)?.requires ?? []) {
      requiredFlags.add(dependency);
      pending.push(dependency);
    }
  }
  const flagArguments = syntax?.args.filter((arg) => !arg.key.startsWith("#") && !arg.hidden);
  const flags = syntax ? [
    ...flagArguments.filter((arg) => requiredFlags.has(arg.key)),
    ...flagArguments.filter((arg) => !requiredFlags.has(arg.key) && keys.includes(arg.key)),
  ].map((arg) => arg.label)
    : keys.map((key) => optionLabel(name, key, command));
  if (syntax && keys.includes("help")) flags.push("--help");
  const required = command.positional.length - (command.optionalPositionals ?? 0);
  const positionals = syntax ? syntax.args.filter((arg) => arg.key.startsWith("#")).map((arg) =>
    keys.includes(arg.key) ? arg.label.replace(/^\[([^\]]+)\]/, "<$1>") : arg.label)
    : command.positional.map((item, index) =>
      `${index < required ? `<${item}>` : `[${item}]`}${command.variadic && index === command.positional.length - 1 ? "..." : ""}`);
  return `Usage: ait ${name}${[...flags, ...positionals].map((item) => ` ${item}`).join("")}`;
}

function syntaxFailure(message, diagnostic) {
  const error = new TerminalUsageError(message);
  error.terminalDiagnostic = diagnostic;
  throw error;
}

function booleanValueUsage(name, command, key, processed) {
  if (["external update", "task start", "task quick", "snapshot restore-lines"].includes(name)) return usageText(name);
  const syntax = COMMAND_SYNTAX[name];
  const matched = processed.map((item) => command.variadic && item.startsWith("#")
    ? `#${Math.min(Number(item.slice(1)), command.positional.length - 1)}` : item);
  const pending = matched.at(-1);
  if (pending?.startsWith("#") || command.options[pending]?.type === "string") {
    matched.pop();
    if (command.variadic && pending.startsWith("#")) while (matched.at(-1) === pending) matched.pop();
  }
  // Attached boolean-value errors retain Clap's already resolved argument groups.
  const groups = syntax.groups.filter((group) => group.members.some((member) => matched.includes(member)));
  if (groups.length === 0) return selectedUsage(name, command, [key], [], false);
  const covered = new Set(groups.flatMap((group) => group.members));
  const included = new Set([...matched, key]);
  const args = syntax.args.filter((arg) => !covered.has(arg.key) && !arg.hidden);
  const flags = args.filter((arg) => !arg.key.startsWith("#") && (arg.required || included.has(arg.key))).map((arg) => arg.label);
  if (key === "help") flags.push("--help");
  const groupLabels = groups.map((group) => `<${group.members.map((member) => {
    const arg = syntax.args.find((arg) => arg.key === member);
    return member.startsWith("#") ? arg.label.replace(/^[<\[]([^>\]]+)[>\]](?:\.\.\.)?$/, "$1") : arg.label;
  }).join("|")}>`);
  const positionals = args.filter((arg) => arg.key.startsWith("#")).map((arg) => included.has(arg.key)
    ? arg.label.replace(/^\[([^\]]+)\]/, "<$1>") : arg.label);
  return `Usage: ait ${name}${[...flags, ...groupLabels, ...positionals].map((value) => ` ${value}`).join("")}`;
}

function similarValues(value, candidates) {
  function confidence(candidate) {
    const left = Array.from(value), right = Array.from(candidate);
    if (value === candidate) return 1;
    if (left.length === 0 || right.length === 0) return 0;
    const distance = Math.max(0, Math.floor(Math.max(left.length, right.length) / 2) - 1);
    const usedLeft = new Set(), usedRight = new Set();
    for (let index = 0; index < left.length; index++) {
      for (let other = Math.max(0, index - distance); other <= Math.min(index + distance, right.length - 1); other++) {
        if (!usedRight.has(other) && left[index] === right[other]) {
          usedLeft.add(index); usedRight.add(other); break;
        }
      }
    }
    const matches = usedLeft.size;
    if (matches === 0) return 0;
    const matchedLeft = [...usedLeft].map((index) => left[index]);
    const matchedRight = [...usedRight].sort((left, right) => left - right).map((index) => right[index]);
    const transpositions = Math.floor(matchedLeft.filter((value, index) => value !== matchedRight[index]).length / 2);
    return (matches / left.length + matches / right.length + (matches - transpositions) / matches) / 3;
  }
  return candidates.map((value) => ({ value, score: confidence(value) }))
    .filter((candidate) => candidate.score > 0.7)
    .sort((left, right) => left.score - right.score).map((candidate) => candidate.value);
}

function childCommandNames(name) {
  const prefix = name ? `${name} ` : "";
  return [...(name === "current-source-cache" ? ["run-cli"] : []), ...terminalHelpPaths().filter((path) => path.startsWith(prefix) && path !== name &&
    !path.slice(prefix.length).includes(" ")).flatMap((path) => {
      const child = path.slice(prefix.length);
      return name === "" && child === "line" ? [child, "branch"]
        : name === "worktree" && child === "path" ? [child, "open"] : [child];
    }), "help"];
}

function longFlagNames(name) {
  return COMMAND_SYNTAX[name]?.args.filter((arg) => !arg.key.startsWith("#")).map((arg) => arg.key)
    .concat("help") ?? (name === "" ? ["help", "version"] : ["help"]);
}

function unrecognizedSubcommand(name, value, remaining = [], asPositional = false) {
  const argument = !asPositional && value.length > 1 && value.startsWith("-");
  const message = argument ? `unexpected argument '${value}' found` : `unrecognized subcommand '${value}'`;
  let tip = "";
  let usage = usageText(name);
  if (argument && value.startsWith("--")) {
    const candidate = similarValues(value.slice(2), longFlagNames(name)).at(-1);
    if (candidate) {
      tip = `\n  tip: a similar argument exists: '--${candidate}'\n`;
      usage = `Usage: ait${name ? ` ${name}` : ""} --${candidate} <COMMAND>`;
    }
    else {
      const children = childCommandNames(name).filter((child) => remaining.includes(child));
      children.sort((left, right) => remaining.indexOf(left) - remaining.indexOf(right));
      for (const child of children) {
        const suggestion = similarValues(value.slice(2), longFlagNames(name ? `${name} ${child}` : child)).at(-1);
        if (suggestion) { tip = `\n  tip: '${child} --${suggestion}' exists\n`; break; }
      }
    }
  } else if (!argument) {
    const candidates = similarValues(value, childCommandNames(name).filter((candidate) => candidate !== value));
    if (candidates.length > 0) tip = `\n  tip: ${candidates.length === 1 ? "a similar subcommand exists" : "some similar subcommands exist"}: ${candidates.map((candidate) => `'${candidate}'`).join(", ")}\n`;
  }
  syntaxFailure(message, `error: ${message}\n${tip}\n${usage}\n${SYNTAX_FOOTER}`);
}

function validateTerminalConstraints(name, command, tokens) {
  const syntax = COMMAND_SYNTAX[name];
  if (!syntax) return;
  let positional = 0;
  const ordered = tokens.filter((token) => token.kind !== "option-terminator")
    .map((token) => token.kind === "positional" ? `#${positional++}` : token.name);
  const present = new Set(ordered);
  const byKey = new Map(syntax.args.map((arg) => [arg.key, arg]));
  const groups = new Map((syntax.groups ?? []).map((group) => [group.key, group]));
  const has = (key) => present.has(key) || groups.get(key)?.members.some((member) => present.has(member));
  const label = (key) => byKey.get(key)?.label
    ?? `<${groups.get(key).members.map((member) => byKey.get(member).label).join("|")}>`;
  for (const key of ordered) {
    const arg = byKey.get(key);
    const conflicts = arg?.conflicts?.filter((other) => present.has(other)) ?? [];
    if (conflicts.length === 0) continue;
    const detail = conflicts.length === 1 ? `'${label(conflicts[0])}'`
      : `:\n${conflicts.map((other) => `  ${label(other)}`).join("\n")}`;
    const message = `the argument '${arg.label}' cannot be used with${conflicts.length === 1 ? " " : ""}${detail}`;
    const usage = selectedUsage(name, command, ordered.filter((other) => !conflicts.includes(other)));
    syntaxFailure(`${arg.label} conflicts with ${conflicts.map(label).join(", ")}`,
      `error: ${message}\n\n${usage}\n${SYNTAX_FOOTER}`);
  }
  const required = new Set(syntax.args.filter((arg) => arg.required).map((arg) => arg.key));
  for (const group of syntax.groups ?? []) if (group.required) required.add(group.key);
  const pending = [...ordered, ...required];
  const visited = new Set();
  for (let index = 0; index < pending.length; index++) {
    const key = pending[index];
    if (visited.has(key)) continue;
    visited.add(key);
    for (const dependency of byKey.get(key)?.requires ?? []) {
      required.add(dependency);
      pending.push(dependency);
    }
  }
  for (const arg of syntax.args) {
    if (arg.unless && !arg.unless.some(has)) required.add(arg.key);
  }
  const missing = [...required].filter((key) => !has(key));
  missing.sort((left, right) => Number(left.startsWith("#")) - Number(right.startsWith("#")));
  if (missing.length > 0) {
    const usage = selectedUsage(name, command, [...present, ...required], [...required]);
    const message = `the following required arguments were not provided:\n${missing.map((key) => `  ${key.startsWith("#") ? label(key).replace(/^\[([^\]]+)\]/, "<$1>") : label(key)}`).join("\n")}`;
    syntaxFailure(message, `error: ${message}\n\n${usage}\n${SYNTAX_FOOTER}`);
  }
}

function unexpectedArgument(name, command, value, positional = false, keys = [], suppressSuggestions = false) {
  const candidate = !positional && value.startsWith("--") ? similarValues(value.slice(2), longFlagNames(name)).at(-1) : undefined;
  const tip = suppressSuggestions ? "" : candidate ? `\n  tip: a similar argument exists: '--${candidate}'\n`
    : !positional && command.positional.length > 0 ? `\n  tip: to pass '${value}' as a value, use '-- ${value}'\n` : "";
  const selected = candidate ? [...keys, candidate] : keys;
  const usage = !positional && value.startsWith("--") && selected.length > 0
    ? selectedUsage(name, command, selected, [], false) : usageText(name);
  syntaxFailure(`unexpected argument '${value}' found`,
    `error: unexpected argument '${value}' found\n${tip}\n${usage}\n${SYNTAX_FOOTER}`);
}

function scalarErrorReason(kind, value, message) {
  const unsigned = /^\+?[0-9]+$/.test(value);
  if (kind === "jobs") {
    return !unsigned || BigInt(value) > 18446744073709551615n
      ? "jobs must be an integer from 1 through 64" : "jobs must be from 1 through 64";
  }
  if (kind === "jobLimit") {
    return !unsigned || BigInt(value) > 4294967295n
      ? "limit must be an integer from 1 through 1000" : "limit must be from 1 through 1000";
  }
  if (kind === "line" || kind === "positive") {
    const subject = kind === "line" ? "line number" : "value";
    return !unsigned || BigInt(value) > 18446744073709551615n
      ? `${subject} must be a positive integer` : `${subject} must be 1 or greater`;
  }
  if (["u32", "u64", "usize", "i64"].includes(kind)) {
    if (value === "") return "cannot parse integer from empty string";
    if (!(["i64", "u32"].includes(kind) ? /^[+-]?[0-9]+$/ : /^\+?[0-9]+$/).test(value)) {
      return "invalid digit found in string";
    }
    const integer = BigInt(value);
    if (integer < -9223372036854775808n) return "number too small to fit in target type";
    if (integer > (["i64", "u32"].includes(kind) ? 9223372036854775807n : 18446744073709551615n)) {
      return "number too large to fit in target type";
    }
    if (kind === "u32") return `${integer} is not in 0..=4294967295`;
  }
  if (kind === "f64") return value === "" ? "cannot parse float from empty string" : "invalid float literal";
  return message;
}

function parseTerminalScalar(definition, value, runtime) {
  const name = definition.label;
  switch (definition.kind) {
    case "choice": validateChoice(value, name, definition.choices); break;
    case "path":
      if (value === "") throw new TerminalUsageError(`a value is required for '${name}' but none was supplied`);
      break;
    case "u32": unsigned32(value, name); break;
    case "u64":
    case "usize": nativeSize(value, name); break;
    case "i64": signed64(value, name); break;
    case "f64": nativeFloat(value, name); break;
    case "line":
    case "positive": positiveNativeSize(value, name); break;
    case "jobs": parseRemoteRecoveryJobs(value); break;
    case "jobLimit": {
      const limit = unsigned32(value, name);
      if (limit < 1 || limit > 1000) throw new TerminalUsageError("limit must be from 1 through 1000");
      break;
    }
    case "authoring": return nativeParserValue(() => runtime.parseSnapshotAuthoringReference(value));
    case "work": return parseWorkReference(value);
    case "patchset": return parsePatchsetReference(runtime, value);
    case "review": return parseReviewPatchsetSelector(runtime, value);
  }
  return value;
}

function validateTerminalTokens(name, command, tokens, values, positionals, runtime, suppressSuggestions) {
  let positionalIndex = 0;
  const seen = new Set();
  const processed = [];
  for (const token of tokens) {
    if (token.kind === "option-terminator") continue;
    const index = token.kind === "positional" ? positionalIndex++ : undefined;
    const key = index === undefined ? token.name : `#${index}`;
    if (index === undefined) {
      const option = key === "help" || key === "json" && COMMAND_SYNTAX[name].args.some((arg) => arg.key === "json") ? BOOLEAN
        : Object.hasOwn(command.options, key) ? command.options[key] : undefined;
      if (option === undefined) unexpectedArgument(name, command, token.rawName, false, processed, suppressSuggestions);
      const label = optionLabel(name, key, command);
      if (option.type === "string" && token.inlineValue && !token.rawName.startsWith("--") && token.value.startsWith("=")) {
        token.value = token.value.slice(1);
        values[key] = token.value;
      }
      if (option.type === "string" && (token.value === undefined ||
          !token.inlineValue && token.value.length > 1 && token.value.startsWith("-"))) {
        const next = token.value;
        if (next && next !== "--") {
          const nextKey = next.startsWith("--") ? next.slice(2).split("=", 1)[0]
            : Object.keys(command.options).find((key) => command.options[key].short === next[1]);
          if (!["help", "json"].includes(nextKey) && !Object.hasOwn(command.options, nextKey) && !next.startsWith("-h")) {
            unexpectedArgument(name, command, next.startsWith("--") ? next.split("=", 1)[0] : next.slice(0, 2), false, processed, suppressSuggestions);
          }
        }
        const message = `a value is required for '${label}' but none was supplied`;
        const choices = SCALAR_ARGUMENTS[name]?.[key]?.choices;
        syntaxFailure(message, `error: ${message}${choices ? `\n  [possible values: ${choices.join(", ")}]` : ""}\n${SYNTAX_FOOTER}`);
      }
      if (option.type === "boolean" && token.value !== undefined) {
        const message = `unexpected value '${token.value}' for '${label}' found; no more were expected`;
        syntaxFailure(message, `error: ${message}\n\n${booleanValueUsage(name, command, key, processed)}\n${SYNTAX_FOOTER}`);
      }
      if (seen.has(key) && !option.multiple) {
        const message = `the argument '${label}' cannot be used multiple times`;
        syntaxFailure(message, `error: ${message}\n\n${usageText(name)}\n${SYNTAX_FOOTER}`);
      }
      seen.add(key);
    } else if (index >= command.positional.length && !command.variadic) {
      unexpectedArgument(name, command, token.value, true, processed, suppressSuggestions);
    }
    processed.push(key);
    const definition = SCALAR_ARGUMENTS[name]?.[key];
    if (!definition || token.value === undefined) continue;
    try {
      const parsed = parseTerminalScalar(definition, token.value, runtime);
      if (index !== undefined) positionals[index] = parsed;
      else values[key] = parsed;
    } catch (error) {
      if (!(error instanceof TerminalUsageError)) throw error;
      if (definition.kind === "path") {
        error.terminalDiagnostic = `error: ${error.message}\n${SYNTAX_FOOTER}`;
      } else {
        const reason = definition.kind === "choice"
          ? `\n  [possible values: ${definition.choices.join(", ")}]`
          : `: ${scalarErrorReason(definition.kind, token.value, error.message)}`;
        const candidate = definition.kind === "choice" && !suppressSuggestions
          ? similarValues(token.value, definition.choices).at(-1) : undefined;
        const tip = candidate ? `\n  tip: a similar value exists: '${candidate}'\n` : "";
        const message = definition.kind === "choice" && token.value === ""
          ? `a value is required for '${definition.label}' but none was supplied`
          : `invalid value '${token.value}' for '${definition.label}'`;
        error.terminalDiagnostic = `error: ${message}${reason}\n${tip}${SYNTAX_FOOTER}`;
      }
      throw error;
    }
  }
}

function argumentsThroughHelp(args, options) {
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    if (arg === "--") break;
    if (arg === "--help") return args.slice(0, index + 1);
    if (arg.startsWith("--")) {
      const [name] = arg.slice(2).split("=", 1);
      if (!arg.includes("=") && options[name]?.type === "string") index++;
    } else if (arg.startsWith("-")) {
      for (let letter = 1; letter < arg.length; letter++) {
        if (arg[letter] === "h") return [...args.slice(0, index), arg.slice(0, letter + 1)];
        const option = Object.values(options).find((value) => value.short === arg[letter]);
        if (option?.type === "string") {
          if (letter === arg.length - 1) index++;
          break;
        }
      }
    }
  }
  return args;
}

export function terminalExitCode(error) {
  return error instanceof TerminalUsageError ||
    String(error?.code).startsWith("ERR_PARSE_ARGS_") ? 2 : 1;
}

export function reportTerminalError(error, {
  runtime,
  writeError = (text) => process.stderr.write(text),
} = {}) {
  const message = error?.message ?? String(error);
  if (terminalExitCode(error) === 2) {
    writeError(error.terminalDiagnostic ?? `error: ${message}\n`);
    return 2;
  }
  if (error instanceof NativeBridgeError) {
    writeError(`ait: ${message}\n`);
    return 1;
  }
  writeError(`${runtime.formatDomainError(message)}\n`);
  return runtime.domainErrorExitCode();
}

// This terminal-only parser never crosses the N-API boundary. Each handler
// calls a concrete Rust function with domain parameters, not CLI arguments.
export function runTerminal(args, {
  runtime,
  cwd = process.cwd(),
  stdoutIsTTY = process.stdout.isTTY === true,
  write = (text) => process.stdout.write(text),
  writeError = (text) => process.stderr.write(text),
} = {}) {
  if (!Array.isArray(args) || args.some((arg) => typeof arg !== "string" || arg.includes("\0"))) {
    throw new TerminalUsageError("terminal arguments must be NUL-free strings");
  }
  if (["--version", "-V"].includes(args[0])) {
    write(`ait ${runtime.version()}\n`);
    return 0;
  }
  if (args[0] === "branch") args = ["line", ...args.slice(1)];
  if (args[0] === "worktree" && args[1] === "open") args = ["worktree", "path", ...args.slice(2)];
  if (args.length === 0) {
    write(terminalHelp(""));
    return 0;
  }
  let name = "";
  let offset = 0;
  while (!COMMANDS[name]) {
    const token = args[offset];
    if (["--help", "-h"].includes(token)) {
      write(terminalHelp(name, token === "--help"));
      return 0;
    }
    if (token?.startsWith("--help=") || name === "" && token?.startsWith("--version=")) {
      const [flag] = token.split("=", 1);
      const value = token.slice(flag.length + 1);
      const message = `unexpected value '${value}' for '${flag}' found; no more were expected`;
      syntaxFailure(message, `error: ${message}\n\nUsage: ait${name ? ` ${name}` : ""} ${flag} <COMMAND>\n${SYNTAX_FOOTER}`);
    }
    if (token?.startsWith("-") && !token.startsWith("--") && token.length > 2) {
      if (token[1] === "h") { write(terminalHelp(name, false)); return 0; }
      if (token[1] === "V" && name === "") { write(`ait ${runtime.version()}\n`); return 0; }
      unrecognizedSubcommand(name, token.slice(0, 2), args.slice(offset + 1));
    }
    if (token === "help") {
      let path = name;
      for (const part of args.slice(offset + 1)) {
        const child = path === "" && part === "branch" ? "line" : path === "worktree" && part === "open" ? "path" : part;
        const next = path ? `${path} ${child}` : child;
        if (terminalHelp(next) === undefined) {
          const message = `unrecognized subcommand '${part}'`;
          syntaxFailure(message, `error: ${message}\n\n${usageText(path)}\n${terminalHelpPaths().includes(path) ? SYNTAX_FOOTER : ""}`);
        }
        path = next;
      }
      write(terminalHelp(path));
      return 0;
    }
    if (token === "--") {
      const value = args[offset + 1];
      if (value === undefined) { writeError(terminalHelp(name, false)); return 2; }
      if (childCommandNames(name).includes(value)) {
        const message = `unexpected argument '${value}' found`;
        syntaxFailure(message, `error: ${message}\n\n  tip: subcommand '${value}' exists; to use it, remove the '--' before it\n\n${usageText(name)}\n${SYNTAX_FOOTER}`);
      }
      unrecognizedSubcommand(name, value, args.slice(offset + 2), true);
    }
    if (token === undefined) {
      writeError(terminalHelp(name, false));
      return 2;
    }
    const next = name ? `${name} ${token}` : token;
    if (terminalHelp(next) === undefined) {
      unrecognizedSubcommand(name, token, args.slice(offset + 1));
    }
    name = next;
    offset++;
  }
  const command = COMMANDS[name];
  const commandArgs = argumentsThroughHelp(args.slice(offset), command.options);
  const { values, positionals, tokens } = parseArgs({
    args: commandArgs,
    options: { ...command.options,
      ...(COMMAND_SYNTAX[name].args.some((arg) => arg.key === "json") ? { json: BOOLEAN } : {}),
      help: { ...BOOLEAN, short: "h" } },
    allowPositionals: true,
    strict: false,
    tokens: true,
  });
  const suppressSuggestions = ["workflow ready", "workflow finish"].includes(name) && args.slice(offset)
    .some((arg) => arg === "--json" || arg.startsWith("--json="));
  validateTerminalTokens(name, command, tokens, values, positionals, runtime, suppressSuggestions);
  if (values.help) {
    write(terminalHelp(name, commandArgs.at(-1).startsWith("--")));
    return 0;
  }
  validateTerminalConstraints(name, command, tokens);
  if ((!command.variadic && positionals.length > command.positional.length) ||
      positionals.length < command.positional.length - (command.optionalPositionals ?? 0)) {
    throw new TerminalUsageError(`ait ${name} requires ${command.positional.length} positional argument(s)`);
  }
  if (values.full && !values.json) {
    throw new TerminalUsageError("--full requires --json");
  }
  command.validate?.(values, positionals, runtime);
  const context = {
    authorityRoot: command.retainAuthorityRoot || command.discoverRepository ? runtime.repositoryAuthorityRoot(cwd) : cwd,
    stdoutIsTTY,
  };
  let operation;
  try {
    operation = command.invoke(runtime, cwd, values, positionals, context);
  } catch (error) {
    if (!command.queryError || error instanceof NativeBridgeError || terminalExitCode(error) === 2) {
      throw error;
    }
    writeError(`${runtime.formatSnapshotQueryError(error?.message ?? String(error))}\n`);
    return runtime.snapshotQueryErrorExitCode();
  }
  const result = command.payload ? command.payload(operation) : operation;
  const projected = values.json && command.project
    ? command.project(runtime, result, values, positionals, context.authorityRoot) : result;
  const text = !values.json && command.format
    ? command.format(runtime, result, values, positionals, context, operation)
    : runtime.encodeJson(projected, true);
  if (values.json || text.length > 0) write(command.verbatimText && !values.json ? text : `${text}\n`);
  if (command.afterOutputError) {
    const error = command.afterOutputError(runtime, result);
    if (error != null) {
      writeError(`${runtime.formatDomainError(error)}\n`);
      return runtime.domainErrorExitCode();
    }
  }
  return command.exitCode ? command.exitCode(runtime, result, operation, values, context) : 0;
}
