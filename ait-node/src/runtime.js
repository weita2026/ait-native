import { lstatSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { LANGUAGE_BINDING_CONTRACT } from "./contract.js";
import {
  NativeProtocolError,
  NativeResolutionError,
} from "./errors.js";

const require = createRequire(import.meta.url);
const CONTRACT_PATH = fileURLToPath(
  new URL("../lib/npm-payload-contract.json", import.meta.url),
);
const LOCAL_ADDON_PATH = fileURLToPath(
  new URL("../native/ait_napi.node", import.meta.url),
);
const REQUIRED_EXPORTS = Object.freeze([
  "bindingInfoJson",
  "agentWorkerCapabilitiesJson",
  "agentManagementJson",
  "agentWorkerTransactionJson",
  "runCli",
]);
const CONTRACT_KEYS = Object.freeze([
  "family_version",
  "payloads",
  "schema",
  "top_level_package",
]);
const PAYLOAD_KEYS = Object.freeze([
  "addon",
  "binding_repository",
  "binding_snapshot",
  "component",
  "cpu",
  "license",
  "os",
  "package",
  "target",
  "version",
]);
const ADDON_METADATA_KEYS = Object.freeze([
  "addon",
  "binding_repository",
  "binding_snapshot",
  "component",
  "schema",
  "target",
]);

let validatedContract = null;

export class NativeRuntime {
  constructor(options = {}) {
    if (!isRecord(options)) {
      throw new TypeError("NativeRuntime options must be an object");
    }
    const unknown = Object.keys(options).filter((name) => name !== "addonPath");
    if (unknown.length > 0) {
      throw new TypeError(
        `unsupported NativeRuntime options: ${unknown.sort().join(", ")}`,
      );
    }
    this.addonPath =
      options.addonPath === undefined
        ? null
        : normalizeAddonPath(options.addonPath);
    this.addon = null;
  }

  resolveAddonPath() {
    if (this.addonPath !== null) {
      assertRegularAddon(this.addonPath);
      return this.addonPath;
    }

    if (regularNonSymlinkFile(LOCAL_ADDON_PATH)) {
      this.addonPath = LOCAL_ADDON_PATH;
      return this.addonPath;
    }

    const payload = selectPlatformPayload(loadContract());
    let packageJsonPath;
    try {
      packageJsonPath = require.resolve(`${payload.package}/package.json`);
    } catch (error) {
      if (error?.code === "MODULE_NOT_FOUND") {
        throw new NativeResolutionError(
          `Rust AIT binding package ${payload.package}@${payload.version} is missing; reinstall with optional dependencies enabled`,
          { cause: error },
        );
      }
      throw error;
    }
    const packageJson = readJson(packageJsonPath, payload.package);
    validatePlatformPackage(packageJson, payload);
    const packageRoot = path.dirname(packageJsonPath);
    const addonPath = path.resolve(packageRoot, ...payload.addon.split("/"));
    if (
      addonPath === packageRoot ||
      !addonPath.startsWith(`${packageRoot}${path.sep}`)
    ) {
      throw new NativeResolutionError(
        `Rust AIT binding path for ${payload.package} escapes its package`,
      );
    }
    assertRegularAddon(addonPath);
    this.addonPath = addonPath;
    return addonPath;
  }

  loadAddon() {
    if (this.addon !== null) {
      return this.addon;
    }
    const addonPath = this.resolveAddonPath();
    let addon;
    try {
      addon = require(addonPath);
    } catch (error) {
      throw new NativeResolutionError(
        `Rust AIT binding could not be loaded from ${addonPath}`,
        { cause: error },
      );
    }
    if (!isRecord(addon)) {
      throw new NativeResolutionError(
        `Rust AIT binding did not export an object: ${addonPath}`,
      );
    }
    for (const name of REQUIRED_EXPORTS) {
      if (typeof addon[name] !== "function") {
        throw new NativeResolutionError(
          `Rust AIT binding does not export ${name} as a supported function`,
        );
      }
    }
    this.addon = addon;
    return addon;
  }

  resolveCallable(name) {
    if (typeof name !== "string" || name.trim().length === 0) {
      throw new NativeResolutionError(
        "native addon export name must be non-empty text",
      );
    }
    const exportName = name.trim();
    const resolved = this.loadAddon()[exportName];
    if (typeof resolved !== "function") {
      throw new NativeResolutionError(
        `Rust AIT binding does not export ${exportName} as a supported function`,
      );
    }
    return resolved;
  }

  call(name, ...args) {
    return this.resolveCallable(name)(...args);
  }

  bindingInfo() {
    const payload = parseObjectPayload(
      this.call("bindingInfoJson"),
      "language binding info",
    );
    if (payload.contract !== LANGUAGE_BINDING_CONTRACT) {
      throw new NativeProtocolError(
        "ait-napi returned an unsupported language binding contract",
      );
    }
    if (payload.runtime_authority !== "rust") {
      throw new NativeProtocolError(
        "ait-napi language binding does not identify Rust authority",
      );
    }
    if (payload.node_binding !== "napi") {
      throw new NativeProtocolError(
        "ait-napi language binding does not identify Node-API",
      );
    }
    if (payload.process_transport_allowed !== false) {
      throw new NativeProtocolError(
        "ait-napi language binding permits a process API transport",
      );
    }
    const version = requiredText(payload, "version", "language binding info");
    if (version !== loadContract().family_version) {
      throw new NativeProtocolError(
        `ait-napi version ${version} does not match npm family ${loadContract().family_version}`,
      );
    }
    return payload;
  }

  version() {
    return requiredText(this.bindingInfo(), "version", "language binding info");
  }

  runCli(args = []) {
    if (
      !Array.isArray(args) ||
      args.some(
        (value) => typeof value !== "string" || value.includes("\u0000"),
      )
    ) {
      throw new TypeError("AIT CLI arguments must be an array of NUL-free strings");
    }
    const status = this.call("runCli", [...args]);
    if (!Number.isSafeInteger(status) || status < 0 || status > 255) {
      throw new NativeProtocolError(
        "ait-napi CLI entrypoint returned an invalid exit status",
      );
    }
    return status;
  }

  agentCapabilities() {
    return parseObjectPayload(
      this.call("agentWorkerCapabilitiesJson"),
      "ait-agent-worker capabilities",
    );
  }

  agentManagement(request) {
    return parseJsonPayload(
      this.call("agentManagementJson", encodeRequest(request, "ait-agent")),
      "ait-agent response",
    );
  }

  agentWorkerTransaction(request) {
    return parseJsonPayload(
      this.call(
        "agentWorkerTransactionJson",
        encodeRequest(request, "ait-agent-worker"),
      ),
      "ait-agent-worker response",
    );
  }
}

export function requiredAddonExports() {
  return [...REQUIRED_EXPORTS];
}

function loadContract() {
  if (validatedContract !== null) {
    return validatedContract;
  }
  const contract = readJson(CONTRACT_PATH, "npm addon contract");
  assertExactKeys(contract, CONTRACT_KEYS, "npm addon contract");
  if (
    contract.schema !== "ait.node.napi-platform-packages/v1" ||
    contract.top_level_package !== "ait-native" ||
    contract.family_version !== "1.0.0-rc.2" ||
    !Array.isArray(contract.payloads) ||
    contract.payloads.length !== 6
  ) {
    throw new NativeResolutionError("npm addon contract identity is invalid");
  }
  for (const [index, payload] of contract.payloads.entries()) {
    assertExactKeys(payload, PAYLOAD_KEYS, `npm addon contract row ${index}`);
    if (
      payload.component !== "ait-node" ||
      payload.binding_repository !== "ait-core" ||
      payload.binding_snapshot !== "SNP-4D8A3DA8FE1D" ||
      payload.license !== "Apache-2.0" ||
      payload.version !== contract.family_version ||
      payload.addon !== "native/ait_napi.node"
    ) {
      throw new NativeResolutionError(
        `npm addon contract row ${index} has invalid authority`,
      );
    }
  }
  validatedContract = contract;
  return contract;
}

function selectPlatformPayload(contract) {
  const matches = contract.payloads.filter(
    (payload) => payload.os === process.platform && payload.cpu === process.arch,
  );
  if (matches.length !== 1) {
    throw new NativeResolutionError(
      `ait-native does not support Node-API on ${process.platform}/${process.arch}`,
    );
  }
  return matches[0];
}

function validatePlatformPackage(packageJson, payload) {
  const metadata = packageJson.aitNativeAddon;
  assertExactKeys(metadata, ADDON_METADATA_KEYS, payload.package);
  if (
    packageJson.name !== payload.package ||
    packageJson.version !== payload.version ||
    packageJson.license !== payload.license ||
    packageJson.main !== payload.addon ||
    !Array.isArray(packageJson.os) ||
    packageJson.os.length !== 1 ||
    packageJson.os[0] !== payload.os ||
    !Array.isArray(packageJson.cpu) ||
    packageJson.cpu.length !== 1 ||
    packageJson.cpu[0] !== payload.cpu ||
    metadata.schema !== "ait.node.napi-platform-addon/v1" ||
    metadata.component !== payload.component ||
    metadata.target !== payload.target ||
    metadata.addon !== payload.addon ||
    metadata.binding_repository !== payload.binding_repository ||
    metadata.binding_snapshot !== payload.binding_snapshot ||
    packageJson.dependencies !== undefined ||
    packageJson.optionalDependencies !== undefined ||
    packageJson.scripts !== undefined ||
    packageJson.bin !== undefined
  ) {
    throw new NativeResolutionError(
      `Rust AIT binding package ${payload.package} is invalid`,
    );
  }
}

function readJson(filePath, label) {
  try {
    return JSON.parse(readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new NativeResolutionError(`${label} is not valid JSON`, {
      cause: error,
    });
  }
}

function assertExactKeys(value, expectedKeys, label) {
  if (!isRecord(value)) {
    throw new NativeResolutionError(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...expectedKeys].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new NativeResolutionError(
      `${label} fields must be exactly ${expected.join(", ")}`,
    );
  }
}

function normalizeAddonPath(value) {
  const candidate =
    value instanceof URL ? fileURLToPath(value) : String(value).trim();
  if (candidate.length === 0) {
    throw new NativeResolutionError("native addon path must not be empty");
  }
  return path.resolve(candidate);
}

function assertRegularAddon(addonPath) {
  let entry;
  try {
    entry = lstatSync(addonPath);
  } catch (error) {
    throw new NativeResolutionError(
      `Rust AIT binding is unavailable because ${addonPath} could not be inspected`,
      { cause: error },
    );
  }
  if (entry.isSymbolicLink() || !entry.isFile()) {
    throw new NativeResolutionError(
      `Rust AIT binding must be a regular non-symlink file: ${addonPath}`,
    );
  }
}

function regularNonSymlinkFile(filePath) {
  try {
    const entry = lstatSync(filePath);
    return entry.isFile() && !entry.isSymbolicLink();
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

function encodeRequest(request, label) {
  if (!isRecord(request)) {
    throw new TypeError(`${label} request must be an object`);
  }
  try {
    return JSON.stringify(request);
  } catch (error) {
    throw new TypeError(`${label} request is not JSON-serializable`, {
      cause: error,
    });
  }
}

function parseJsonPayload(payload, label) {
  if (typeof payload !== "string") {
    throw new NativeProtocolError(`${label} must be JSON text`);
  }
  try {
    return JSON.parse(payload);
  } catch (error) {
    throw new NativeProtocolError(`${label} is invalid JSON`, {
      cause: error,
    });
  }
}

function parseObjectPayload(payload, label) {
  const value = parseJsonPayload(payload, label);
  if (!isRecord(value)) {
    throw new NativeProtocolError(`${label} must be a JSON object`);
  }
  return value;
}

function requiredText(payload, field, label) {
  const value = payload[field];
  if (typeof value !== "string" || value.length === 0) {
    throw new NativeProtocolError(
      `${label} field ${field} must be non-empty text`,
    );
  }
  return value;
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
