import assert from "node:assert/strict";
import { isUtf8 } from "node:buffer";
import { lstat, readFile, readdir } from "node:fs/promises";
import path from "node:path";

export const ENVELOPE_FILES = Object.freeze([
  "LICENSE", "NOTICE", "README.md", "bin/ait.mjs",
  "lib/npm-payload-contract.json", "package.json",
  "src/agent.js", "src/contract.js", "src/errors.js", "src/index.d.ts",
  "src/index.js", "src/runtime.js", "src/terminal.js", "src/terminal-help.js",
]);

export const ADDON_FILES = Object.freeze([
  "LICENSE", "NOTICE", "native/ait_napi.node", "package.json", "provenance.json",
]);

function range(bytes, offset, size, label) {
  assert.ok(Number.isSafeInteger(offset) && offset >= 0 && size >= 0 && offset + size <= bytes.length,
    `truncated native image: ${label}`);
}

function imageKind(bytes) {
  if (bytes.length < 4) return undefined;
  const magic = bytes.readUInt32BE(0);
  if (magic === 0x7f454c46) return { format: "ELF" };
  const mach = new Map([
    [0xfeedface, { little: false, wide: false }], [0xcefaedfe, { little: true, wide: false }],
    [0xfeedfacf, { little: false, wide: true }], [0xcffaedfe, { little: true, wide: true }],
  ]).get(magic);
  if (mach) return { format: "Mach-O", ...mach };
  const fat = new Map([
    [0xcafebabe, { little: false, wide: false }], [0xbebafeca, { little: true, wide: false }],
    [0xcafebabf, { little: false, wide: true }], [0xbfbafeca, { little: true, wide: true }],
  ]).get(magic);
  if (fat) return { format: "Mach-O fat", ...fat };
  if (bytes[0] === 0x4d && bytes[1] === 0x5a && bytes.length >= 64) {
    const offset = bytes.readUInt32LE(60);
    if (offset >= 64 && offset <= bytes.length - 4 && bytes.readUInt32LE(offset) === 0x00004550) return { format: "PE", offset };
  }
  return undefined;
}

function assertModuleImage(bytes, label, machOnly = false) {
  const kind = imageKind(bytes);
  assert.ok(kind && (!machOnly || kind.format === "Mach-O"), `packaged addon must declare a loadable native image: ${label}`);
  const u16 = (offset, little) => little ? bytes.readUInt16LE(offset) : bytes.readUInt16BE(offset);
  const u32 = (offset, little) => little ? bytes.readUInt32LE(offset) : bytes.readUInt32BE(offset);
  const u64 = (offset, little) => little ? bytes.readBigUInt64LE(offset) : bytes.readBigUInt64BE(offset);
  if (kind.format === "ELF") {
    range(bytes, 0, 16, label);
    const wide = bytes[4] === 2;
    assert.ok([1, 2].includes(bytes[4]) && [1, 2].includes(bytes[5]) && bytes[6] === 1, `invalid ELF identification: ${label}`);
    const little = bytes[5] === 1;
    range(bytes, 0, wide ? 64 : 52, label);
    assert.equal(u32(20, little), 1, `invalid ELF header version: ${label}`);
    assert.ok(u16(wide ? 52 : 40, little) >= (wide ? 64 : 52), `invalid ELF header size: ${label}`);
    assert.equal(u16(16, little), 3, `standalone ELF executable is forbidden: ${label}`);
    const entry = wide ? u64(24, little) : BigInt(u32(24, little));
    assert.equal(entry, 0n, `ELF process entrypoint is forbidden: ${label}`);
    const offset = wide ? u64(32, little) : BigInt(u32(28, little));
    const size = u16(wide ? 54 : 42, little), count = u16(wide ? 56 : 44, little);
    if (count > 0) {
      assert.ok(size >= (wide ? 56 : 32), `invalid ELF program headers: ${label}`);
      assert.ok(offset <= BigInt(bytes.length), `truncated ELF program headers: ${label}`);
      range(bytes, Number(offset), size * count, label);
      for (let index = 0; index < count; index++) {
        assert.notEqual(u32(Number(offset) + index * size, little), 3, `ELF process interpreter is forbidden: ${label}`);
      }
    }
    return;
  }
  if (kind.format === "PE") {
    range(bytes, kind.offset, 26, label);
    const characteristics = bytes.readUInt16LE(kind.offset + 22);
    assert.ok((characteristics & 0x2000) !== 0, `standalone PE executable is forbidden: ${label}`);
    const optionalSize = bytes.readUInt16LE(kind.offset + 20);
    const optionalMagic = bytes.readUInt16LE(kind.offset + 24);
    assert.ok([0x10b, 0x20b].includes(optionalMagic), `invalid PE optional header: ${label}`);
    assert.ok(optionalSize >= (optionalMagic === 0x20b ? 112 : 96), `truncated PE optional header: ${label}`);
    range(bytes, kind.offset + 24, optionalSize, label);
    return;
  }
  if (kind.format === "Mach-O") {
    const header = kind.wide ? 32 : 28;
    range(bytes, 0, header, label);
    assert.ok([6, 8].includes(u32(12, kind.little)), `standalone Mach-O executable is forbidden: ${label}`);
    const count = u32(16, kind.little), size = u32(20, kind.little);
    range(bytes, header, size, label);
    assert.ok(count <= Math.floor(size / 8), `invalid Mach-O load commands: ${label}`);
    let offset = header;
    for (let index = 0; index < count; index++) {
      range(bytes, offset, 8, label);
      const command = u32(offset, kind.little), commandSize = u32(offset + 4, kind.little);
      assert.ok(commandSize >= 8 && offset + commandSize <= header + size, `invalid Mach-O load command: ${label}`);
      assert.ok(command !== 5 && command !== 0x80000028, `Mach-O process entrypoint is forbidden: ${label}`);
      offset += commandSize;
    }
    assert.equal(offset, header + size, `invalid Mach-O command size: ${label}`);
    return;
  }
  range(bytes, 0, 8, label);
  const count = u32(4, kind.little), size = kind.wide ? 32 : 20;
  assert.ok(count > 0 && count <= 64, `invalid universal Mach-O architecture count: ${label}`);
  range(bytes, 8, count * size, label);
  const slices = [];
  for (let index = 0; index < count; index++) {
    const row = 8 + index * size;
    const offset = kind.wide ? u64(row + 8, kind.little) : BigInt(u32(row + 8, kind.little));
    const length = kind.wide ? u64(row + 16, kind.little) : BigInt(u32(row + 12, kind.little));
    assert.ok(offset >= BigInt(8 + count * size) && length > 0n && offset + length <= BigInt(bytes.length), `invalid universal Mach-O slice: ${label}`);
    assert.ok(slices.every((slice) => offset >= slice.end || offset + length <= slice.start), `overlapping universal Mach-O slices: ${label}`);
    slices.push({ start: offset, end: offset + length });
    assertModuleImage(bytes.subarray(Number(offset), Number(offset + length)), `${label} slice ${index}`, true);
  }
}

function assertFileContents(relativePath, bytes) {
  if (relativePath === "native/ait_napi.node") {
    assertAddonImage(bytes, relativePath);
  } else {
    assert.equal(imageKind(bytes), undefined, `native image is forbidden in packaged text: ${relativePath}`);
    assert.ok(isUtf8(bytes) && !bytes.includes(0), `packaged text must be NUL-free UTF-8: ${relativePath}`);
  }
}

// Image-kind checks complement the mandatory target-specific dlopen/API smoke.
export function assertAddonImage(bytes, label = "native/ait_napi.node") {
  assertModuleImage(bytes, label);
}

// Check bytes delivered by npm's archive extraction, including hidden entries.
// The envelope permits its JS launcher; the implementation permits its loadable
// addon. Neither allowlist permits a standalone native executable or relay.
export async function assertPackageFiles(packageRoot, expectedFiles) {
  const root = await lstat(packageRoot);
  assert.ok(root.isDirectory() && !root.isSymbolicLink(), "package root must be a regular directory");
  const files = new Set(expectedFiles);
  const directories = new Set();
  for (const file of files) {
    assert.ok(!path.posix.isAbsolute(file) && !file.split("/").includes(".."));
    const components = file.split("/");
    for (let index = 1; index < components.length; index++) {
      directories.add(components.slice(0, index).join("/"));
    }
  }
  const found = [];
  async function inspect(relativeRoot) {
    const absoluteRoot = path.join(packageRoot, ...relativeRoot.split("/").filter(Boolean));
    for (const entry of await readdir(absoluteRoot, { withFileTypes: true })) {
      const relativePath = relativeRoot ? `${relativeRoot}/${entry.name}` : entry.name;
      if (entry.isDirectory()) {
        assert.ok(directories.has(relativePath), `unexpected packaged directory: ${relativePath}`);
        await inspect(relativePath);
      } else {
        assert.ok(entry.isFile() && !entry.isSymbolicLink(), `packaged path must be a regular file: ${relativePath}`);
        assert.ok(files.has(relativePath), `unexpected packaged file: ${relativePath}`);
        assertFileContents(relativePath, await readFile(path.join(packageRoot, ...relativePath.split("/"))));
        found.push(relativePath);
      }
    }
  }
  await inspect("");
  assert.deepEqual(found.sort(), [...files].sort(), "installed archive inventory differs from the package contract");
}
