import assert from "node:assert/strict";
import path from "node:path";
import { pathToFileURL } from "node:url";

const installRoot = path.resolve(process.argv[2] ?? "");
if (process.argv[2] === undefined) {
  throw new Error("installed smoke requires an npm --prefix root");
}
const entrypoint = pathToFileURL(
  path.join(installRoot, "node_modules", "ait-native", "src", "index.js"),
);
const { AgentClient, NativeRuntime } = await import(entrypoint.href);

const runtime = new NativeRuntime();
const info = runtime.bindingInfo();
assert.equal(info.contract, "ait.language.binding.v1");
assert.equal(info.runtime_authority, "rust");
assert.equal(info.node_binding, "napi");
assert.equal(info.process_transport_allowed, false);
assert.equal(info.version, "1.0.0-rc.2");
assert.match(runtime.resolveAddonPath(), /ait_napi\.node$/);

const capabilities = new AgentClient(runtime).capabilities();
assert.equal(capabilities.contract, "ait.agent.worker.capabilities.v1");
const reply = new AgentClient(runtime).replyProvider({
  contract: "unsupported",
});
assert.equal(
  reply.contract,
  "ait.agent.gateway_reply_provider_response.v1",
);
process.stdout.write(
  `${JSON.stringify({
    contract: info.contract,
    version: info.version,
    agent_contract: capabilities.contract,
    reply_contract: reply.contract,
  })}\n`,
);
