import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnNpmSync } from "./npm-command.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const LOCAL_ADDON = path.join(ROOT, "native", "ait_napi.node");

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
  if (payloads.length !== 1) {
    throw new Error(
      `fixture addon does not support ${process.platform}/${process.arch}`,
    );
  }
  return payloads;
}

async function regularAddon(addonPath) {
  const entry = await lstat(addonPath);
  if (!entry.isFile() || entry.isSymbolicLink() || entry.size === 0) {
    throw new Error(`fixture addon must be a non-empty regular file: ${addonPath}`);
  }
}

export async function stageFixturePayloads(
  nodeModulesRoot,
  addonPath = LOCAL_ADDON,
) {
  await regularAddon(addonPath);
  const [payload] = await currentPayloads();
  await mkdir(nodeModulesRoot, { recursive: true });
  const packageRoot = path.join(nodeModulesRoot, payload.package);
  const installedAddon = path.join(
    packageRoot,
    ...payload.addon.split("/"),
  );
  await mkdir(path.dirname(installedAddon), { recursive: true });
  await copyFile(addonPath, installedAddon);
  const packageJson = {
    name: payload.package,
    version: payload.version,
    description: "Local direct Node-API fixture; never publish",
    license: payload.license,
    os: [payload.os],
    cpu: [payload.cpu],
    main: payload.addon,
    files: ["native", "provenance.json", "LICENSE", "NOTICE"],
    aitNativeAddon: {
      schema: "ait.node.napi-platform-addon/v1",
      component: payload.component,
      target: payload.target,
      addon: payload.addon,
      binding_repository: payload.binding_repository,
      binding_snapshot: payload.binding_snapshot,
    },
  };
  await writeFile(
    path.join(packageRoot, "package.json"),
    `${JSON.stringify(packageJson, null, 2)}\n`,
  );
  await writeFile(
    path.join(packageRoot, "provenance.json"),
    `${JSON.stringify(
      {
        schema: "ait.node.fixture-napi-platform-addon/v1",
        publishable: false,
        component: payload.component,
        target: payload.target,
      },
      null,
      2,
    )}\n`,
  );
  await writeFile(
    path.join(packageRoot, "LICENSE"),
    `Fixture metadata only: ${payload.license}\n`,
  );
  await writeFile(
    path.join(packageRoot, "NOTICE"),
    "Non-publishable direct Node-API fixture NOTICE\n",
  );
  return [payload];
}

function runNpm(args) {
  const result = spawnNpmSync(args, {
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
      `npm ${args.join(" ")} failed with status ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

export async function createFixturePayloadTarballs(
  outputRoot,
  addonPath = LOCAL_ADDON,
) {
  const temporaryRoot = await mkdtemp(
    path.join(os.tmpdir(), "ait-node-addon-fixture-"),
  );
  try {
    const nodeModulesRoot = path.join(temporaryRoot, "node_modules");
    const [payload] = await stageFixturePayloads(nodeModulesRoot, addonPath);
    await mkdir(outputRoot, { recursive: true });
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
    return [
      {
        ...payload,
        tarball: path.join(outputRoot, packed[0].filename),
      },
    ];
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}
