import { spawnSync } from "node:child_process";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";

async function contract() {
  return JSON.parse(
    await readFile(
      path.join(ROOT, "lib", "npm-payload-contract.json"),
      "utf8",
    ),
  );
}

export async function currentPayloads() {
  const value = await contract();
  const payloads = value.payloads.filter(
    (payload) =>
      payload.os === process.platform && payload.cpu === process.arch,
  );
  if (payloads.length !== 2) {
    throw new Error(
      `fixture payloads do not support ${process.platform}/${process.arch}`,
    );
  }
  return payloads;
}

export function fixtureForwardArgs(first, second) {
  if (process.platform === "win32") {
    return [
      "-e",
      "process.stdout.write(JSON.stringify(process.argv.slice(1)))",
      first,
      second,
    ];
  }
  return ["--fixture-argv", first, second];
}

export function fixtureFailureArgs(status) {
  if (process.platform === "win32") {
    return ["-e", `process.exit(${status})`];
  }
  return ["--fixture-exit", String(status)];
}

export async function stageFixturePayloads(nodeModulesRoot) {
  const payloads = await currentPayloads();
  await mkdir(nodeModulesRoot, { recursive: true });
  for (const payload of payloads) {
    const packageRoot = path.join(nodeModulesRoot, payload.package);
    const executable = path.join(
      packageRoot,
      ...payload.executable.split("/"),
    );
    await mkdir(path.dirname(executable), { recursive: true });
    if (payload.os === "win32") {
      await copyFile(process.execPath, executable);
    } else {
      await writeFile(
        executable,
        '#!/bin/sh\nif [ "$1" = "--fixture-argv" ]; then\n  printf \'["%s","%s"]\' "$2" "$3"\n  exit 0\nfi\nif [ "$1" = "--fixture-exit" ]; then\n  exit "$2"\nfi\nexit 64\n',
      );
      await chmod(executable, 0o755);
    }
    const packageJson = {
      name: payload.package,
      version: payload.version,
      description: "Local executable-resolution fixture; never publish",
      license: payload.license,
      os: [payload.os],
      cpu: [payload.cpu],
      files: ["bin", "provenance.json", "LICENSE", "NOTICE"],
      aitNativePayload: {
        schema: "ait.node.npm-platform-payload/v1",
        component: payload.component,
        target: payload.target,
        executable: payload.executable,
        source_repository: payload.source_repository,
        source_snapshot: "SNP-000000000000",
      },
    };
    await writeFile(
      path.join(packageRoot, "package.json"),
      `${JSON.stringify(packageJson, null, 2)}\n`,
    );
    await writeFile(
      path.join(packageRoot, "provenance.json"),
      `${JSON.stringify({
        schema: "ait.node.fixture-platform-payload/v1",
        publishable: false,
        component: payload.component,
        target: payload.target,
      }, null, 2)}\n`,
    );
    await writeFile(
      path.join(packageRoot, "LICENSE"),
      `Fixture metadata only: ${payload.license}\n`,
    );
    await writeFile(
      path.join(packageRoot, "NOTICE"),
      `Non-publishable ${payload.component} fixture NOTICE\n`,
    );
  }
  return payloads;
}

function runNpm(args) {
  const result = spawnSync(NPM, args, {
    cwd: ROOT,
    encoding: "utf8",
    env: {
      ...process.env,
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_update_notifier: "false",
    },
    windowsHide: true,
  });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `${NPM} ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

export async function createFixturePayloadTarballs(outputRoot) {
  const temporaryRoot = await mkdtemp(
    path.join(os.tmpdir(), "ait-node-payload-fixtures-"),
  );
  try {
    const nodeModulesRoot = path.join(temporaryRoot, "node_modules");
    const payloads = await stageFixturePayloads(nodeModulesRoot);
    await mkdir(outputRoot, { recursive: true });
    const tarballs = [];
    for (const payload of payloads) {
      const result = runNpm([
        "pack",
        "--ignore-scripts",
        "--json",
        "--pack-destination",
        outputRoot,
        path.join(nodeModulesRoot, payload.package),
      ]);
      const packed = JSON.parse(result.stdout);
      if (packed.length !== 1) {
        throw new Error(`fixture pack failed for ${payload.package}`);
      }
      tarballs.push({
        ...payload,
        tarball: path.join(outputRoot, packed[0].filename),
      });
    }
    return tarballs;
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}
