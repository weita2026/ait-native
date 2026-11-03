import assert from "node:assert/strict";
import * as fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { dependencyTreeDigest, runnerCoreDeclaration, validateRunnerDependencySource } from "./release_runner_dependency.mjs";

const root = fs.mkdtempSync(path.join(os.tmpdir(), "ait-runner-source-test."));
try {
  const runner = path.join(root, "ait-runner"), main = path.join(root, "ait-core");
  const dependency = path.join(runner, ".ait-external/ait-core");
  fs.mkdirSync(main, { recursive: true }); fs.mkdirSync(dependency, { recursive: true });
  for (const [filename, table] of [["ait-external.toml", "external"], ["ait-external.lock", "node"]]) {
    fs.writeFileSync(path.join(runner, filename), `[[${table}]]\nname = "ait-core"\nrepo_name = "ait-core"\nrepository_index = 0\nsnapshot = "SNP-BBBBBBBBBBBB"\nmaterialize_to = ".ait-external/ait-core"\nlicense = "Apache-2.0"\nversion = "1.1.4"\n`);
  }
  const evidence = { contract: "ait.release.source-cache/v1", status: "ready", repo_name: "ait-core", repository_index: 0,
    source_snapshot: "SNP-BBBBBBBBBBBB", source_manifest_hash: "b".repeat(64), source_snapshot_created_at: "1700000000",
    version: "1.1.4", license: "Apache-2.0", source_authority: "ait_remote_snapshot_boundary", workspace_clean: true,
    remote_coordinates_embedded: false, public_publish: false, materialize_to: ".ait-external/ait-core" };
  const metadata = path.join(dependency, "source-cache.evidence.json");
  fs.writeFileSync(metadata, JSON.stringify(evidence));
  fs.writeFileSync(path.join(main, "source.rs"), "main Core\n");
  fs.writeFileSync(path.join(dependency, "source.rs"), "declared Core B1\n");
  const cargo = value => fs.writeFileSync(path.join(runner, "Cargo.toml"), `[dependencies]\nait-core = { path = "${value}" }\n`);
  const lockedPath = ".ait-external/ait-core/rust/crates/ait-core", sharedPath = "../ait-core/rust/crates/ait-core";
  cargo(lockedPath);
  const before = dependencyTreeDigest(dependency);
  assert.equal(validateRunnerDependencySource(runner, main).equivalent, false, "distinct accepted source remains valid");
  assert.equal(dependencyTreeDigest(dependency), before, "validation must preserve immutable source bytes");
  cargo(sharedPath);
  assert.throws(() => validateRunnerDependencySource(runner, main), /does not preserve/);
  cargo(lockedPath);
  fs.writeFileSync(metadata, JSON.stringify({ ...evidence, source_snapshot: "SNP-CCCCCCCCCCCC" }));
  assert.throws(() => validateRunnerDependencySource(runner, main), /provenance differs/);
  fs.writeFileSync(metadata, JSON.stringify(evidence));
  fs.mkdirSync(path.join(dependency, ".ait"));
  assert.throws(() => validateRunnerDependencySource(runner, main), /operational state/);
  fs.rmSync(path.join(dependency, ".ait"), { recursive: true });
  fs.writeFileSync(path.join(dependency, "source.rs"), "main Core\n");
  cargo(sharedPath);
  assert.equal(validateRunnerDependencySource(runner, main).equivalent, true, "shared path requires actual byte/mode equivalence");
  fs.chmodSync(path.join(dependency, "source.rs"), 0o755);
  assert.throws(() => validateRunnerDependencySource(runner, main), /does not preserve/, "different executable modes are not equivalent");
  fs.chmodSync(path.join(dependency, "source.rs"), 0o644);
  const lock = path.join(runner, "ait-external.lock");
  fs.appendFileSync(lock, '\n[[node]]\nname = "ait-core"\nparent_path = ""\nsnapshot = "SNP-CCCCCCCCCCCC"\n');
  assert.throws(() => runnerCoreDeclaration(runner), /exactly one/);
} finally { fs.rmSync(root, { recursive: true, force: true }); }
console.log("Runner immutable dependency declarations, equivalence and path guards: pass");
