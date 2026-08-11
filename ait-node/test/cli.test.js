import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cp, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  currentPayloads,
  fixtureFailureArgs,
  fixtureForwardArgs,
  stageFixturePayloads,
} from "../scripts/fixture-payloads.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

async function fixture(context) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-cli-test-"));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
  });
  const nodeModules = path.join(root, "node_modules");
  const packageRoot = path.join(nodeModules, "ait-native");
  await mkdir(packageRoot, { recursive: true });
  await cp(path.join(ROOT, "bin"), path.join(packageRoot, "bin"), {
    recursive: true,
  });
  await cp(path.join(ROOT, "lib"), path.join(packageRoot, "lib"), {
    recursive: true,
  });
  await stageFixturePayloads(nodeModules);
  return { nodeModules, packageRoot, root };
}

function run(packageRoot, command, args, cwd = packageRoot) {
  return spawnSync(
    process.execPath,
    [path.join(packageRoot, "bin", `${command}.mjs`), ...args],
    { cwd, encoding: "utf8" },
  );
}

test("both npm commands forward argv and native failure status", async (context) => {
  const { packageRoot } = await fixture(context);
  for (const command of ["ait", "ait-server"]) {
    const forwarded = run(packageRoot, command, fixtureForwardArgs(
      `${command}-one`,
      `${command}-two`,
    ));
    assert.equal(forwarded.status, 0, forwarded.stderr);
    assert.deepEqual(JSON.parse(forwarded.stdout), [
      `${command}-one`,
      `${command}-two`,
    ]);

    const failed = run(packageRoot, command, fixtureFailureArgs(29));
    assert.equal(failed.status, 29, failed.stderr);
  }
});

test("repository language and build manifests do not change either command", async (context) => {
  const { packageRoot, root } = await fixture(context);
  const repositoryShapes = [
    ["python", "pyproject.toml", "[project]\nname = \"fixture\"\n"],
    ["node", "package.json", "{\"name\":\"fixture\"}\n"],
    ["dotnet", "Fixture.csproj", "<Project />\n"],
    ["php", "composer.json", "{\"name\":\"fixture/project\"}\n"],
    ["c-cpp", "CMakeLists.txt", "project(fixture)\n"],
    ["java", "pom.xml", "<project />\n"],
  ];

  for (const [name, manifest, contents] of repositoryShapes) {
    const repository = path.join(root, name);
    await mkdir(repository);
    await writeFile(path.join(repository, manifest), contents);
    for (const command of ["ait", "ait-server"]) {
      const result = run(packageRoot, command, fixtureForwardArgs(
        `${command}-ok`,
        `${command}-stable`,
      ), repository);
      assert.equal(result.status, 0, `${name}/${command}: ${result.stderr}`);
      assert.deepEqual(JSON.parse(result.stdout), [
        `${command}-ok`,
        `${command}-stable`,
      ]);
    }
  }
});

test("missing and malformed implementation payloads fail closed", async (context) => {
  const { nodeModules, packageRoot } = await fixture(context);
  const payloads = await currentPayloads();
  const aitPayload = payloads.find((payload) => payload.component === "ait");
  const serverPayload = payloads.find(
    (payload) => payload.component === "ait-server",
  );
  await rm(path.join(nodeModules, aitPayload.package), {
    recursive: true,
    force: true,
  });
  const missing = run(packageRoot, "ait", ["--version"]);
  assert.equal(missing.status, 1);
  assert.match(missing.stderr, new RegExp(aitPayload.package));
  assert.match(missing.stderr, /optional dependencies enabled/);

  const serverExecutable = path.join(
    nodeModules,
    serverPayload.package,
    ...serverPayload.executable.split("/"),
  );
  await rm(serverExecutable, { force: true });
  await mkdir(serverExecutable);
  const malformed = run(packageRoot, "ait-server", ["--version"]);
  assert.equal(malformed.status, 1);
  assert.match(malformed.stderr, /must be a regular file/);
});
