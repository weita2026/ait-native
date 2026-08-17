import { fileURLToPath } from "node:url";

import {
  AGENT_CAPABILITIES_CONTRACT,
  SUPPORTED_TRANSPORTS,
  SUPPORTED_WORKER_OPERATIONS,
} from "./contract.js";
import { NativeProtocolError } from "./errors.js";
import { NativeRuntime } from "./runtime.js";

const TRANSPORTS = new Set(SUPPORTED_TRANSPORTS);
const WORKER_OPERATIONS = new Set(SUPPORTED_WORKER_OPERATIONS);
const WORKER_CONTEXT_FIELDS = Object.freeze({
  cwd: "cwd",
  repoRoot: "repo_root",
  manifestPath: "manifest_path",
  env: "env",
  signature: "signature",
  signatureTimestamp: "signature_timestamp",
  nowUnixSeconds: "now_unix_seconds",
});

export class AgentCapabilities {
  constructor(payload) {
    if (!isRecord(payload)) {
      throw new NativeProtocolError(
        "ait-agent-worker capabilities must be a JSON object",
      );
    }
    if (payload.contract !== AGENT_CAPABILITIES_CONTRACT) {
      throw new NativeProtocolError(
        "ait-agent-worker returned an unsupported capabilities contract",
      );
    }
    if (payload.binary !== "ait-agent-worker") {
      throw new NativeProtocolError(
        "ait-agent-worker capabilities identify the wrong binary",
      );
    }
    if (payload.python_worker_execution_allowed !== false) {
      throw new NativeProtocolError(
        "ait-agent-worker capabilities permit a forbidden Python fallback",
      );
    }

    const supportedTransports = requiredTextArray(
      payload,
      "supported_transports",
    );
    const unknown = supportedTransports.filter((value) => !TRANSPORTS.has(value));
    if (unknown.length > 0) {
      throw new NativeProtocolError(
        `ait-agent-worker reported unsupported transports: ${unknown
          .sort()
          .join(", ")}`,
      );
    }
    const eventLoopBackends = requiredTextArray(
      payload,
      "event_loop_backends",
    );
    const defaultEventLoopBackend = requiredText(
      payload,
      "default_event_loop_backend",
    );
    if (!eventLoopBackends.includes(defaultEventLoopBackend)) {
      throw new NativeProtocolError(
        "ait-agent-worker default event-loop backend is not available",
      );
    }

    this.contract = AGENT_CAPABILITIES_CONTRACT;
    this.version = requiredText(payload, "version");
    this.platform = requiredText(payload, "platform");
    this.architecture = requiredText(payload, "architecture");
    this.supportedTransports = Object.freeze(supportedTransports);
    this.eventLoopBackends = Object.freeze(eventLoopBackends);
    this.defaultEventLoopBackend = defaultEventLoopBackend;
    this.raw = payload;
    Object.freeze(this);
  }
}

export class AgentClient {
  constructor(runtime = new NativeRuntime()) {
    if (!(runtime instanceof NativeRuntime)) {
      throw new TypeError("runtime must be a NativeRuntime");
    }
    this.runtime = runtime;
  }

  capabilities() {
    return new AgentCapabilities(this.runtime.agentCapabilities());
  }

  workerTransaction(operation, payload, options = {}) {
    if (typeof operation !== "string") {
      throw new TypeError("worker operation must be text");
    }
    const normalizedOperation = operation.trim();
    if (!WORKER_OPERATIONS.has(normalizedOperation)) {
      throw new TypeError(
        `unsupported worker operation ${JSON.stringify(operation)}; expected: ${[
          ...WORKER_OPERATIONS,
        ]
          .sort()
          .join(", ")}`,
      );
    }
    if (payload === undefined) {
      throw new TypeError("worker payload is required");
    }
    const normalizedOptions = optionsObject(options, "worker options");
    const { worker = "main", ...context } = normalizedOptions;
    return this.runtime.agentWorkerTransaction({
      operation: normalizedOperation,
      payload,
      worker: normalizeWorkerName(worker),
      ...workerContext(context),
    });
  }

  slackCommand(payload, options = {}) {
    return this.workerTransaction("slack-command", payload, options);
  }

  discordInteraction(payload, options = {}) {
    return this.workerTransaction("discord-interaction", payload, options);
  }

  replyProvider(payload, options = {}) {
    return this.workerTransaction("reply-provider", payload, options);
  }

}

function workerContext(options) {
  return bindingContext(options, WORKER_CONTEXT_FIELDS, "worker options");
}

function bindingContext(options, fields, label) {
  const values = optionsObject(options, label);
  const unknown = Object.keys(values).filter(
    (name) => !Object.hasOwn(fields, name),
  );
  if (unknown.length > 0) {
    throw new TypeError(
      `unsupported ${label} fields: ${unknown.sort().join(", ")}`,
    );
  }
  const result = {};
  for (const [publicName, requestName] of Object.entries(fields)) {
    const value = values[publicName];
    if (value === undefined || value === null) {
      continue;
    }
    if (publicName === "env") {
      result[requestName] = normalizeEnvironment(value);
    } else if (publicName === "nowUnixSeconds") {
      if (!Number.isSafeInteger(value)) {
        throw new TypeError("nowUnixSeconds must be a safe integer");
      }
      result[requestName] = value;
    } else {
      result[requestName] = normalizeTextOrFileUrl(value, publicName);
    }
  }
  return result;
}

function normalizeEnvironment(value) {
  if (!isRecord(value)) {
    throw new TypeError("env must be an object");
  }
  const result = {};
  for (const [name, item] of Object.entries(value)) {
    if (name.length === 0) {
      throw new TypeError("environment override names must be non-empty");
    }
    if (item === null || item === undefined) {
      result[name] = null;
    } else if (
      typeof item === "string" ||
      typeof item === "number" ||
      typeof item === "boolean"
    ) {
      result[name] = String(item);
    } else {
      throw new TypeError(
        `environment override ${name} must be text, number, boolean, or null`,
      );
    }
  }
  return result;
}

function normalizeTextOrFileUrl(value, field) {
  const normalized =
    value instanceof URL ? fileURLToPath(value) : String(value).trim();
  if (normalized.length === 0) {
    throw new TypeError(`${field} must not be empty`);
  }
  return normalized;
}

function normalizeWorkerName(value) {
  if (typeof value !== "string") {
    throw new TypeError("worker name must be text");
  }
  const normalized = value.trim();
  if (normalized.length === 0) {
    throw new TypeError("worker name must not be empty");
  }
  return normalized;
}

function requiredText(payload, field) {
  const value = payload[field];
  if (typeof value !== "string" || value.length === 0) {
    throw new NativeProtocolError(
      `ait-agent-worker capabilities field ${field} must be non-empty text`,
    );
  }
  return value;
}

function requiredTextArray(payload, field) {
  const value = payload[field];
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.some((item) => typeof item !== "string" || item.length === 0) ||
    new Set(value).size !== value.length
  ) {
    throw new NativeProtocolError(
      `ait-agent-worker capabilities field ${field} must be unique text`,
    );
  }
  return [...value];
}

function optionsObject(value, label) {
  if (!isRecord(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value;
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
