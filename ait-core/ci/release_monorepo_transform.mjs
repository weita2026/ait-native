#!/usr/bin/env node

import { lstat, readFile, rename, unlink, writeFile } from "node:fs/promises";

function fail(message) {
  throw new Error(message);
}

async function main() {
  const [filePath, from, to] = process.argv.slice(2);
  if (filePath === undefined || from === undefined || to === undefined || process.argv.length !== 5) {
    fail("usage: release_monorepo_transform.mjs <file> <from> <to>");
  }
  if (from.length === 0 || to.length === 0 || from === to || to.includes(from)) {
    fail("monorepo transform requires two distinct bounded literal values");
  }
  const entry = await lstat(filePath);
  if (!entry.isFile() || entry.isSymbolicLink()) {
    fail(`monorepo transform target must be a regular file: ${filePath}`);
  }
  const source = await readFile(filePath, "utf8");
  const occurrences = source.split(from).length - 1;
  if (occurrences !== 1 || source.includes(to)) {
    fail(`monorepo transform target must contain its from value exactly once and not contain its to value: ${filePath}`);
  }
  const transformed = source.replace(from, to);
  const temporary = `${filePath}.ait-monorepo-transform`;
  try {
    await writeFile(temporary, transformed, { mode: entry.mode & 0o777 });
    await rename(temporary, filePath);
  } catch (error) {
    await unlink(temporary).catch(() => {});
    throw error;
  }
}

main().catch((error) => {
  process.stderr.write(`monorepo transform failed: ${error.message}\n`);
  process.exitCode = 1;
});
