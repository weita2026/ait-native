export type AgentTransport = "telegram" | "discord" | "slack" | "line";
export type AgentWorkerOperation =
  | "slack-command"
  | "discord-interaction"
  | "reply-provider";
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };
export type EnvironmentOverrides = Record<
  string,
  string | number | boolean | null | undefined
>;

export const LANGUAGE_BINDING_CONTRACT: "ait.language.binding.v1";
export const AGENT_CAPABILITIES_CONTRACT: "ait.agent.worker.capabilities.v1";
export const SUPPORTED_TRANSPORTS: readonly AgentTransport[];
export const SUPPORTED_WORKER_OPERATIONS: readonly AgentWorkerOperation[];

export class NativeBridgeError extends Error {}
export class NativeResolutionError extends NativeBridgeError {}
export class NativeProtocolError extends NativeBridgeError {}

export interface NativeAddon {
  bindingInfoJson(): string;
  agentWorkerCapabilitiesJson(): string;
  agentWorkerTransactionJson(requestJson: string): string;
  runCli(args: string[]): number;
}

export interface LanguageBindingInfo extends JsonObject {
  contract: "ait.language.binding.v1";
  version: string;
  runtime_authority: "rust";
  python_binding: "pyo3";
  node_binding: "napi";
  process_transport_allowed: false;
  supported_surfaces: ["ait-core", "ait-agent-worker"];
}

export class NativeRuntime {
  constructor(options?: { addonPath?: string | URL });
  addonPath: string | null;
  resolveAddonPath(): string;
  loadAddon(): NativeAddon;
  resolveCallable(name: keyof NativeAddon | string): (...args: unknown[]) => unknown;
  call(name: "bindingInfoJson"): string;
  call(name: "agentWorkerCapabilitiesJson"): string;
  call(
    name: "agentWorkerTransactionJson",
    requestJson: string,
  ): string;
  call(name: "runCli", args: string[]): number;
  call(name: string, ...args: unknown[]): unknown;
  bindingInfo(): LanguageBindingInfo;
  version(): string;
  runCli(args?: string[]): number;
  agentCapabilities(): AgentCapabilitiesPayload;
  agentWorkerTransaction<T extends JsonValue = JsonValue>(
    request: JsonObject,
  ): T;
}

export function requiredAddonExports(): string[];

export interface AgentCapabilitiesPayload extends JsonObject {
  contract: "ait.agent.worker.capabilities.v1";
  binary: "ait-agent-worker";
  version: string;
  platform: string;
  architecture: string;
  supported_transports: AgentTransport[];
  event_loop_backends: string[];
  default_event_loop_backend: string;
  python_worker_execution_allowed: false;
}

export class AgentCapabilities {
  constructor(payload: AgentCapabilitiesPayload);
  readonly contract: "ait.agent.worker.capabilities.v1";
  readonly version: string;
  readonly platform: string;
  readonly architecture: string;
  readonly supportedTransports: readonly AgentTransport[];
  readonly eventLoopBackends: readonly string[];
  readonly defaultEventLoopBackend: string;
  readonly raw: AgentCapabilitiesPayload;
}

export interface AgentContext {
  cwd?: string | URL;
  repoRoot?: string | URL;
  manifestPath?: string | URL;
  env?: EnvironmentOverrides;
}

export interface AgentWorkerContext extends AgentContext {
  worker?: string;
  signature?: string;
  signatureTimestamp?: string;
  nowUnixSeconds?: number;
}

export class AgentClient {
  constructor(runtime?: NativeRuntime);
  readonly runtime: NativeRuntime;
  capabilities(): AgentCapabilities;
  workerTransaction<T extends JsonValue = JsonValue>(
    operation: AgentWorkerOperation,
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
  slackCommand<T extends JsonValue = JsonValue>(
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
  discordInteraction<T extends JsonValue = JsonValue>(
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
  replyProvider<T extends JsonValue = JsonValue>(
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
}
