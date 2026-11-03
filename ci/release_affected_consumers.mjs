import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

// Preparation inputs, not a family admission or source-cache receipt.
export const SOURCES = [
  { name: "ait-core", snapshot: "SNP-49762426B59A", manifest: "49762426b59aee14f415f1cb2835b68178ba5fed3dc012b8ee7d264cace0a9fd", count: 1360, tree: "b82d5da005fe1e024da90447872acfedaf17d5001f05c38a9d4fe027fe632383", epoch: "1793698437", authority: "origin Repo0 RCT-1941/P-01/L-01 complete; genuine authority23 pass; historical RCT1940 partial retained under owner disposition" },
  { name: "ait-server", snapshot: "SNP-720371F44A21", manifest: "720371f44a215ad07113c3767e84032544997cc2309e2947c161a3150bf3626b", count: 477, tree: "8b175d4f0b5739dc0c2f10622388ea723472745a8e2532d0ff698b7f36ccfa14", epoch: "1793634636", authority: "accepted origin Repo1 source; not tested by this batch" },
  { name: "ait-node", snapshot: "SNP-271BA1BD3422", manifest: "271ba1bd3422833fc23a5c303fef0f0685688f389fa67691d0289831845f762a", count: 41, tree: "ad7d98dc011d50e111b1e43b422e74bfc085604864cde4463a86fdb7e4b452f6", epoch: "1793700521", authority: "LNT-0022 R01 local complete; origin Repo3 RNT-0098/P-02/L-01 complete; CI Job118/a1" },
  { name: "ait-python", snapshot: "SNP-33459A90024F", manifest: "33459a90024f127893230c8aef150373791f03a8935e7d86592638c684223623", count: 28, tree: "bb27d72d08edf8c0280f7cfffd0dfdf1253d3d97c88502e178da273d238bfc92", epoch: "1793700091", authority: "LPT-0021/C-01 R02 local complete; origin Repo2 RPT-0090/P-02/L-01 complete; CI Job108/run1/a1" },
  { name: "ait-runner", snapshot: "SNP-07E0076369EE", manifest: "07e0076369ee06ddcc7f99bd59f549107221b7ee2f3610fc61082a877f9588d8", count: 28, tree: "fddcb7a3fa45ef442329bc1592fc8cfead8c713304b9d51130c5a26b599d21cc", epoch: "1793700130", authority: "origin Repo4 RRT-0071/P-02/L-01 complete; CI Job82/run1" },
];

const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const byteOrder = (a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b));
export function inventoryDigest(rows) {
  return hash([...rows].sort((a, b) => byteOrder(a.path, b.path))
    .map(row => `${row.mode} ${row.sha256} ${row.path}\n`).join(""));
}

function git(root, args) {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr || result.error?.message);
  return result.stdout;
}

export function verifyControlIdentity(root, snapshot) {
  assert(/^SNP-[0-9A-F]{12}$/.test(snapshot ?? ""), "invalid coordinator_snapshot");
  const message = git(root, ["show", "-s", "--format=%B", "HEAD"]);
  const records = [...message.matchAll(/^control (SNP-[0-9A-F]{12}) ([0-9a-f]{64}) \(([1-9][0-9]*) files\)$/gm)];
  assert.equal(records.length, 1, "carrier must identify one committed Native control Snapshot/manifest");
  const [, committedSnapshot, manifest, count] = records[0];
  assert.equal(committedSnapshot, `SNP-${manifest.slice(0, 12).toUpperCase()}`, "carrier Native control manifest identity mismatch");
  assert.equal(snapshot, committedSnapshot, "coordinator_snapshot differs from frozen carrier Native control");
  return { snapshot: committedSnapshot, manifest, files: Number(count) };
}

function files(root, prefix = "") {
  return readdirSync(path.join(root, prefix), { withFileTypes: true }).flatMap(entry => {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    assert(!entry.isSymbolicLink(), `source symlink: ${relative}`);
    if (entry.isDirectory()) return files(root, relative);
    assert(entry.isFile(), `source special file: ${relative}`);
    return [relative];
  });
}

export function verifySource(root, source, extra = []) {
  const prefix = `${source.name}/`;
  const rows = git(root, ["ls-files", "--stage", "-z", "--", source.name]).split("\0").filter(Boolean).map(line => {
    const match = /^(100644|100755) [0-9a-f]+ 0\t([\s\S]+)$/.exec(line);
    assert(match && match[2].startsWith(prefix), "unexpected source index entry");
    return { mode: match[1], path: match[2].slice(prefix.length) };
  });
  const actual = files(path.join(root, source.name)).sort(byteOrder);
  assert.deepEqual(actual, rows.map(row => row.path).sort(byteOrder), `${source.name}: extra/missing source file`);
  for (const row of rows) {
    const file = path.join(root, source.name, row.path), entry = lstatSync(file);
    assert(entry.isFile() && !entry.isSymbolicLink(), "source must be regular");
    if (process.platform !== "win32") assert.equal(Boolean(entry.mode & 0o111), row.mode === "100755", `source mode: ${row.path}`);
    row.sha256 = hash(readFileSync(file));
  }
  const selected = rows.filter(row => !extra.includes(row.path));
  assert.equal(selected.length, source.count, `${source.name}: Snapshot membership count`);
  assert.equal(inventoryDigest(selected), source.tree, `${source.name}: full Snapshot bytes/executable modes`);
  return selected;
}

export function verifyCoreMarker(marker, core, verifiedRows) {
  for (const [key, value] of Object.entries({ format: "ait.external.materialized", version: 3, name: "ait-core", repo_name: "ait-core", repository_index: 0, remote: "origin", snapshot: core.snapshot, parent_path: "", materialize_to: ".ait-external/ait-core" })) {
    assert.equal(marker[key], value, `original Core materialization marker: ${key}`);
  }
  assert.equal(marker.files.length, core.count);
  // Native marker v3 records paths and hashes; mode proof comes from the
  // independently checked full Snapshot inventory and the Git index.
  const content = rows => rows.map(({ path: name, sha256 }) => ({ path: name, sha256 }))
    .sort((a, b) => byteOrder(a.path, b.path));
  assert.deepEqual(content(marker.files), content(verifiedRows), "materialized origin Core differs from full selected Snapshot");
}

export function verifyCarrier(root) {
  const core = SOURCES[0];
  let coreRows;
  for (const source of SOURCES) {
    const rows = verifySource(root, source, source === core ? [".ait-external-marker.json"] : []);
    if (source === core) coreRows = rows;
  }
  const marker = JSON.parse(readFileSync(path.join(root, "ait-core/.ait-external-marker.json"), "utf8"));
  verifyCoreMarker(marker, core, coreRows);
  return SOURCES;
}

export function materializeCore(root, consumer) {
  assert(["ait-node", "ait-python", "ait-runner"].includes(consumer));
  const destination = path.join(root, consumer, ".ait-external/ait-core");
  assert(!existsSync(destination), "consumer materialization must start absent");
  mkdirSync(path.dirname(destination), { recursive: true });
  cpSync(path.join(root, "ait-core"), destination, { recursive: true, dereference: false, errorOnExist: true });
}

async function execute(root, consumer, target) {
  const source = SOURCES.find(row => row.name === `ait-${consumer}`);
  assert(source, "unknown affected consumer");
  const matrix = JSON.parse(readFileSync(path.join(root, "ci/native_bootstrap_matrix.json"), "utf8"));
  const platform = matrix.targets.find(row => row.target === target);
  assert(platform && matrix.rust_toolchain === "1.96.0");
  assert.equal(process.arch, platform.architecture === "x86_64" ? "x64" : "arm64");
  assert.equal(process.platform, { macos: "darwin", linux: "linux", windows: "win32" }[platform.os]);
  assert.equal(process.env.GITHUB_EVENT_NAME, "workflow_dispatch");
  assert.equal(process.env.DIAGNOSTIC_AFFECTED_CONSUMERS, "true");
  assert.equal(git(root, ["rev-parse", "HEAD"]).trim(), process.env.GITHUB_SHA);
  const control = verifyControlIdentity(root, process.env.AIT_CONTROL_SNAPSHOT);
  verifyCarrier(root);
  console.log(JSON.stringify({ control_snapshot: control.snapshot, control_manifest: control.manifest, sources: SOURCES, scope: "affected preparation; no family/origin-delivery/publication verdict", native_mode_mapping: "Core content_store.rs 0600 maps to Git100644; original native mode checked before capture" }));
  materializeCore(root, source.name);
  const cwd = path.join(root, source.name), owned = path.join(process.env.RUNNER_TEMP, `ait-affected-${consumer}`);
  mkdirSync(owned, { recursive: true });
  const env = { ...process.env, SOURCE_DATE_EPOCH: source.epoch, PYTHONNOUSERSITE: "1", PYTHONDONTWRITEBYTECODE: "1", CARGO_INCREMENTAL: "0" };
  delete env.PYTHONPATH;
  const results = new Map();
  async function run(name, command, args, needs = [], options = {}) {
    if (needs.some(dependency => results.get(dependency) !== "pass")) {
      results.set(name, "blocked"); console.log(JSON.stringify({ step: name, status: "blocked", needs })); return;
    }
    console.log(`::group::${name}`);
    let result;
    try {
      if (command === "npm") {
        const { spawnNpmSync } = await import(pathToFileURL(path.join(root, "ait-node/scripts/npm-command.mjs")));
        result = spawnNpmSync(args, { cwd, env, stdio: "inherit" });
      } else result = spawnSync(command, args, { cwd, env, stdio: "inherit", ...options });
      assert.equal(result.status, 0, result.error?.message || `exit ${result.status}`);
      results.set(name, "pass");
    } catch (error) { results.set(name, "fail"); console.error(error.message); }
    console.log("::endgroup::");
    console.log(JSON.stringify({ step: name, status: results.get(name) }));
  }
  await run("NOTICE", "bash", ["ci/generate_notice.sh", "--check"]);
  if (consumer === "node") {
    const adapter = "release/release-adapter.mjs";
    await run("addon build", process.execPath, [adapter, "build", target, "1.2.1"]);
    await run("JS CLI and typed N-API suites", "npm", ["test"], ["addon build"]);
    await run("JS syntax", "npm", ["run", "check"]);
    await run("portable build", process.execPath, [adapter, "build", "portable", "1.2.1"]);
    await run("portable installed boundaries", process.execPath, [adapter, "smoke", "portable", "1.2.1"], ["portable build"]);
    await run("addon installed boundaries", process.execPath, [adapter, "smoke", target, "1.2.1"], ["addon build", "portable build"]);
    const tools = path.join(owned, "typescript"), installed = path.join(owned, "consumer");
    mkdirSync(installed, { recursive: true });
    copyFileSync(path.join(root, "ci/release_affected_consumer.mts"), path.join(installed, "consumer.mts"));
    const payload = JSON.parse(readFileSync(path.join(cwd, "lib/npm-payload-contract.json"), "utf8")).payloads.find(row => row.target === target);
    assert(payload, "missing exact target addon");
    const portable = path.join(cwd, "dist/wa120-ait-native-1.2.1.tgz");
    const addon = path.join(cwd, "dist/npm-addons", `${payload.package.replace(/^@/, "").replace("/", "-")}-1.2.1.tgz`);
    await run("pinned TypeScript tools", "npm", ["install", "--ignore-scripts", "--no-audit", "--no-fund", "--prefix", tools, "typescript@5.9.3", "@types/node@25.9.5"]);
    await run("offline exact typed consumer install", "npm", ["install", "--ignore-scripts", "--offline", "--no-audit", "--no-fund", "--no-save", "--prefix", installed, portable, addon], ["portable build", "addon build"]);
    await run("strict installed TypeScript", process.execPath, [path.join(tools, "node_modules/typescript/lib/tsc.js"), "--strict", "--module", "NodeNext", "--moduleResolution", "NodeNext", "--target", "ES2022", "--esModuleInterop", "--types", "node", "--typeRoots", path.join(tools, "node_modules/@types"), "--outDir", "compiled", "consumer.mts"], ["pinned TypeScript tools", "offline exact typed consumer install"], { cwd: installed });
    await run("compiled installed TS and direct addon", process.execPath, ["compiled/consumer.mjs", payload.package], ["strict installed TypeScript"], { cwd: installed });
  } else if (consumer === "python") {
    const adapter = "release/release_adapter.py";
    await run("locked PyO3 ABI check", "python", [adapter, "check", target, "1.2.1"]);
    await run("cp311-abi3 wheel build and payload", "python", [adapter, "build", target, "1.2.1"], ["locked PyO3 ABI check"]);
    await run("clean wheel CLI/PyO3 smoke", "python", [adapter, "smoke", target, "1.2.1"], ["cp311-abi3 wheel build and payload"]);
    const venv = path.join(owned, "tests"), python = path.join(venv, process.platform === "win32" ? "Scripts/python.exe" : "bin/python");
    await run("test environment", "python", ["-m", "venv", venv]);
    const wheels = existsSync(path.join(cwd, "dist/wheels")) ? readdirSync(path.join(cwd, "dist/wheels")).filter(name => name.endsWith(".whl")) : [];
    if (results.get("cp311-abi3 wheel build and payload") === "pass") assert.equal(wheels.length, 1);
    await run("install exact wheel and declared test extra", python, ["-m", "pip", "install", "--no-cache-dir", `${path.join(cwd, "dist/wheels", wheels[0] ?? "missing.whl")}[test]`], ["cp311-abi3 wheel build and payload", "test environment"]);
    await run("complete Python suites against installed wheel", python, ["-m", "pytest", "-p", "no:cacheprovider"], ["install exact wheel and declared test extra"]);
    await run("installed dependency check", python, ["-m", "pip", "check"], ["install exact wheel and declared test extra"]);
  } else {
    if (process.platform === "win32") await run("Runner full platform contracts", "pwsh", ["-NoProfile", "-File", "ci/run.ps1", "all"]);
    else await run("Runner full platform contracts", "bash", ["ci/run.sh", "all"]);
    env.CARGO_TARGET_DIR = path.join(owned, "core-archive");
    await run("new Core archive contracts", "cargo", ["test", "--locked", "--manifest-path", ".ait-external/ait-core/rust/Cargo.toml", "-p", "ait-core", "--lib", "archive", "--no-fail-fast"]);
    env.CARGO_TARGET_DIR = path.join(owned, "binary");
    if (platform.os === "macos") env.MACOSX_DEPLOYMENT_TARGET = platform.minimum_platform;
    await run("Runner target binary", "cargo", ["build", "--locked", "--release", "--target", target]);
    const binary = path.join(env.CARGO_TARGET_DIR, target, "release", `ait-runner${platform.executable_suffix}`);
    await run("Runner version", process.execPath, ["-e", "const r=require('node:child_process').spawnSync(process.argv[1],['--version'],{encoding:'utf8'});process.stdout.write(r.stdout||'');if(r.status!==0||!/(^|\\s)1\\.2\\.1(\\s|$)/.test(r.stdout))process.exit(1)", binary], ["Runner target binary"]);
  }
  await run("tracked source unchanged", "git", ["diff", "--exit-code", "--", "ait-core", "ait-server", "ait-node", "ait-python", "ait-runner"], [], { cwd: root });
  console.log(JSON.stringify({ consumer, target, results: Object.fromEntries(results), qualification: "preparation only" }));
  return [...results.values()].every(result => result === "pass") ? 0 : 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  execute(root, process.argv[2], process.argv[3]).then(code => { process.exitCode = code; }).catch(error => { console.error(error); process.exitCode = 1; });
}
