// Existing release identities only. A reviewed WT assertion correction is not
// a replacement product candidate or permission to replace the old W carrier.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { isDeepStrictEqual } from "node:util";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const read = file => {
  const stat = lstatSync(file); assert(stat.isFile() && !stat.isSymbolicLink(), `recovery input must be regular: ${file}`);
  return readFileSync(file);
};
const json = file => JSON.parse(read(file));
export const productComponents = ["core", "server", "runner", "node", "python"];

export function priorWebInput(nativeRoot, release) {
  if (release.version !== "1.2.0" || release.channel !== "stable") return null;
  const workOrder = path.join(nativeRoot, "docs/sprints/release_1_2_0_binding.md");
  if (!existsSync(workOrder)) return null;
  const prose = read(workOrder).toString(), heading = "Stage 5 assertion disposition — 2026-11-01";
  assert.equal(prose.split(heading).length, 2, "reviewed Stage 5 disposition is missing or ambiguous");
  const section = prose.slice(prose.indexOf(heading)), locations = [...section.matchAll(/^- W: `([^`]+)`/gm)];
  assert.equal(locations.length, 1, "reviewed W location is missing or ambiguous");
  const inputPath = path.join(locations[0][1], "qualification-input.json"), bytes = read(inputPath), input = JSON.parse(bytes);
  assert(section.includes(sha(bytes)), "reviewed W input identity differs");
  assert.equal(input.contract, "ait.release.web-admission-input/v1");
  assert.equal(input.release.version, release.version); assert.equal(input.release.channel, release.channel);
  assert.deepEqual(Object.keys(input.executables).sort(), ["ait", "server"]);
  assert.deepEqual(input.browser_evidence, {});
  return { inputPath, input, root: path.dirname(path.dirname(inputPath)), workOrder };
}

export function sameProductComponents(selected, prior) {
  for (const key of productComponents) assert.equal(selected[key], prior.components[key], `selected component cohort differs: ${key} product Snapshot changed`);
  assert.deepEqual(Object.keys(selected).sort(), Object.keys(prior.components).sort(), "selected product inventory changed");
}

function command(cli, repository, argv) {
  const result = spawnSync(cli, argv, { cwd: repository, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  assert.equal(result.status, 0, `WT assertion identity cannot be verified: ${result.stderr}`);
  return JSON.parse(result.stdout);
}

export function correctedWTIdentity(accepted, prior, components = json(accepted.paths.web_components)) {
  sameProductComponents(components, prior);
  assert.equal(components.web_test, accepted.web.acceptance_snapshot, "selected WT Snapshot differs from accepted input");
  if (components.web_test === prior.components.web_test) return null;
  assert.match(components.web_test, /^SNP-[0-9A-F]{12}$/);
  const repository = accepted.paths.web_test_root, cli = prior.executables.ait.path;
  assert.equal(sha(read(cli)), prior.executables.ait.sha256, "WT identity reader candidate changed");
  const status = command(cli, repository, ["status", "--json"]);
  assert.equal(status.repo_name, "ait-web-test"); assert.equal(status.line_name, "main");
  assert.equal(status.head_snapshot_id, components.web_test); assert.equal(status.worktree, null);
  assert.equal(status.workspace?.dirty, false); assert.equal(status.workspace?.changed_count, 0);
  const diff = command(cli, repository, ["snapshot", "diff", prior.components.web_test, components.web_test, "--json"]);
  assert.equal(diff.old_snapshot_id, prior.components.web_test); assert.equal(diff.new_snapshot_id, components.web_test);
  const allowed = new Set(["acceptance/run-randomized-commands.sh", "acceptance/randomized-self-test.sh", "acceptance/randomized-collection-self-test.sh"]);
  assert(Array.isArray(diff.files) && diff.files.length > 0);
  assert.equal(new Set(diff.files.map(row => row.path)).size, diff.files.length);
  for (const row of diff.files) {
    assert(allowed.has(row.path), `WT correction changes an unreviewed path: ${row.path}`);
    assert(["modified", "added"].includes(row.status), "WT correction deletes or renames source");
    if (row.status === "added") assert(/test/.test(path.basename(row.path)), "WT correction adds executable surface");
  }
  for (const required of allowed)
    assert(diff.files.some(row => row.path === required), `WT correction omits the reviewed assertion repair: ${required}`);
  assert.deepEqual(diff.mode_changed, [], "WT correction changes existing executable modes");
  // The complete Snapshot diff excludes matrix, generator, scenario weights,
  // exclusions and retired paths. Recompute the current tree for normal input
  // production and pin every exact source file; no stored digest is edited.
  const files = {}, tree = createHash("sha256");
  const walk = (directory, relative = "") => {
    for (const name of readdirSync(directory).sort()) {
      const file = path.join(directory, name), next = path.join(relative, name), stat = lstatSync(file);
      assert(!stat.isSymbolicLink(), "corrected WT source contains a symlink");
      if (stat.isDirectory()) walk(file, next);
      else { const bytes = read(file); files[file] = sha(bytes); tree.update(next).update("\0").update(bytes).update("\0"); }
    }
  };
  walk(path.join(repository, "acceptance"));
  return { snapshot: components.web_test, acceptanceSha256: tree.digest("hex"), files, diff };
}

export function qualificationCarrier(accepted) {
  const root = path.dirname(accepted.paths.web_components);
  if (path.resolve(root) !== path.resolve(accepted.records_root)) return root;
  const prior = priorWebInput(accepted.paths.native_root, accepted.release);
  if (!prior || accepted.web.acceptance_snapshot === prior.input.components.web_test) return null;
  const selected = json(accepted.paths.web_components);
  // A different product cohort follows normal qualification. Only this exact
  // stopped candidate may inherit its prior Q/E carrier for a WT-only repair.
  if (productComponents.some(key => selected[key] !== prior.input.components[key])) return null;
  const previousAccepted = path.join(prior.root, "accepted-input.json");
  if (!existsSync(previousAccepted)) return null;
  {
    const previous = json(previousAccepted);
    if (!isDeepStrictEqual(previous.release, accepted.release) ||
        !isDeepStrictEqual(previous.component_closeouts, accepted.component_closeouts) ||
        !["native_root", "core_root", "public_source", "web_test_root"].every(key => previous.paths[key] === accepted.paths[key]) ||
        !isDeepStrictEqual({ ...previous.web, acceptance_snapshot: accepted.web.acceptance_snapshot }, accepted.web)) return null;
  }
  correctedWTIdentity(accepted, prior.input);
  return prior.root;
}
