#!/usr/bin/env node

import { cpSync, existsSync, lstatSync, mkdirSync, readdirSync, chmodSync } from "node:fs";
import path from "node:path";

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

const argv = process.argv.slice(2);
if (argv.shift() !== "--source" || argv.length !== 3 || argv[1] !== "--output") {
  fail("usage: release_source_normalize.mjs --source <absolute-dir> --output <absolute-dir>", 64);
}
const source = path.resolve(argv[0]);
const output = path.resolve(argv[2]);
if (!path.isAbsolute(argv[0]) || !path.isAbsolute(argv[2])) fail("source normalization paths must be absolute", 64);
if (!existsSync(source) || !lstatSync(source).isDirectory() || lstatSync(source).isSymbolicLink()) fail("source normalization input must be a real directory", 66);
if (existsSync(output)) fail("source normalization output already exists", 73);
mkdirSync(output, { mode: 0o755 });
cpSync(source, output, { recursive: true, preserveTimestamps: false, dereference: false });

function normalize(directory) {
  chmodSync(directory, 0o755);
  for (const name of readdirSync(directory).sort()) {
    const entry = path.join(directory, name);
    const stat = lstatSync(entry);
    if (stat.isSymbolicLink()) fail(`source normalization rejects symbolic links: ${entry}`);
    if (stat.isDirectory()) normalize(entry);
    else if (stat.isFile()) chmodSync(entry, stat.mode & 0o111 ? 0o755 : 0o644);
    else fail(`source normalization rejects unsupported entries: ${entry}`);
  }
}
normalize(output);
process.stdout.write(`${output}\n`);
