import { spawnSync } from "node:child_process";
import { readFileSync, lstatSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);
const contract = JSON.parse(
  readFileSync(
    new URL("../lib/npm-payload-contract.json", import.meta.url),
    "utf8",
  ),
);

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(1);
}

function selectPayload(command) {
  if (contract.schema !== "ait.node.npm-platform-packages/v1") {
    fail("ait-native payload contract is invalid");
  }
  const matches = contract.payloads.filter(
    (payload) =>
      payload.command === command &&
      payload.os === process.platform &&
      payload.cpu === process.arch,
  );
  if (matches.length !== 1) {
    fail(
      `ait-native does not support ${command} on ${process.platform}/${process.arch}`,
    );
  }
  return matches[0];
}

function resolveExecutable(payload) {
  let packageJsonPath;
  try {
    packageJsonPath = require.resolve(`${payload.package}/package.json`);
  } catch (error) {
    if (error?.code === "MODULE_NOT_FOUND") {
      fail(
        `ait-native is missing ${payload.package}@${payload.version}; reinstall with optional dependencies enabled`,
      );
    }
    throw error;
  }

  const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));
  const metadata = packageJson.aitNativePayload;
  const metadataKeys = [
    "component",
    "executable",
    "schema",
    "source_repository",
    "source_snapshot",
    "target",
  ];
  if (
    packageJson.name !== payload.package ||
    packageJson.version !== payload.version ||
    packageJson.license !== payload.license ||
    !Array.isArray(packageJson.os) ||
    packageJson.os.length !== 1 ||
    packageJson.os[0] !== payload.os ||
    !Array.isArray(packageJson.cpu) ||
    packageJson.cpu.length !== 1 ||
    packageJson.cpu[0] !== payload.cpu ||
    metadata?.schema !== "ait.node.npm-platform-payload/v1" ||
    metadata.component !== payload.component ||
    metadata.target !== payload.target ||
    metadata.executable !== payload.executable ||
    metadata.source_repository !== payload.source_repository ||
    !/^SNP-[0-9A-F]{12}$/.test(metadata.source_snapshot ?? "") ||
    JSON.stringify(Object.keys(metadata).sort()) !==
      JSON.stringify(metadataKeys) ||
    packageJson.bin !== undefined ||
    packageJson.main !== undefined ||
    packageJson.exports !== undefined ||
    packageJson.dependencies !== undefined ||
    packageJson.optionalDependencies !== undefined ||
    packageJson.scripts !== undefined
  ) {
    fail(`ait-native platform package ${payload.package} is invalid`);
  }

  const packageRoot = path.dirname(packageJsonPath);
  const executable = path.resolve(
    packageRoot,
    ...payload.executable.split("/"),
  );
  if (
    executable === packageRoot ||
    !executable.startsWith(`${packageRoot}${path.sep}`)
  ) {
    fail(`ait-native executable path for ${payload.package} is invalid`);
  }

  let entry;
  try {
    entry = lstatSync(executable);
  } catch (error) {
    if (error?.code === "ENOENT") {
      fail(`ait-native platform package ${payload.package} is missing its executable`);
    }
    throw error;
  }
  if (!entry.isFile() || entry.isSymbolicLink()) {
    fail(`ait-native executable from ${payload.package} must be a regular file`);
  }
  return executable;
}

export function launch(command) {
  if (command !== "ait" && command !== "ait-server") {
    fail(`ait-native launcher command ${command} is invalid`);
  }
  const executable = resolveExecutable(selectPayload(command));
  const result = spawnSync(executable, process.argv.slice(2), {
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error !== undefined) {
    fail(`failed to start ${command}: ${result.error.message}`);
  }
  if (result.signal !== null) {
    fail(`${command} terminated by ${result.signal}`);
  }
  process.exit(result.status ?? 1);
}
