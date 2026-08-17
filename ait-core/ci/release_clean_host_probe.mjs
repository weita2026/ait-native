#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

function fail(message, code = 65) {
  const error = new Error(message);
  error.exitCode = code;
  throw error;
}

function options(argv) {
  const result = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index]?.replace(/^--/, "");
    const value = argv[index + 1];
    if (!key || value === undefined || !["platforms", "target", "output"].includes(key)) {
      fail("usage: release_clean_host_probe.mjs --platforms <json> --target <triple> --output <absolute-json>", 64);
    }
    result[key] = value;
  }
  if (!result.platforms || !result.target || !result.output) {
    fail("clean-host probe arguments are incomplete", 64);
  }
  return result;
}

function readJson(file) {
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink()) {
    fail("clean-host probe platform authority must be a regular file", 66);
  }
  return JSON.parse(readFileSync(file, "utf8"));
}

function locate(command) {
  const locator = process.platform === "win32" ? "where.exe" : "which";
  const result = spawnSync(locator, [command], { encoding: "utf8", windowsHide: true });
  return result.status === 0 ? result.stdout.trim().split(/\r?\n/)[0] : null;
}

function execute(command, args) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    timeout: 120_000,
    windowsHide: true,
  });
  return {
    command: [command, ...args].join(" "),
    exit_status: result.status ?? 127,
    stdout_tail: String(result.stdout ?? "").slice(-2048),
    stderr_tail: String(result.stderr ?? result.error?.message ?? "").slice(-2048),
  };
}

function main() {
  const input = options(process.argv.slice(2));
  if (!path.isAbsolute(input.output) || existsSync(input.output)) {
    fail("clean-host probe output must be a new absolute path", 73);
  }
  const platforms = readJson(input.platforms);
  if (
    platforms.contract !== "ait-native-bootstrap-matrix/v1" ||
    platforms.matrix_revision !== "six-target-2026-07-19.1" ||
    !Array.isArray(platforms.targets) ||
    platforms.targets.length !== 6
  ) {
    fail("clean-host probe platform authority is invalid");
  }
  const row = platforms.targets.find((candidate) => candidate.target === input.target);
  if (!row) {
    fail("clean-host probe target is undeclared");
  }
  const expectedPlatform = { macos: "darwin", linux: "linux", windows: "win32" }[row.os];
  const expectedArchitecture = row.architecture === "x86_64" ? "x64" : "arm64";
  const runnerLabel = process.env.AIT_CLEAN_HOST_RUNNER_LABEL ?? "";
  const failures = [];
  if (
    process.env.GITHUB_ACTIONS !== "true" ||
    runnerLabel !== row.runner ||
    process.platform !== expectedPlatform ||
    process.arch !== expectedArchitecture
  ) {
    failures.push("runner_target_mismatch");
  }
  const required = {
    macos: [["node"], ["python3", "python"], ["npm"], ["brew"], ["pgrep"]],
    linux: [
      ["node"],
      ["python3", "python"],
      ["npm"],
      ["brew"],
      ["sudo"],
      ["apt-get"],
      ["apt-cache"],
      ["systemctl"],
      ["docker"],
      ["pgrep"],
    ],
    windows: [["node"], ["python", "python3"], ["npm.cmd", "npm"], ["winget.exe"], ["powershell.exe"]],
  }[row.os];
  const commands = {};
  for (const alternatives of required) {
    const selected = alternatives.map((command) => [command, locate(command)]).find((entry) => entry[1]);
    const identity = alternatives.join("|");
    commands[identity] = selected ? { command: selected[0], path: selected[1] } : null;
    if (!selected) {
      failures.push(`missing_command:${identity}`);
    }
  }
  const probes = [];
  if (row.os === "linux") {
    probes.push(execute("sudo", ["-n", "true"]));
    probes.push(execute("systemctl", ["--version"]));
    probes.push(execute("docker", ["info", "--format", "{{.ServerVersion}}"]));
    probes.push(execute("brew", ["services", "list"]));
  } else if (row.os === "macos") {
    probes.push(execute("brew", ["services", "list"]));
  } else {
    probes.push(execute("winget.exe", ["--version"]));
  }
  for (const probe of probes) {
    if (probe.exit_status !== 0) {
      failures.push(`capability_probe:${probe.command}`);
    }
  }
  const evidence = {
    contract: "ait.release.clean-host.runner-probe/v1",
    status: failures.length === 0 ? "pass" : "fail",
    target: row,
    runner: {
      label: runnerLabel,
      name: process.env.RUNNER_NAME ?? null,
      os: process.env.RUNNER_OS ?? null,
      architecture: process.env.RUNNER_ARCH ?? null,
      image_os: process.env.ImageOS ?? null,
      image_version: process.env.ImageVersion ?? null,
      node_platform: process.platform,
      node_architecture: process.arch,
      run_id: process.env.GITHUB_RUN_ID ?? null,
      run_attempt: process.env.GITHUB_RUN_ATTEMPT ?? null,
      job: process.env.GITHUB_JOB ?? null,
    },
    capabilities: { commands, probes },
    failures,
  };
  writeFileSync(input.output, `${JSON.stringify(evidence, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o644,
    flag: "wx",
  });
  process.stdout.write(`${input.output}\n`);
  if (failures.length > 0) {
    process.stderr.write(`clean-host runner probe failed: ${failures.join(", ")}\n`);
    process.exitCode = 1;
  }
}

try {
  main();
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitCode ?? 70);
}
