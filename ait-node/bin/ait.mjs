#!/usr/bin/env node

import { NativeRuntime } from "../src/index.js";
import { runTerminal, reportTerminalError } from "../src/terminal.js";

const runtime = new NativeRuntime();
try {
  process.exitCode = runTerminal(process.argv.slice(2), {
    runtime,
  });
} catch (error) {
  process.exitCode = reportTerminalError(error, { runtime });
}
