// Finite reviewed supplemental locations. Selection is not admission.
import assert from "node:assert/strict";
import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

export function recoveryLocations(priorInputPath, repository, selectedWT = null) {
  const original = path.dirname(path.dirname(priorInputPath));
  const root = path.join(path.dirname(original), "records-stage5-composition-20261101");
  const collection = path.join(root, "web-admission/results/randomized_3000-collection-1");
  if (!existsSync(collection)) return null;
  const file = path.join(collection, "collector.argv"), stat = lstatSync(file);
  assert(stat.isFile() && !stat.isSymbolicLink());
  const argv = readFileSync(file, "utf8").split("\n"); assert.equal(argv.pop(), "");
  assert.deepEqual(argv.slice(0, 3), ["sh", "-s", "--"]);
  assert.equal(argv[4], repository); assert.equal(argv[6], collection); assert.equal(argv[7], priorInputPath);
  const priorRoots = [path.join(repository, ".ait-runtime/stage5-random-HZU51i/run"),
    ...[8, 10, 12, 13].map(index => path.join(repository, `.ait-runtime/stage5-replay-20261101-case${index}`))];
  assert.deepEqual(argv.slice(8), priorRoots, "reviewed random reuse inventory differs");
  const ownerDirectory = path.join(original, "conductor-logs"), prefix = "wt-owner-positive";
  const ownerFiles = existsSync(ownerDirectory) ? readdirSync(ownerDirectory).filter(name => name.startsWith(prefix + ".")).sort().map(name => path.join(ownerDirectory, name)) : [];
  for (const number of [1, 2]) for (const suffix of ["argv", "stdout", "stderr"])
    assert(ownerFiles.includes(path.join(ownerDirectory, `${prefix}.init-${number}.${suffix}`)), "reviewed owner-positive raw is incomplete");
  const forkPath = path.join(root, "web-admission/results/native.current.reviewed-fork-1");
  const finalPath = path.join(root, "web-admission/results/closeout-1");
  const fork = existsSync(forkPath) ? forkPath : null;
  const final = fork && existsSync(finalPath) ? finalPath : null;
  const runtimeFiles = [];
  if (fork) {
    assert.match(selectedWT ?? "", /^SNP-[0-9A-F]{12}$/);
    const runtime = path.join(root, "runtime-wt-" + selectedWT.slice(4));
    for (const file of ["accepted-input.json", "web-components.json", "web-admission/qualification-input.json",
      ...["starting-config.json", "starting-status.json", "starting-queue.json", "starting-worktrees.json", "remote-repository.json", "acceptance-set.sha256"]
        .map(name => "web-admission/evidence/" + name)]) {
      const value = path.join(runtime, file), stat = lstatSync(value);
      assert(stat.isFile() && !stat.isSymbolicLink(), "fork needs its original normally produced input/entry captures"); runtimeFiles.push(value);
    }
  }
  return { root, collection, priorRoots, ownerFiles, fork, final, runtimeFiles,
    initial: path.join(root, "web-admission/results/ait.semantic.init-wt-1"),
    dual: path.join(root, "web-admission/results/native.current.two-authorities") };
}

export const recoveryEvidence = recovery => recovery ? [recovery.initial, recovery.dual, recovery.collection,
  ...[recovery.fork, recovery.final].filter(Boolean), ...recovery.priorRoots, ...recovery.ownerFiles, ...recovery.runtimeFiles] : [];
