import assert from "node:assert/strict";
import { gzipSync } from "node:zlib";
import test from "node:test";

import {
  releaseCommandEnvironment,
  validateWindowsReceiptArtifact,
} from "./build-release.mjs";

const WINDOWS_TARGETS = [
  "aarch64-pc-windows-msvc",
  "x86_64-pc-windows-msvc",
];

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
    declared_path: "ait-native-ait-win32-x64.tgz",
  };
  assert.throws(
    () =>
      validateWindowsReceiptArtifact(
        tarGzip("package/native/ait_napi.node", fakePe("MSVCP140_ATOMIC_WAIT.dll")),
        row,
        "ait-native-ait-win32-x64.tgz",
      ),
    /dynamically imports MSVCP140_ATOMIC_WAIT\.dll/u,
  );
  assert.doesNotThrow(() =>
    validateWindowsReceiptArtifact(
      tarGzip("package/native/ait_napi.node", fakePe("KERNEL32.dll")),
      row,
      "ait-native-ait-win32-x64.tgz",
    ),
  );
});
