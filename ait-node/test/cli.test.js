import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ENTRYPOINT = path.join(ROOT, "bin", "ait.mjs");

function run(args, cwd = ROOT) {
  return spawnSync(process.execPath, [ENTRYPOINT, ...args], {
    cwd,
    encoding: "utf8",
  });
}

test("npm ait command enters the embedded Rust CLI and preserves status", () => {
  const version = run(["--version"]);
  assert.equal(version.status, 0, version.stderr);
  assert.equal(version.stdout, "ait 1.0.0-rc.9\n");
  assert.equal(version.stderr, "");

  const invalid = run(["definitely-not-an-ait-command"]);
  assert.equal(invalid.status, 2);
  assert.match(invalid.stderr, /unrecognized subcommand/);
});

test("repository language and build manifests do not change the command", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-cli-shapes-"));
  context.after(() => rm(root, { recursive: true, force: true }));
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
    const result = run(["--version"], repository);
    assert.equal(result.status, 0, `${name}: ${result.stderr}`);
    assert.equal(result.stdout, "ait 1.0.0-rc.9\n");
  }
});

test("installed runtime has no subprocess or executable resolver", async () => {
  const runtime = await readFile(path.join(ROOT, "src", "runtime.js"), "utf8");
  const command = await readFile(ENTRYPOINT, "utf8");
  const source = `${runtime}\n${command}`;

  assert.doesNotMatch(source, /node:child_process/);
  assert.doesNotMatch(source, /\bspawn(?:Sync)?\s*\(/);
  assert.doesNotMatch(source, /\bexec(?:File|Sync)?\s*\(/);
  assert.doesNotMatch(source, /ait-server|\.exe\b|ambient.*PATH/i);
  assert.match(command, /\.runCli\(process\.argv\.slice\(2\)\)/);
});
