#!/usr/bin/env node

import { NativeRuntime } from "../src/index.js";

try {
  process.exitCode = new NativeRuntime().runCli(process.argv.slice(2));
} catch (error) {
  process.stderr.write(`ait: ${error?.message ?? String(error)}\n`);
  process.exitCode = 1;
}
