import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { ADDON_FILES, assertPackageFiles, ENVELOPE_FILES } from "./package-inventory.mjs";

const installRoot = path.resolve(process.argv[2] ?? "");
if (process.argv[2] === undefined) {
  throw new Error("installed smoke requires an npm --prefix root");
}
const packageRoot = path.join(installRoot, "node_modules", "@wa120", "ait-native");
await assertPackageFiles(packageRoot, ENVELOPE_FILES);
const entrypoint = pathToFileURL(
  path.join(
    installRoot,
    "node_modules",
    "@wa120",
    "ait-native",
    "src",
    "index.js",
  ),
);
const { AgentClient, NativeRuntime } = await import(entrypoint.href);

const runtime = new NativeRuntime();
const info = runtime.bindingInfo();
assert.equal(info.contract, "ait.language.binding.v1");
assert.equal(info.runtime_authority, "rust");
assert.equal(info.node_binding, "napi");
assert.equal(info.process_transport_allowed, false);
assert.equal(info.version, "1.2.0");
assert.match(runtime.resolveAddonPath(), /ait_napi\.node$/);
await assertPackageFiles(path.dirname(path.dirname(runtime.resolveAddonPath())), ADDON_FILES);

assert.deepEqual(info.supported_surfaces, ["ait-core"]);
const client = new AgentClient(runtime);
for (const call of [
  () => client.capabilities(),
  () => client.listWorkers("telegram"),
  () => client.replyProvider({ contract: "unsupported" }),
]) {
  assert.throws(call, (error) => {
    assert.ok(error instanceof Error);
    assert.ok(error.message.includes("Chat integrations are retired in AIT 1.2.0"));
    return true;
  });
}

const temporary = await mkdtemp(path.join(os.tmpdir(), "ait-node-installed-api-"));
try {
  const root = path.join(temporary, "repository with spaces");
  await mkdir(root);
  runtime.taskWorkflowInit(root, { name: "node-installed-smoke" });
  runtime.taskWorkflowConfigSet(root, { agent_teams: "disabled", sprint: "off" });
  const changed = runtime.configureRepository(root, { plan_language: "ja-JP" });
  assert.equal(runtime.configurationError(changed), null);
  const configuration = runtime.taskWorkflowConfigShow(root);
  assert.match(runtime.encodeJson(configuration), /ja-JP/);
  const inventory = runtime.lineInventory(root);
  assert.equal(typeof inventory.current_line, "string");
  assert.ok(runtime.formatLineList(inventory).length > 0);
  assert.deepEqual(runtime.taskWorkflowTaskList(root, { local: true, all: true }), []);
  assert.throws(() => runtime.taskWorkflowTaskShow(root, "LNT-9999", { local: true }), Error);
  assert.throws(() => runtime.lineShow(root, undefined, "missing-remote"), Error);

  const entry = path.join(installRoot, "node_modules", "@wa120", "ait-native", "bin", "ait.mjs");
  function cli(...args) {
    const result = spawnSync(process.execPath, [entry, ...args], {
      cwd: root, encoding: "utf8", windowsHide: true,
    });
    assert.ifError(result.error);
    assert.equal(result.signal, null);
    return result;
  }
  const help = cli("task", "start", "--help");
  assert.equal(help.status, 0, help.stderr);
  assert.equal(help.stderr, "");
  assert.match(help.stdout, /--from <MARKDOWN_PATH#ITEM_REF>/);
  assert.match(help.stdout, /--intent <INTENT>/);
  const version = cli("--version");
  assert.equal(version.status, 0, version.stderr);
  assert.equal(version.stderr, "");
  assert.equal(version.stdout, `ait ${runtime.version()}\n`);
  const group = cli("review", "team");
  assert.equal(group.status, 2);
  assert.equal(group.stdout, "");
  assert.match(group.stderr, /Usage: ait review team <COMMAND>/);
  const shown = cli("config", "show", "--json");
  assert.equal(shown.status, 0, shown.stderr);
  assert.equal(shown.stderr, "");
  assert.deepEqual(runtime.decodeJson(shown.stdout), configuration);
  const configured = cli("config", "set", "--plan-language", "en-GB", "--json");
  assert.equal(configured.status, 0, configured.stderr);
  assert.equal(configured.stderr, "");
  assert.match(runtime.encodeJson(runtime.taskWorkflowConfigShow(root)), /en-GB/);
  const status = cli("status");
  assert.equal(status.status, 0, status.stderr);
  assert.equal(status.stderr, "");
  assert.ok(status.stdout.length > 0);
  const lines = cli("branch", "list", "--json");
  assert.equal(lines.status, 0, lines.stderr);
  assert.deepEqual(runtime.decodeJson(lines.stdout), runtime.lineInventory(root).payload);
  const missing = cli("task", "show", "LNT-9999", "--local");
  assert.equal(missing.status, runtime.domainErrorExitCode());
  assert.equal(missing.stdout, "");
  assert.ok(missing.stderr.length > 0);
  const remote = cli("line", "show", "--remote", "missing-remote", "--json");
  assert.equal(remote.status, runtime.domainErrorExitCode());
  assert.equal(remote.stdout, "");
  assert.ok(remote.stderr.length > 0);
  const syntax = cli("line", "switch", "main", "--force");
  assert.equal(syntax.status, 2);
  assert.equal(syntax.stdout, "");
} finally {
  await rm(temporary, { recursive: true, force: true });
}
process.stdout.write(
  `${JSON.stringify({
    contract: info.contract,
    version: info.version,
    supported_surfaces: info.supported_surfaces,
    chat_retired: true,
    direct_api: true,
    javascript_cli: true,
  })}\n`,
);
