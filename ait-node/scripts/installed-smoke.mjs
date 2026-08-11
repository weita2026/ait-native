import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { lstatSync } from "node:fs";
import path from "node:path";
import {
  fixtureFailureArgs,
  fixtureForwardArgs,
} from "./fixture-payloads.mjs";

const installRoot = path.resolve(process.argv[2] ?? "");
if (process.argv[2] === undefined) {
  throw new Error("installed smoke requires an npm --prefix root");
}

function run(command, args) {
  const packageLauncher = path.join(
    installRoot,
    "node_modules",
    "ait-native",
    "bin",
    `${command}.mjs`,
  );
  const npmLauncher = path.join(
    installRoot,
    "node_modules",
    ".bin",
    process.platform === "win32" ? `${command}.cmd` : command,
  );
  const shim = lstatSync(npmLauncher);
  assert.equal(shim.isFile() || shim.isSymbolicLink(), true);
  if (process.platform !== "win32") {
    return spawnSync(npmLauncher, args, { encoding: "utf8" });
  }
  return spawnSync(process.execPath, [packageLauncher, ...args], {
    encoding: "utf8",
  });
}

const commands = [];
for (const command of ["ait", "ait-server"]) {
  const forwarded = run(command, fixtureForwardArgs(
    `${command}-alpha`,
    `${command}-beta`,
  ));
  assert.equal(forwarded.status, 0, forwarded.stderr);
  assert.deepEqual(JSON.parse(forwarded.stdout), [
    `${command}-alpha`,
    `${command}-beta`,
  ]);

  const failed = run(command, fixtureFailureArgs(23));
  assert.equal(failed.status, 23, failed.stderr);
  commands.push(command);
}

process.stdout.write(
  `${JSON.stringify({ commands, argv_forwarded: true, failure_status: 23 })}\n`,
);
