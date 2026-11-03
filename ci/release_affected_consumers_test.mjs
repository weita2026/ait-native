import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, mkdtempSync, mkdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { inventoryDigest, verifyControlIdentity, verifyCoreMarker, verifySource } from "./release_affected_consumers.mjs";

test("frozen source verification rejects changed bytes, membership, types and executable modes", () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "ait-affected-source-test-"));
  try {
    const folder = path.join(root, "component"); mkdirSync(folder);
    writeFileSync(path.join(folder, "source.rs"), "immutable source\n");
    const git = args => { const r = spawnSync("git", ["-C", root, ...args]); assert.equal(r.status, 0, r.stderr.toString()); };
    git(["init"]); git(["add", "component/source.rs"]);
    const source = { name: "component", count: 1, tree: inventoryDigest([{ path: "source.rs", mode: "100644", sha256: createHash("sha256").update("immutable source\n").digest("hex") }]) };
    assert.equal(verifySource(root, source).length, 1);
    writeFileSync(path.join(folder, "source.rs"), "changed source\n");
    assert.throws(() => verifySource(root, source), /full Snapshot bytes/);
    writeFileSync(path.join(folder, "source.rs"), "immutable source\n");
    writeFileSync(path.join(folder, "extra.rs"), "unreviewed\n");
    assert.throws(() => verifySource(root, source), /extra\/missing/);
    rmSync(path.join(folder, "extra.rs"));
    if (process.platform !== "win32") {
      chmodSync(path.join(folder, "source.rs"), 0o755);
      assert.throws(() => verifySource(root, source), /source mode/);
      chmodSync(path.join(folder, "source.rs"), 0o644);
      rmSync(path.join(folder, "source.rs"));
      symlinkSync("../elsewhere", path.join(folder, "source.rs"));
      assert.throws(() => verifySource(root, source), /source symlink/);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("native marker v3 binds paths/hashes while validated Snapshot rows supply modes", () => {
  const core = { snapshot: "SNP-DE9A9BA9465D", count: 1 };
  const rows = [{ path: "ci/run.sh", sha256: "a".repeat(64), mode: "100755" }];
  const marker = { format: "ait.external.materialized", version: 3, name: "ait-core", repo_name: "ait-core", repository_index: 0, remote: "origin", snapshot: core.snapshot, parent_path: "", materialize_to: ".ait-external/ait-core", files: [{ path: rows[0].path, sha256: rows[0].sha256 }] };
  assert.doesNotThrow(() => verifyCoreMarker(marker, core, rows));
  assert.throws(() => verifyCoreMarker({ ...marker, repository_index: 3 }, core, rows), /repository_index/);
  assert.throws(() => verifyCoreMarker({ ...marker, files: [{ path: "other.sh", sha256: rows[0].sha256 }] }, core, rows), /differs from full selected/);
});

test("committed control identity accepts the exact Snapshot and refuses a valid stale input before consumer work", () => {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "ait-affected-control-test-")));
  try {
    mkdirSync(path.join(root, "ci"));
    copyFileSync(new URL("./release_affected_consumers.mjs", import.meta.url), path.join(root, "ci/release_affected_consumers.mjs"));
    copyFileSync(new URL("./native_bootstrap_matrix.json", import.meta.url), path.join(root, "ci/native_bootstrap_matrix.json"));
    const git = args => {
      const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
    };
    git(["init"]); git(["add", "ci"]);
    const snapshot = "SNP-AAAAAAAAAAAA", manifest = "a".repeat(64);
    git(["-c", "user.name=AIT Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false", "commit", "-m", `Frozen control\n\ncontrol ${snapshot} ${manifest} (2 files)`]);
    assert.deepEqual(verifyControlIdentity(root, snapshot), { snapshot, manifest, files: 2 });
    assert.throws(() => verifyControlIdentity(root, "SNP-1FED647C7C38"), /coordinator_snapshot differs/);
    const target = { darwin: { arm64: "aarch64-apple-darwin", x64: "x86_64-apple-darwin" }, linux: { arm64: "aarch64-unknown-linux-gnu", x64: "x86_64-unknown-linux-gnu" }, win32: { arm64: "aarch64-pc-windows-msvc", x64: "x86_64-pc-windows-msvc" } }[process.platform]?.[process.arch];
    assert(target);
    const result = spawnSync(process.execPath, [path.join(root, "ci/release_affected_consumers.mjs"), "runner", target], {
      cwd: root, encoding: "utf8", env: { ...process.env, GITHUB_EVENT_NAME: "workflow_dispatch", GITHUB_SHA: git(["rev-parse", "HEAD"]), DIAGNOSTIC_AFFECTED_CONSUMERS: "true", AIT_CONTROL_SNAPSHOT: "SNP-1FED647C7C38", RUNNER_TEMP: root },
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /coordinator_snapshot differs from frozen carrier Native control/);
    assert.doesNotMatch(result.stdout, /control_snapshot|::group::|Runner full platform contracts|NOTICE/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
