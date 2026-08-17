import assert from "node:assert/strict";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { gzipSync } from "node:zlib";
import test from "node:test";

import {
  cleanLocalNodeBuildTransients,
  releaseCommandEnvironment,
  validateNpmAddonContract,
  validateWindowsReceiptArtifact,
} from "./build-release.mjs";

const WINDOWS_TARGETS = [
  "aarch64-pc-windows-msvc",
  "x86_64-pc-windows-msvc",
];

function npmAddonContract() {
  return {
    schema: "ait.node.napi-platform-packages/v2",
    family_version: "1.0.0",
    top_level_package: "@wa120/ait-native",
    payloads: [
      ["aarch64-apple-darwin", "darwin", "arm64", null],
      ["x86_64-apple-darwin", "darwin", "x64", null],
      ["aarch64-unknown-linux-gnu", "linux", "arm64", "glibc"],
      ["x86_64-unknown-linux-gnu", "linux", "x64", "glibc"],
      ["aarch64-pc-windows-msvc", "win32", "arm64", null],
      ["x86_64-pc-windows-msvc", "win32", "x64", null],
    ].map(([target, osName, cpu, libc]) => ({
      target,
      os: osName,
      cpu,
      libc,
      component: "ait-node",
      package: `@wa120/ait-native-${osName}-${cpu}`,
      version: "1.0.0",
      binding_repository: "ait-core",
      binding_snapshot: "SNP-AAAAAAAAAAAA",
      license: "Apache-2.0",
      addon: "native/ait_napi.node",
    })),
  };
}

test("public source contract requires exact GNU libc admission", () => {
  const contract = npmAddonContract();
  assert.doesNotThrow(() => validateNpmAddonContract(contract, "1.0.0"));

  for (const mutate of [
    (value) => delete value.payloads[2].libc,
    (value) => {
      value.payloads[2].libc = null;
    },
    (value) => {
      value.payloads[2].libc = "musl";
    },
    (value) => {
      value.payloads[0].libc = "glibc";
    },
  ]) {
    const invalid = structuredClone(contract);
    mutate(invalid);
    assert.throws(
      () => validateNpmAddonContract(invalid, "1.0.0"),
      /target, libc, or binding metadata is not exact/u,
    );
  }
});

test("local source builds remove exact outputs without deleting admitted Node.js dist", async () => {
  const sourceRoot = await mkdtemp(path.join(os.tmpdir(), "ait-source-cleanup-test-"));
  try {
    const nodeRoot = path.join(sourceRoot, "ait-node");
    const retained = path.join(nodeRoot, "src", "index.js");
    await mkdir(path.dirname(retained), { recursive: true });
    await writeFile(retained, "export const retained = true;\n");
    for (const [directory, file] of [
      [".ait-native-target", "release/libait_napi.dylib"],
      ["native", "ait_napi.node"],
      ["dist", "wa120-ait-native-1.0.0-rc.7.tgz"],
    ]) {
      const generated = path.join(nodeRoot, directory, file);
      await mkdir(path.dirname(generated), { recursive: true });
      await writeFile(generated, "generated\n");
    }
    const admittedDist = path.join(nodeRoot, "dist", "REL-ADMITTED", "ait-release.manifest.json");
    await mkdir(path.dirname(admittedDist), { recursive: true });
    await writeFile(admittedDist, "admitted\n");

    await cleanLocalNodeBuildTransients("1.0.0-rc.7", sourceRoot);

    assert.equal(await readFile(retained, "utf8"), "export const retained = true;\n");
    await assert.rejects(lstat(path.join(nodeRoot, ".ait-native-target")), { code: "ENOENT" });
    await assert.rejects(lstat(path.join(nodeRoot, "native", "ait_napi.node")), { code: "ENOENT" });
    await assert.rejects(
      lstat(path.join(nodeRoot, "dist", "wa120-ait-native-1.0.0-rc.7.tgz")),
      { code: "ENOENT" },
    );
    assert.equal(await readFile(admittedDist, "utf8"), "admitted\n");
  } finally {
    await rm(sourceRoot, { recursive: true, force: true });
  }
});

function fakePe(...imports) {
  return Buffer.concat([
    Buffer.from("MZ", "ascii"),
    Buffer.alloc(62),
    Buffer.from(imports.join("\0"), "ascii"),
  ]);
}

function storedZip(name, bytes) {
  const nameBytes = Buffer.from(name, "utf8");
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4);
  local.writeUInt32LE(bytes.length, 18);
  local.writeUInt32LE(bytes.length, 22);
  local.writeUInt16LE(nameBytes.length, 26);

  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4);
  central.writeUInt16LE(20, 6);
  central.writeUInt32LE(bytes.length, 20);
  central.writeUInt32LE(bytes.length, 24);
  central.writeUInt16LE(nameBytes.length, 28);

  const centralOffset = local.length + nameBytes.length + bytes.length;
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(1, 8);
  eocd.writeUInt16LE(1, 10);
  eocd.writeUInt32LE(central.length + nameBytes.length, 12);
  eocd.writeUInt32LE(centralOffset, 16);
  return Buffer.concat([local, nameBytes, bytes, central, nameBytes, eocd]);
}

function tarGzip(name, bytes) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  header.write(bytes.length.toString(8).padStart(11, "0"), 124, 11, "ascii");
  header[135] = 0;
  header[156] = 0x30;
  const padding = Buffer.alloc(Math.ceil(bytes.length / 512) * 512 - bytes.length);
  return gzipSync(Buffer.concat([header, bytes, padding, Buffer.alloc(1024)]));
}

test("Windows receipt commands force the exact target-scoped static CRT flag", () => {
  for (const target of WINDOWS_TARGETS) {
    const key = `CARGO_TARGET_${target.toUpperCase().replaceAll("-", "_")}_RUSTFLAGS`;
    const environment = releaseCommandEnvironment(
      { AIT_RELEASE_TARGET: target },
      {
        PATH: "/fixture",
        RUSTFLAGS: "-Cdebuginfo=2",
        CARGO_ENCODED_RUSTFLAGS: "-C\u001fopt-level=0",
        [key]: "-Ctarget-feature=-crt-static",
      },
    );
    assert.equal(environment[key], "-Ctarget-feature=+crt-static");
    assert.equal(environment.RUSTFLAGS, undefined);
    assert.equal(environment.CARGO_ENCODED_RUSTFLAGS, undefined);
  }
});

test("non-Windows receipt commands preserve caller Rust flags", () => {
  const environment = releaseCommandEnvironment(
    { AIT_RELEASE_TARGET: "x86_64-unknown-linux-gnu" },
    { RUSTFLAGS: "-Cdebuginfo=1" },
  );
  assert.equal(environment.RUSTFLAGS, "-Cdebuginfo=1");
});

test("Windows native executable receipts reject a dynamic MSVC runtime", () => {
  assert.throws(
    () =>
      validateWindowsReceiptArtifact(
        fakePe("KERNEL32.dll", "VCRUNTIME140.dll"),
        {
          target: "x86_64-pc-windows-msvc",
          kind: "native-executable",
          declared_path: "ait.exe",
        },
        "ait.exe",
      ),
    /dynamically imports VCRUNTIME140\.dll/u,
  );
  assert.doesNotThrow(() =>
    validateWindowsReceiptArtifact(
      fakePe("KERNEL32.dll", "api-ms-win-crt-runtime-l1-1-0.dll"),
      {
        target: "x86_64-pc-windows-msvc",
        kind: "native-executable",
        declared_path: "ait.exe",
      },
      "ait.exe",
    ),
  );
});

test("Windows Python wheels inspect their packaged pyd", () => {
  const row = {
    target: "aarch64-pc-windows-msvc",
    kind: "python-wheel",
    declared_path: "ait_native.whl",
  };
  assert.throws(
    () =>
      validateWindowsReceiptArtifact(
        storedZip("ait_native/ait_native.pyd", fakePe("VCRUNTIME140_1.dll")),
        row,
        "ait_native.whl",
      ),
    /dynamically imports VCRUNTIME140_1\.dll/u,
  );
  assert.doesNotThrow(() =>
    validateWindowsReceiptArtifact(
      storedZip("ait_native/ait_native.pyd", fakePe("KERNEL32.dll")),
      row,
      "ait_native.whl",
    ),
  );
});

test("Windows npm addon archives inspect their packaged node binary", () => {
  const row = {
    target: "x86_64-pc-windows-msvc",
    kind: "npm-napi-addon",
    declared_path: "wa120-ait-native-win32-x64.tgz",
  };
  assert.throws(
    () =>
      validateWindowsReceiptArtifact(
        tarGzip("package/native/ait_napi.node", fakePe("MSVCP140_ATOMIC_WAIT.dll")),
        row,
        "wa120-ait-native-win32-x64.tgz",
      ),
    /dynamically imports MSVCP140_ATOMIC_WAIT\.dll/u,
  );
  assert.doesNotThrow(() =>
    validateWindowsReceiptArtifact(
      tarGzip("package/native/ait_napi.node", fakePe("KERNEL32.dll")),
      row,
      "wa120-ait-native-win32-x64.tgz",
    ),
  );
});
