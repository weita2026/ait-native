import assert from "node:assert/strict";
import { copyFile, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ADDON_FILES, assertPackageFiles, ENVELOPE_FILES } from "../scripts/package-inventory.mjs";

// Structural fixtures exercise image classification; actual dlopen/API smoke
// remains mandatory for every released target.
function elf({ type = 3, entry = 0n, wide = true, little = true, interpreter = false } = {}) {
  const header = wide ? 64 : 52, programHeader = wide ? 56 : 32;
  const bytes = Buffer.alloc(header + (interpreter ? programHeader : 0));
  bytes.write("\x7fELF", 0, "latin1");
  bytes[4] = wide ? 2 : 1; bytes[5] = little ? 1 : 2; bytes[6] = 1;
  const u16 = (value, offset) => bytes[little ? "writeUInt16LE" : "writeUInt16BE"](value, offset);
  const u32 = (value, offset) => bytes[little ? "writeUInt32LE" : "writeUInt32BE"](value, offset);
  const u64 = (value, offset) => bytes[little ? "writeBigUInt64LE" : "writeBigUInt64BE"](value, offset);
  u16(type, 16); u32(1, 20); u16(header, wide ? 52 : 40);
  if (wide) u64(entry, 24); else u32(Number(entry), 24);
  if (interpreter) {
    if (wide) u64(BigInt(header), 32); else u32(header, 28);
    u16(programHeader, wide ? 54 : 42); u16(1, wide ? 56 : 44); u32(3, header);
  }
  return bytes;
}

function mach({ type = 6, wide = true, little = true, entryCommand = false } = {}) {
  const header = wide ? 32 : 28;
  const bytes = Buffer.alloc(header + (entryCommand ? 8 : 0));
  const u32 = (value, offset) => bytes[little ? "writeUInt32LE" : "writeUInt32BE"](value, offset);
  u32(wide ? 0xfeedfacf : 0xfeedface, 0); u32(type, 12);
  if (entryCommand) { u32(1, 16); u32(8, 20); u32(0x80000028, header); u32(8, header + 4); }
  return bytes;
}

function universal(images, { wide = false, little = false } = {}) {
  const rowSize = wide ? 32 : 20, tableEnd = 8 + rowSize * images.length;
  const bytes = Buffer.alloc(tableEnd + images.reduce((total, image) => total + image.length, 0));
  const u32 = (value, offset) => bytes[little ? "writeUInt32LE" : "writeUInt32BE"](value, offset);
  const u64 = (value, offset) => bytes[little ? "writeBigUInt64LE" : "writeBigUInt64BE"](BigInt(value), offset);
  u32(wide ? 0xcafebabf : 0xcafebabe, 0); u32(images.length, 4);
  let offset = tableEnd;
  images.forEach((image, index) => {
    const row = 8 + index * rowSize;
    if (wide) { u64(offset, row + 8); u64(image.length, row + 16); }
    else { u32(offset, row + 8); u32(image.length, row + 12); }
    image.copy(bytes, offset); offset += image.length;
  });
  return bytes;
}

function pe({ dll = true, wide = true } = {}) {
  const optionalSize = wide ? 112 : 96;
  const bytes = Buffer.alloc(64 + 24 + optionalSize);
  bytes.write("MZ", 0); bytes.writeUInt32LE(64, 60); bytes.writeUInt32LE(0x4550, 64);
  bytes.writeUInt16LE(optionalSize, 84); bytes.writeUInt16LE(dll ? 0x2002 : 2, 86);
  bytes.writeUInt16LE(wide ? 0x20b : 0x10b, 88);
  return bytes;
}

async function fixture(context, files) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-inventory-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  for (const relativePath of files) {
    const absolutePath = path.join(root, ...relativePath.split("/"));
    await mkdir(path.dirname(absolutePath), { recursive: true });
    await writeFile(absolutePath, relativePath === "native/ait_napi.node" ? elf() : "inventory test fixture");
  }
  return root;
}

test("package inventories admit the JS launcher and addon in their own packages", async (context) => {
  const envelope = await fixture(context, ENVELOPE_FILES);
  const addon = await fixture(context, ADDON_FILES);
  await assertPackageFiles(envelope, ENVELOPE_FILES);
  await assertPackageFiles(addon, ADDON_FILES);
  await assert.rejects(assertPackageFiles(envelope, ADDON_FILES));
  await assert.rejects(assertPackageFiles(addon, ENVELOPE_FILES));
});

test("installed archives reject standalone executables and hidden extras", async (context) => {
  const addon = await fixture(context, ADDON_FILES);
  for (const name of ["ait", "ait-server", "ait-runner", "ait.exe", ".hidden-cli"]) {
    const unexpected = path.join(addon, "native", name);
    await writeFile(unexpected, "forbidden executable fixture");
    await assert.rejects(assertPackageFiles(addon, ADDON_FILES), /unexpected packaged file/);
    await rm(unexpected);
  }
  const envelope = await fixture(context, ENVELOPE_FILES);
  await mkdir(path.join(envelope, "native"));
  await assert.rejects(assertPackageFiles(envelope, ENVELOPE_FILES), /unexpected packaged directory/);
});

test("an installed archive cannot omit its declared JS launcher or addon", async (context) => {
  for (const [files, missing] of [[ENVELOPE_FILES, "bin/ait.mjs"], [ADDON_FILES, "native/ait_napi.node"]]) {
    const root = await fixture(context, files);
    await rm(path.join(root, ...missing.split("/")));
    await assert.rejects(assertPackageFiles(root, files), /installed archive inventory differs/);
  }
});

test("package inventory rejects a symlink replacing an expected regular file", { skip: process.platform === "win32" }, async (context) => {
  const root = await fixture(context, ENVELOPE_FILES);
  const launcher = path.join(root, "bin", "ait.mjs");
  await rm(launcher);
  await symlink(path.join(root, "src", "terminal.js"), launcher);
  await assert.rejects(assertPackageFiles(root, ENVELOPE_FILES), /must be a regular file/);
});

test("installed archive inspection rejects a real executable renamed as its JS launcher", async (context) => {
  const root = await fixture(context, ENVELOPE_FILES);
  await copyFile(process.platform === "darwin" ? "/usr/bin/true" : process.execPath, path.join(root, "bin", "ait.mjs"));
  await assert.rejects(assertPackageFiles(root, ENVELOPE_FILES), /native image is forbidden in packaged text/);
  const addon = await fixture(context, ADDON_FILES);
  await copyFile(process.platform === "darwin" ? "/usr/bin/true" : process.execPath, path.join(addon, "native", "ait_napi.node"));
  await assert.rejects(assertPackageFiles(addon, ADDON_FILES), /forbidden/);
});

test("native images cannot occupy declarations, documentation or JavaScript slots", async (context) => {
  const root = await fixture(context, ENVELOPE_FILES);
  for (const name of ["src/index.d.ts", "README.md", "src/agent.js"]) {
    for (const bytes of [elf({ type: 2 }), mach({ type: 2 }), pe({ dll: false }), elf()]) {
      await writeFile(path.join(root, name), bytes);
      await assert.rejects(assertPackageFiles(root, ENVELOPE_FILES), /native image is forbidden in packaged text/);
    }
    await writeFile(path.join(root, name), "restored text fixture");
  }
  await writeFile(path.join(root, "NOTICE"), Buffer.from([0xff, 0xfe, 1, 2]));
  await assert.rejects(assertPackageFiles(root, ENVELOPE_FILES), /NUL-free UTF-8/);
});

test("the addon slot admits ELF shared, PE DLL and Mach-O shared or bundle header kinds", async (context) => {
  const root = await fixture(context, ADDON_FILES);
  for (const bytes of [elf(), elf({ wide: false, little: false }), pe(), pe({ wide: false }),
    mach(), mach({ type: 8, little: false }), mach({ wide: false }),
    universal([mach(), mach({ type: 8 })]), universal([mach()], { wide: true, little: true })]) {
    await writeFile(path.join(root, "native", "ait_napi.node"), bytes);
    await assertPackageFiles(root, ADDON_FILES);
  }
});

test("the addon suffix cannot exempt executable kinds or process entrypoints", async (context) => {
  const root = await fixture(context, ADDON_FILES);
  for (const bytes of [elf({ type: 2 }), elf({ entry: 1n }), elf({ interpreter: true }),
    pe({ dll: false }), mach({ type: 2 }), mach({ entryCommand: true }),
    universal([mach(), mach({ type: 2 })]), Buffer.from("module.exports = {}")]) {
    await writeFile(path.join(root, "native", "ait_napi.node"), bytes);
    await assert.rejects(assertPackageFiles(root, ADDON_FILES), /forbidden|must declare a loadable native image/);
  }
});

test("malformed native headers and universal slices fail before qualification", async (context) => {
  const root = await fixture(context, ADDON_FILES);
  const overlapping = universal([mach(), mach()]);
  overlapping.writeUInt32BE(overlapping.readUInt32BE(16), 36);
  const beyondEnd = universal([mach()]);
  beyondEnd.writeUInt32BE(0xffffffff, 16);
  const truncatedCommands = mach();
  truncatedCommands.writeUInt32LE(1, 16);
  truncatedCommands.writeUInt32LE(8, 20);
  for (const bytes of [elf().subarray(0, 8), pe().subarray(0, 90), truncatedCommands,
    overlapping, beyondEnd, universal([Buffer.from("not a Mach-O slice")])]) {
    await writeFile(path.join(root, "native", "ait_napi.node"), bytes);
    await assert.rejects(assertPackageFiles(root, ADDON_FILES), /truncated|invalid|overlapping|must declare/);
  }
});
